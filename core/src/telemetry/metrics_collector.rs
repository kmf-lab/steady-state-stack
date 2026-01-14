//! The `metrics_collector` module provides the `MetricsCollector` actor, which is responsible for
//! gathering telemetry data from all actors and channels in the graph. It aggregates this data
//! into a `DotState` for visualization and Prometheus metrics.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use crate::*;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};

/// The name of the metrics collector actor.
pub const NAME: &str = "metrics_collector";

/// Represents a telemetry receiver and its associated metadata.
pub struct CollectorDetail {
    /// The identity of the actor being monitored.
    pub ident: ActorIdentity,
    /// A queue of telemetry receivers for this actor.
    pub telemetry_take: VecDeque<Box<dyn RxTel>>,
}

/// Data packet sent to the metrics server for visualization.
#[derive(Clone, Debug)]
pub enum DiagramData {
    /// Definition of a node and its connected channels.
    NodeDef(u64, Box<(Arc<ActorMetaData>, Box<[Arc<ChannelMetaData>]>, Box<[Arc<ChannelMetaData>]>)>),
    /// Performance status updates for a set of nodes.
    NodeProcessData(u64, Box<[ActorStatus]>),
    /// Throughput and volume data for all channels.
    ChannelVolumeData(u64, Box<[(i64, i64)]>),
}

/// Entry point to run the MetricsCollector actor.
pub async fn run(
    context: SteadyContext, 
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>, 
    targets: Arc<[SteadyTx<DiagramData>; 1]>
) -> Result<(), Box<dyn std::error::Error>> {
    let frame_rate_ms = context.frame_rate_ms;
    let collector = MetricsCollector::new(all_telemetry_rx, targets, frame_rate_ms);
    collector.run(context).await
}

/// The `MetricsCollector` actor gathers telemetry data from all actors and channels.
pub struct MetricsCollector {
    /// Shared telemetry receivers for all actors in the graph.
    all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    /// Channels to send diagram data to the metrics server.
    targets: Arc<[SteadyTx<DiagramData>; 1]>,
    /// The frame rate in milliseconds for telemetry collection.
    frame_rate_ms: u64,
    /// Sequence number for data packets.
    seq: u64,
    /// Cache of known actor IDs to avoid redundant NodeDef sends.
    known_actors: HashMap<usize, u32>,

    /// # CRITICAL DESIGN REQUIREMENT: Persistent Accumulation Buffers
    /// These vectors MUST persist for the entire life of the MetricsCollector actor.
    /// 
    /// The Steady State architecture relies on monotonically increasing absolute counters 
    /// for 'send' and 'take' operations. The downstream stats evaluation (channel_stats.rs) 
    /// performs unsigned math (send - take) to determine inflight volume.
    ///
    /// If these buffers are moved to local loop variables, the collector will only send 
    /// frame-deltas. Due to timing jitter between telemetry production and collection, 
    /// a frame-delta can occasionally show 'take' > 'send', resulting in a negative 
    /// value that, when cast to unsigned, causes a catastrophic overflow panic.
    ///
    /// DO NOT MOVE THESE TO THE LOOP. THEY ARE NON-NEGOTIABLE FOR SYSTEM STABILITY.
    take_send_source: Vec<(i64, i64)>,
    future_take: Vec<i64>,
    future_send: Vec<i64>,
}

impl MetricsCollector {
    /// Creates a new `MetricsCollector` instance.
    pub(crate) fn new(
        all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
        targets: Arc<[SteadyTx<DiagramData>; 1]>,
        frame_rate_ms: u64,
    ) -> Self {
        MetricsCollector {
            all_telemetry_rx,
            targets,
            frame_rate_ms,
            seq: 0,
            known_actors: HashMap::new(),
            take_send_source: Vec::new(),
            future_take: Vec::new(),
            future_send: Vec::new(),
        }
    }

