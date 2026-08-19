 //! The `metrics_collector` module provides the `MetricsCollector` actor, which is responsible for
//! gathering telemetry data from all actors and channels in the graph. It aggregates this data
//! into a `DotState` for visualization and Prometheus metrics.
//!
//! Set **`STEADY_TELEMETRY_EDGE_DIAG=1`** at process start to log one structured line per actor’s first
//! `NodeDef`: numeric actor id, name, rx/tx channel telemetry ids plus `Arc` pointers—see
//! [telemetry-edge-conflict.md](../../../docs/telemetry-edge-conflict.md).

// ss[impl telemetry.prometheus-metrics]
use std::collections::{VecDeque};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[impl telemetry.prometheus-metrics]
use parking_lot::RwLock;
// ss[related philosophy.structural-hierarchy]
use crate::*;
// ss[related philosophy.structural-hierarchy]
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
// ss[impl telemetry.prometheus-metrics]
use crate::telemetry::{metrics_collector, metrics_server};

/// The name of the metrics collector actor.
// ss[impl telemetry.prometheus-metrics]
pub const NAME: &str = "metrics_collector";

/// Represents a telemetry receiver and its associated metadata.
// ss[impl telemetry.prometheus-metrics]
pub struct CollectorDetail {
    /// The identity of the actor being monitored.
    pub ident: ActorIdentity,
    /// A queue of telemetry receivers for this actor.
    pub telemetry_take: VecDeque<Box<dyn RxTel>>,
}

/// Data packet sent to the metrics server for visualization.
#[derive(Clone, Debug)]
// ss[impl telemetry.prometheus-metrics]
pub enum DiagramData {
    /// Definition of a node and its connected channels.
    NodeDef(u64, Box<(Arc<ActorMetaData>, Box<[Arc<ChannelMetaData>]>, Box<[Arc<ChannelMetaData>]>)>),
    /// Performance status updates for a set of nodes.
    NodeProcessData(u64, Box<[ActorStatus]>),
    /// Optional DOT subtitle line per actor id (`None` value clears the subtitle).
    NodeDotSubtitle(u64, Box<[(usize, Option<String>)]>),
    /// Throughput and volume data for a subset of channels.
    ChannelVolumeData(u64, Box<[(usize, i64, i64)]>),
}

/// Entry point to run the MetricsCollector actor.
// ss[impl telemetry.prometheus-metrics]
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
// ss[impl telemetry.prometheus-metrics]
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
    cursor: usize,
}

