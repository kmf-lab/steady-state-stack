//! The `metrics_collector` module provides the `MetricsCollector` actor, which is responsible for
//! gathering telemetry data from all actors and channels in the graph. It aggregates this data
//! into a `DotState` for visualization and Prometheus metrics.

use std::collections::{VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::RwLock;
use crate::*;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
use crate::telemetry::{metrics_collector, metrics_server};

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
    /// Tracks which actors have already had their NodeDef sent.
    sent_node_def: Vec<bool>,
    /// Tracks the last time a status update was received for each actor ID.
    last_seen: Vec<Instant>,
    /// Tracks which actors we have already warned about stalling.
    logged_is_quiet: Vec<bool>,

    /// # CRITICAL DESIGN REQUIREMENT: Persistent Accumulation Buffers
    /// These vectors MUST persist for the entire life of the MetricsCollector actor.
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
            sent_node_def: Vec::new(),
            last_seen: Vec::new(),
            logged_is_quiet: Vec::new(),
            take_send_source: Vec::new(),
            future_take: Vec::new(),
            future_send: Vec::new(),
        }
    }

    pub async fn run(self, context: SteadyContext) -> Result<(), Box<dyn std::error::Error>> {
        // CRITICAL: MetricsCollector must use the raw SteadyActorShadow (context) to avoid telemetry
        // feedback loops and prevent this internal actor from appearing in user-facing charts.
        // Also we move this to heap in case we have a giant graph
        Box::pin(self.internal_behavior(context)).await
    }
    /// The main loop for the `MetricsCollector` actor.
    pub async fn internal_behavior(mut self, mut context: SteadyContext) -> Result<(), Box<dyn std::error::Error>> {
        let start_time = Instant::now();
        let runtime_state = context.runtime_state.clone();

        // We stay alive as long as the majority of other actors are still working.
        // Once all actors except the telemetry system (Collector and Server) have agreed 
        // to shut down, we cast our 'yes' vote by returning true from this closure, 
        // which terminates the loop. This ensures we capture the final telemetry 
        // from all worker actors before we exit.
        while context.is_running(|| {
            runtime_state.read().is_shutdown_telemetry_complete(2) //for collector and server
        }) {
            self.seq += 1;
            let now_loop = Instant::now();
            
            let mut actor_statuses = Vec::new();
            let mut node_defs_to_send = Vec::new();

            // 1. GATHER PHASE: Acquire the read lock, collect data into local buffers, and release.
            // CRITICAL: We must NOT perform any .await operations (like send_async) while holding 
            // this lock, as it can lead to deadlocks with actors attempting to register themselves.
            {
                let receivers = self.all_telemetry_rx.read();
                for detail in receivers.iter() {

                    if detail.ident.label.name == metrics_collector::NAME ||
                        detail.ident.label.name == metrics_server::NAME {
                        continue; //skip internal system actors
                    }

                    let actor_id = detail.ident.id;

                    // Ensure tracking vectors are large enough for this actor_id
                    if actor_id >= self.sent_node_def.len() {
                        self.sent_node_def.resize(actor_id + 1, false);
                        self.last_seen.resize(actor_id + 1, start_time);
                        self.logged_is_quiet.resize(actor_id + 1, false);
                    }

                    let mut collected_this_time = false;
                    for rx in detail.telemetry_take.iter() {
                        let meta = rx.actor_metadata();

                        // Buffer NodeDef if this is a new actor
                        if !self.sent_node_def[actor_id] {
                            self.sent_node_def[actor_id] = true;
                            node_defs_to_send.push(DiagramData::NodeDef(
                                self.seq, 
                                Box::new((
                                    meta.clone(), 
                                    rx.rx_channel_id_vec().into_boxed_slice(), 
                                    rx.tx_channel_id_vec().into_boxed_slice()
                                ))
                            ));
                        }

                        // Collect Actor Status
                        if let Some(status) = rx.consume_actor() {
                            if actor_id >= actor_statuses.len() {
                                actor_statuses.resize(actor_id + 1, ActorStatus::default());
                            }
                            self.last_seen[actor_id] = now_loop;
                            actor_statuses[actor_id] = status;
                            collected_this_time = true;
                        }

                        // Collect Channel Volume into persistent buffers
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

                    // Detect Stalls (Default 20s timeout)
                    if !collected_this_time {
                        let last_time = self.last_seen[actor_id];
                        if now_loop.duration_since(last_time) > Duration::from_secs(20) {
                            if actor_id >= actor_statuses.len() {
                                actor_statuses.resize(actor_id + 1, ActorStatus::default());
                            }
                            actor_statuses[actor_id].ident = detail.ident;
                            actor_statuses[actor_id].is_quiet = true;
                            
                            if !self.logged_is_quiet[actor_id] {
                                //NOT a bug, just something to watch
                                trace!("Actor {:?} (ID {}) appears to be quiet (no update for {:?})", detail.ident.label, actor_id, now_loop.duration_since(last_time));
                                self.logged_is_quiet[actor_id] = true;
                            }
                        }
                    } else {
                        self.logged_is_quiet[actor_id] = false;
                    }
                }
            } // READ LOCK DROPPED HERE

            // 2. TRANSMIT PHASE: Perform async sends now that the registry lock is released.
            
            // Send buffered NodeDefs
            for def in node_defs_to_send {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, def, SendSaturation::AwaitForRoom).await;
            }

            // Relay batches to server
            if !actor_statuses.is_empty() {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, DiagramData::NodeProcessData(self.seq, actor_statuses.into_boxed_slice()), SendSaturation::AwaitForRoom).await;
            }
            if !self.take_send_source.is_empty() {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, DiagramData::ChannelVolumeData(self.seq, self.take_send_source.clone().into_boxed_slice()), SendSaturation::AwaitForRoom).await;
            }

            // CRITICAL: No locks held during periodic wait
            context.wait_periodic(Duration::from_millis(self.frame_rate_ms)).await;
        }

        // Explicitly mark all target channels as closed. This is necessary because 
        // transmitter clones are often held in spawn closures within the Graph registry, 
        // which prevents automatic closure when this actor is dropped. Marking them 
        // closed here ensures downstream actors (like the metrics_server) can detect 
        // the end of the stream and shut down cleanly.
        for target in self.targets.iter() {
            let mut guard = target.lock().await;
            guard.mark_closed();
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
        assert!(collector.sent_node_def.is_empty());
    }
}