    /// The main loop for the `MetricsCollector` actor.
    pub async fn run(mut self, mut context: SteadyContext) -> Result<(), Box<dyn std::error::Error>> {
        // CRITICAL: Do NOT use into_spotlight() here. 
        // MetricsCollector must use the raw SteadyActorShadow (context) to avoid telemetry 
        // feedback loops and prevent this internal actor from appearing in user-facing charts.
        let mut tx_guard = self.targets[0].lock().await;

        while context.is_running(|| true) {
            self.seq += 1;
            
            let mut actor_statuses = Vec::new();

            let receivers = self.all_telemetry_rx.read();
            for detail in receivers.iter() {
                for rx in detail.telemetry_take.iter() {
                    let meta = rx.actor_metadata();
                    let actor_id = meta.ident.id;

                    // 1. Send NodeDef if this is a new actor
                    if !self.known_actors.contains_key(&actor_id) {
                        let def = DiagramData::NodeDef(
                            self.seq, 
                            Box::new((
                                meta.clone(), 
                                rx.rx_channel_id_vec().into_boxed_slice(), 
                                rx.tx_channel_id_vec().into_boxed_slice()
                            ))
                        );
                        let _ = context.send_async(&mut *tx_guard, def, SendSaturation::AwaitForRoom).await;
                        self.known_actors.insert(actor_id, 0);
                    }

                    // 2. Collect Actor Status
                    if let Some(status) = rx.consume_actor() {
                        if actor_id >= actor_statuses.len() {
                            actor_statuses.resize(actor_id + 1, ActorStatus::default());
                        }
                        actor_statuses[actor_id] = status;
                    }

                    // 3. Collect Channel Volume into persistent buffers
                    let rx_metas = rx.rx_channel_id_vec();
                    let tx_metas = rx.tx_channel_id_vec();
                    let max_id = rx_metas.iter().chain(tx_metas.iter())
                        .map(|m| m.id).max().unwrap_or(0);
                    
                    if max_id >= self.take_send_source.len() {
                        self.take_send_source.resize(max_id + 1, (0i64, 0i64));
                        self.future_take.resize(max_id + 1, 0i64);
                        self.future_send.resize(max_id + 1, 0i64);
                    }

                    rx.consume_take_into(&mut self.take_send_source, &mut self.future_take, &mut self.future_send);
                    rx.consume_send_into(&mut self.take_send_source, &mut self.future_send);
                }
            }
            drop(receivers);

            // 4. Relay batches to server
            if !actor_statuses.is_empty() {
                let _ = context.send_async(&mut *tx_guard, DiagramData::NodeProcessData(self.seq, actor_statuses.into_boxed_slice()), SendSaturation::AwaitForRoom).await;
            }
            if !self.take_send_source.is_empty() {
                // We clone the source to send the current absolute totals to the server
                let _ = context.send_async(&mut *tx_guard, DiagramData::ChannelVolumeData(self.seq, self.take_send_source.clone().into_boxed_slice()), SendSaturation::AwaitForRoom).await;
            }

            context.wait_periodic(Duration::from_millis(self.frame_rate_ms)).await;
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod extra_tests {
    use super::*;

    #[test]
    fn test_collect_channel_data_empty() {
        let all_telemetry_rx = Arc::new(RwLock::new(Vec::new()));
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().build_channel::<DiagramData>();
        let targets = Arc::new([tx.clone()]);
        let _collector = MetricsCollector::new(all_telemetry_rx, targets, 40);
    }
}

#[cfg(test)]
pub(crate) mod metric_collector_tests {
    use super::*;

    #[test]
    fn test_raw_diagram_state_default() {
        let all_telemetry_rx = Arc::new(RwLock::new(Vec::new()));
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().build_channel::<DiagramData>();
        let targets = Arc::new([tx.clone()]);
        let collector = MetricsCollector::new(all_telemetry_rx, targets, 40);
        assert_eq!(collector.seq, 0);
        assert!(collector.known_actors.is_empty());
    }
}