#[inline]
// ss[impl telemetry.prometheus-metrics]
fn telemetry_edge_diag_enabled() -> bool {
    matches!(
        std::env::var("STEADY_TELEMETRY_EDGE_DIAG").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

// ss[impl telemetry.prometheus-metrics]
fn format_diag_channel_slots(metas: &[Arc<ChannelMetaData>]) -> String {
    let mut pairs: Vec<(usize, usize)> =
        metas.iter().map(|m| (m.id, Arc::as_ptr(m) as usize)).collect();
    pairs.sort_by_key(|p| p.0);
    pairs
        .into_iter()
        .map(|(id, ptr)| format!("{}:{:#x}", id, ptr))
        .collect::<Vec<_>>()
        .join(";")
}

// ss[impl telemetry.prometheus-metrics]
impl MetricsCollector {
    /// Creates a new `MetricsCollector` instance.
    // ss[related philosophy.structural-hierarchy]
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
            cursor: 0,
        }
    }

    // ss[impl telemetry.prometheus-metrics]
    pub async fn run(self, context: SteadyContext) -> Result<(), Box<dyn std::error::Error>> {
        // CRITICAL: MetricsCollector must use the raw SteadyActorShadow (context) to avoid telemetry
        // feedback loops and prevent this internal actor from appearing in user-facing charts.
        // Also we move this to heap in case we have a giant graph
        Box::pin(self.internal_behavior(context)).await
    }
    /// The main loop for the `MetricsCollector` actor.
    // ss[impl telemetry.prometheus-metrics]
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
            
            // Sparse actor status accumulation keyed by actor_id.
            // We compact before sending so downstream only processes real updates.
            let mut actor_statuses: Vec<Option<ActorStatus>> = Vec::new();
            let mut node_defs_to_send = Vec::new();
            let mut channel_volumes_to_send = Vec::new();
            let mut dot_subtitle_updates: Vec<(usize, Option<String>)> = Vec::new();

            // 1. GATHER PHASE: Acquire the read lock, collect data into local buffers, and release.
            // CRITICAL: We must NOT perform any .await operations (like send_async) while holding 
            // this lock, as it can lead to deadlocks with actors attempting to register themselves.
            {
                let receivers = self.all_telemetry_rx.read();
                let len = receivers.len();
                if len > 0 {
                    let chunk_size = 250; // Process a manageable slice per frame
                    for i in 0..chunk_size.min(len) {
                        let idx = (self.cursor + i) % len;
                        let detail = &receivers[idx];

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
                                if telemetry_edge_diag_enabled() {
                                    let rxv = rx.rx_channel_id_vec();
                                    let txv = rx.tx_channel_id_vec();
                                    info!(
                                        target: "steady_state::telemetry::dot",
                                        concat!(
                                            "telemetry NodeDef_diag seq={} ",
                                            "actor_numeric_id={} actor_name={:?} ",
                                            "telemetry_bundle_queue_depth={} ",
                                            "rx_channel_id_arc=[{}] tx_channel_id_arc=[{}]",
                                        ),
                                        self.seq,
                                        meta.ident.id,
                                        meta.ident.label,
                                        detail.telemetry_take.len(),
                                        format_diag_channel_slots(&rxv),
                                        format_diag_channel_slots(&txv),
                                    );
                                }
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
                                    actor_statuses.resize_with(actor_id + 1, || None);
                                }
                                self.last_seen[actor_id] = now_loop;
                                actor_statuses[actor_id] = Some(status);
                                collected_this_time = true;
                            }

                            if let Some(upd) = rx.consume_dot_subtitle() {
                                dot_subtitle_updates.push((meta.ident.id, upd));
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

                            // Record sparse updates for the channels belonging to this actor
                            for m in rx_metas.iter().chain(tx_metas.iter()) {
                                let (t, s) = self.take_send_source[m.id];
                                channel_volumes_to_send.push((m.id, t, s));
                            }
                        }

                        // Detect Stalls (Default 20s timeout)
                        if !collected_this_time {
                            let last_time = self.last_seen[actor_id];
                            if now_loop.duration_since(last_time) > Duration::from_secs(20) {
                                if actor_id >= actor_statuses.len() {
                                    actor_statuses.resize_with(actor_id + 1, || None);
                                }
                                actor_statuses[actor_id] = Some(ActorStatus {
                                    ident: detail.ident,
                                    is_quiet: true,
                                    ..ActorStatus::default()
                                });
                                
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
                    self.cursor = (self.cursor + chunk_size) % len;
                }
            } // READ LOCK DROPPED HERE

            // 2. TRANSMIT PHASE: Perform async sends now that the registry lock is released.
            
            // Send buffered NodeDefs
            for def in node_defs_to_send {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, def, SendSaturation::AwaitForRoom).await;
            }

            // Compact sparse actor updates before transmit.
            // Downstream identifies nodes by status.ident.id (not positional index).
            let actor_statuses: Vec<ActorStatus> = actor_statuses.into_iter().flatten().collect();

            // Relay batches to server
            if !actor_statuses.is_empty() {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, DiagramData::NodeProcessData(self.seq, actor_statuses.into_boxed_slice()), SendSaturation::AwaitForRoom).await;
            }
            if !dot_subtitle_updates.is_empty() {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(
                    &mut *tx_guard,
                    DiagramData::NodeDotSubtitle(self.seq, dot_subtitle_updates.into_boxed_slice()),
                    SendSaturation::AwaitForRoom,
                )
                .await;
            }
            if !channel_volumes_to_send.is_empty() {
                let mut tx_guard = self.targets[0].lock().await;
                let _ = context.send_async(&mut *tx_guard, DiagramData::ChannelVolumeData(self.seq, channel_volumes_to_send.into_boxed_slice()), SendSaturation::AwaitForRoom).await;
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
// ss[related philosophy.structural-hierarchy]
pub(crate) mod extra_tests {
    // ss[impl telemetry.prometheus-metrics]
    use super::*;

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_collect_channel_data_empty() {
        let all_telemetry_rx = Arc::new(RwLock::new(Vec::new()));
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().build_channel::<DiagramData>();
        let targets = Arc::new([tx.clone()]);
        let _collector = MetricsCollector::new(all_telemetry_rx, targets, 40);
    }
}

#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
pub(crate) mod metric_collector_tests {
    // ss[impl telemetry.prometheus-metrics]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use proptest::prelude::*;

    #[cfg(feature = "prometheus_metrics")]
    // ss[impl telemetry.prometheus-metrics]
    async fn run_cooperative_shutdown_stub(
        ctx: SteadyActorShadow,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut actor = ctx.into_spotlight([], []);
        while actor.is_running(|| true) {
            actor.wait_periodic(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    #[cfg(feature = "prometheus_metrics")]
    // ss[impl telemetry.prometheus-metrics]
    fn run_collector_graph_integration(
        build: impl FnOnce(&mut Graph) + Send + 'static,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // ss[impl telemetry.prometheus-metrics]
        use std::thread::sleep;
        // ss[related philosophy.structural-hierarchy]
        use std::time::Duration;
        // ss[related philosophy.structural-hierarchy]
        use crate::SteadyRunner;

        SteadyRunner::test_build().run((), move |mut graph| {
            build(&mut graph);
            assert!(
                graph.start_with_timeout(Duration::from_secs(20)),
                "graph failed to reach Running before shutdown"
            );
            sleep(Duration::from_millis(120));
            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(5))
        })
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_raw_diagram_state_default() {
        let all_telemetry_rx = Arc::new(RwLock::new(Vec::new()));
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().build_channel::<DiagramData>();
        let targets = Arc::new([tx.clone()]);
        let collector = MetricsCollector::new(all_telemetry_rx, targets, 40);
        assert_eq!(collector.seq, 0);
        assert!(collector.sent_node_def.is_empty());
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn format_diag_channel_slots_orders_by_id() {
        let a = Arc::new(ChannelMetaData {
            id: 50,
            ..ChannelMetaData::default()
        });
        let b = Arc::new(ChannelMetaData {
            id: 10,
            ..ChannelMetaData::default()
        });
        let line = format_diag_channel_slots(&[a.clone(), b.clone()]);
        assert!(line.starts_with("10:"));
        assert!(line.contains(";50:"));
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn telemetry_edge_diag_enabled_honors_env_flag() {
        // SAFETY: test-local env mutation; restored before return.
        unsafe {
            std::env::set_var("STEADY_TELEMETRY_EDGE_DIAG", "1");
        }
        assert!(telemetry_edge_diag_enabled());
        unsafe {
            std::env::set_var("STEADY_TELEMETRY_EDGE_DIAG", "false");
        }
        assert!(!telemetry_edge_diag_enabled());
        unsafe {
            std::env::remove_var("STEADY_TELEMETRY_EDGE_DIAG");
        }
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    #[cfg(feature = "prometheus_metrics")]
    // ss[related philosophy.structural-hierarchy]
    fn collector_run_processes_node_def_from_registry() {
        // ss[related philosophy.structural-hierarchy]
        use std::sync::Arc;
        // ss[impl telemetry.prometheus-metrics]
        use std::collections::VecDeque;
        // ss[related philosophy.structural-hierarchy]
        use crate::graph_liveliness::ActorIdentity;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
        // ss[impl telemetry.prometheus-metrics]
        use crate::SoloAct;

        // ss[related philosophy.structural-hierarchy]
        struct EmptyTel {
            meta: Arc<ActorMetaData>,
        }
        // ss[impl telemetry.prometheus-metrics]
        impl RxTel for EmptyTel {
            // ss[related philosophy.structural-hierarchy]
            fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
                Vec::new()
            }
            // ss[impl telemetry.prometheus-metrics]
            fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
                Vec::new()
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_actor(&self) -> Option<ActorStatus> {
                None
            }
            // ss[impl telemetry.prometheus-metrics]
            fn actor_metadata(&self) -> Arc<ActorMetaData> {
                self.meta.clone()
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_take_into(
                &self,
                _take_send_source: &mut Vec<(i64, i64)>,
                _future_take: &mut Vec<i64>,
                _future_send: &mut Vec<i64>,
            ) -> bool {
                false
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_send_into(
                &self,
                _take_send_source: &mut Vec<(i64, i64)>,
                _future_send: &mut Vec<i64>,
            ) -> bool {
                false
            }
            // ss[impl telemetry.prometheus-metrics]
            fn actor_rx(&self, _version: u32) -> Option<Box<crate::SteadyRx<ActorStatus>>> {
                None
            }
            // ss[impl telemetry.prometheus-metrics]
            fn is_empty_and_closed(&self) -> bool {
                true
            }
            // ss[impl telemetry.prometheus-metrics]
            fn is_empty(&self) -> bool {
                true
            }
        }

        run_collector_graph_integration(|graph| {
            graph.actor_builder().with_name("phase7_worker").build(
                |ctx| run_cooperative_shutdown_stub(ctx),
                SoloAct,
            );
            crate::telemetry::setup::build_telemetry_metric_features(graph);
            let ident = ActorIdentity::new(4, "phase7_actor", None);
            let meta = Arc::new(ActorMetaData {
                ident,
                ..Default::default()
            });
            graph.all_telemetry_rx.write().push(CollectorDetail {
                ident,
                telemetry_take: VecDeque::from([Box::new(EmptyTel {
                    meta: meta.clone(),
                }) as Box<dyn RxTel>]),
            });
        })
        .expect("collector shutdown");
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    #[cfg(feature = "prometheus_metrics")]
    // ss[related philosophy.structural-hierarchy]
    fn collector_emits_node_def_when_edge_diag_enabled() {
        // ss[related philosophy.structural-hierarchy]
        use std::sync::Arc;
        // ss[impl telemetry.prometheus-metrics]
        use crate::graph_liveliness::ActorIdentity;
        // ss[related philosophy.structural-hierarchy]
        use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
        // ss[related philosophy.structural-hierarchy]
        use crate::SoloAct;

        // ss[impl telemetry.prometheus-metrics]
        struct DiagTel {
            meta: Arc<ActorMetaData>,
            rx_meta: Arc<ChannelMetaData>,
            tx_meta: Arc<ChannelMetaData>,
        }
        // ss[impl telemetry.prometheus-metrics]
        impl RxTel for DiagTel {
            // ss[related philosophy.structural-hierarchy]
            fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
                vec![self.tx_meta.clone()]
            }
            // ss[impl telemetry.prometheus-metrics]
            fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
                vec![self.rx_meta.clone()]
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_actor(&self) -> Option<ActorStatus> {
                None
            }
            // ss[impl telemetry.prometheus-metrics]
            fn actor_metadata(&self) -> Arc<ActorMetaData> {
                self.meta.clone()
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_take_into(
                &self,
                _take_send_source: &mut Vec<(i64, i64)>,
                _future_take: &mut Vec<i64>,
                _future_send: &mut Vec<i64>,
            ) -> bool {
                false
            }
            // ss[impl telemetry.prometheus-metrics]
            fn consume_send_into(
                &self,
                _take_send_source: &mut Vec<(i64, i64)>,
                _future_send: &mut Vec<i64>,
            ) -> bool {
                false
            }
            // ss[impl telemetry.prometheus-metrics]
            fn actor_rx(&self, _version: u32) -> Option<Box<crate::SteadyRx<ActorStatus>>> {
                None
            }
            // ss[impl telemetry.prometheus-metrics]
            fn is_empty_and_closed(&self) -> bool {
                true
            }
            // ss[impl telemetry.prometheus-metrics]
            fn is_empty(&self) -> bool {
                true
            }
        }

        run_collector_graph_integration(|graph| {
            // SAFETY: env is set on the SteadyRunner thread before telemetry actors start.
            unsafe {
                std::env::set_var("STEADY_TELEMETRY_EDGE_DIAG", "1");
            }

            graph.actor_builder().with_name("phase7_diag").build(
                |ctx| run_cooperative_shutdown_stub(ctx),
                SoloAct,
            );
            crate::telemetry::setup::build_telemetry_metric_features(graph);
            let ident = ActorIdentity::new(8, "phase7_diag_actor", None);
            let meta = Arc::new(ActorMetaData {
                ident,
                ..Default::default()
            });
            graph.all_telemetry_rx.write().push(CollectorDetail {
                ident,
                telemetry_take: std::collections::VecDeque::from([Box::new(DiagTel {
                    meta: meta.clone(),
                    rx_meta: Arc::new(ChannelMetaData {
                        id: 3,
                        ..Default::default()
                    }),
                    tx_meta: Arc::new(ChannelMetaData {
                        id: 4,
                        ..Default::default()
                    }),
                }) as Box<dyn RxTel>]),
            });
        })
        .expect("collector shutdown");

        // SAFETY: restore process env after the isolated runner thread finishes.
        unsafe {
            std::env::remove_var("STEADY_TELEMETRY_EDGE_DIAG");
        }
    }

    ss_proptest! {
        /// Property: diagnostic channel slot formatting is sorted by id.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_format_diag_channel_slots_sorted(ids in prop::collection::vec(0usize..100, 1..8)) {
            let metas: Vec<Arc<ChannelMetaData>> = ids
                .iter()
                .map(|id| {
                    Arc::new(ChannelMetaData {
                        id: *id,
                        ..ChannelMetaData::default()
                    })
                })
                .collect();
            let line = format_diag_channel_slots(&metas);
            let parsed: Vec<usize> = line
                .split(';')
                .filter_map(|part| part.split(':').next()?.parse().ok())
                .collect();
            let mut sorted = parsed.clone();
            sorted.sort_unstable();
            prop_assert_eq!(parsed, sorted);
            prop_assert!(!line.is_empty());
        }
    }
}
