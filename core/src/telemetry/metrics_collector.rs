use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, LockResult};
use parking_lot::{RwLock, RwLockReadGuard};
#[allow(unused_imports)]
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use log::*; // Allow unused import

use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
#[allow(unused_imports)]
use crate::{into_monitor, steady_config, yield_now, SendSaturation, SteadyTxBundle};
use futures::future::*;
use futures_timer::Delay;
use futures_util::{select, StreamExt};
use futures_util::lock::MutexGuard;
use futures_util::stream::FuturesUnordered;
use num_traits::{One, Zero};
use ringbuf::consumer::Consumer;
use crate::graph_liveliness::ActorIdentity;
use crate::{i, GraphLivelinessState, SteadyRx};
use crate::steady_actor::SteadyActor;
#[allow(unused_imports)]
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::SendOutcome::{Blocked, Success};
use crate::steady_rx::*;
use crate::steady_tx::*;
use crate::telemetry::metrics_collector;

pub const NAME: &str = "metrics_collector";

// Enable debug logging for detailed runtime insights; can be toggled off in production if needed.
const ENABLE_DEBUG_LOGGING: bool = false;

/// Represents different types of data that can be sent to the diagram for visualization or logging.
#[derive(Clone, Debug)]
pub enum DiagramData {
    /// Defines a node with its actor metadata and channel details (rx and tx).
    #[allow(clippy::type_complexity)]
    NodeDef(
        u64,
        Box<( // Boxed to keep enum variants uniform in size for channel efficiency
              Arc<ActorMetaData>,
              Box<[Arc<ChannelMetaData>]>,
              Box<[Arc<ChannelMetaData>]>,
        )>,
    ),
    /// Contains channel volume data as (take, send) tuples for each channel.
    ChannelVolumeData(u64, Box<[(i64, i64)]>),
    /// Provides actor status updates for process monitoring.
    NodeProcessData(u64, Box<[ActorStatus]>),
}

/// Tracks the internal state of the metrics collector, including frame sequence and telemetry data.
#[derive(Default)]
struct RawDiagramState {
    sequence: u64,                // Tracks the current frame sequence number.
    fill: u8,                     // Indicates frame readiness: 0 (new), 1 (half), 2+ (full).
    actor_count: usize,           // Number of actors being monitored.
    actor_status: Vec<ActorStatus>, // Latest status of each actor.
    total_take_send: Vec<(i64, i64)>, // Aggregated (take, send) counts for channels.
    future_take: Vec<i64>,        // Take counts for the next frame (unmatched sends).
    future_send: Vec<i64>,        // Send counts for the next frame (unmatched takes).
    error_map: HashMap<String, u32>, // Tracks errors for reporting after multiple occurrences.
    actor_last_update: Vec<u64>,  // Last sequence number each actor updated, for detecting stalls.
    #[cfg(debug_assertions)]
    last_instant: Option<Instant> // Timestamp of last frame, useful for debugging timing issues.
}

/// Launches the metrics collector, orchestrating telemetry collection and frame production.
pub(crate) async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    dynamic_senders_vec: Arc<RwLock<Vec<crate::telemetry::metrics_collector::CollectorDetail>>>,
    optional_servers: SteadyTxBundle<DiagramData, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    internal_behavior(context, dynamic_senders_vec, optional_servers).await
}

/// Core logic of the metrics collector, ensuring continuous frame production with robust telemetry handling.
async fn internal_behavior<const GIRTH: usize>(
    context: SteadyActorShadow,
    dynamic_senders_vec: Arc<RwLock<Vec<CollectorDetail>>>,
    optional_servers: SteadyTxBundle<DiagramData, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    let ident = context.identity(); // Unique identifier for this collector instance.
    let mut ctrl = context;         // Control structure for runtime state and configuration.
    let mut state = RawDiagramState::default(); // State for tracking telemetry and frames.
    let mut all_actors_to_scan: Option<Vec<(usize, Box<SteadyRx<ActorStatus>>, ActorIdentity)>> = None; // List of actors to monitor.
    let mut timelords: Option<Vec<&Box<SteadyRx<ActorStatus>>>> = None; // Subset of actors used for timing optimization.
    let mut locked_servers = optional_servers.lock().await; // Access to output channels.
    let mut rebuild_scan_requested = false; // Flag to rebuild the actor scan list.
    let mut trying_to_shutdown = false;     // Indicates shutdown process has started.
    const MAX_TIMELORDS: usize = 5;         // Maximum number of timelords to select.
    let mut consecutive_timeouts = 0;       // Counts consecutive timelord timeouts to trigger rebuild.

    loop {
        // Log iteration details for debugging and performance monitoring.
        if ENABLE_DEBUG_LOGGING {
            debug!(
                "[{:?}] Loop iteration: sequence={}, fill={}, rebuild_requested={}, timeouts={}",
                ident, state.sequence, state.fill, rebuild_scan_requested, consecutive_timeouts
            );
        }

        // Check if the system should continue running or begin shutdown.
        if !ctrl.is_running(&mut || {
            trying_to_shutdown = true;
            i!(is_all_empty_and_closed(Ok(dynamic_senders_vec.read())))
                && i!(state.fill == 0)
                && i!(locked_servers.mark_closed())
        }) {
            break; // Exit loop if shutdown conditions are met.
        }
        let instance_id = ctrl.instance_id; // Current instance ID for filtering telemetry.

        // Rebuild the actor scan list if it's empty or a rebuild is requested due to changes or timeouts.
        if all_actors_to_scan.is_none() || rebuild_scan_requested {
            all_actors_to_scan = gather_valid_actor_telemetry_to_scan(instance_id, &dynamic_senders_vec);
            if ENABLE_DEBUG_LOGGING {
                if let Some(ref scan) = all_actors_to_scan {
                    info!("[{:?}] Gathered {} actors to scan", ident, scan.len());
                } else {
                    info!("[{:?}] No actors to scan", ident);
                }
            }
            timelords = None; // Reset timelords since the scan list has changed.
            state.fill = 15;  // Force immediate frame production after rebuild.
            rebuild_scan_requested = false; // Reset rebuild flag.
        }

        let mut _trigger = "none"; // Debugging aid to track what triggered this iteration.
        let mut _tcount = 0;       // Debugging aid to count timelords.

        {
            if let Some(ref scan) = all_actors_to_scan {
                let mut futures_unordered = FuturesUnordered::new(); // Collection of futures for actor responses.

                if let Some(ref timelords) = timelords {
                    _trigger = "waiting for timelords";
                    _tcount = timelords.len();
                    if ENABLE_DEBUG_LOGGING {
                        info!("[{:?}] Waiting for data from {} timelords", ident, timelords.len());
                    }
                    // Queue futures to check timelord responsiveness.
                    timelords.iter().for_each(|steady_rxf| {
                        futures_unordered.push(future_checking_avail(steady_rxf, steady_config::CONSUMED_MESSAGES_BY_COLLECTOR));
                    });

                    // Wait for a full frame or timeout to ensure frame production isn't blocked.
                    let mut got_full_frame = false;
                    while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                        if full_frame_of_data {
                            got_full_frame = true;
                            if ENABLE_DEBUG_LOGGING {
                                info!("[{:?}] Received full frame from actor id={:?}", ident, id);
                            }
                            break; // Exit once a full frame is received.
                        }
                    }

                    // Track consecutive timeouts to decide when to reselect timelords.
                    if got_full_frame {
                        consecutive_timeouts = 0; // Reset on successful frame.
                    } else {
                        consecutive_timeouts += 1;
                        if ENABLE_DEBUG_LOGGING {
                            info!("[{:?}] Timelords timed out (consecutive: {})", ident, consecutive_timeouts);
                        }
                        if consecutive_timeouts >= 3 { // Rebuild after 3 failures to avoid thrashing.
                            rebuild_scan_requested = true;
                            consecutive_timeouts = 0;
                            if ENABLE_DEBUG_LOGGING {
                                info!("[{:?}] 3 consecutive timeouts, requesting timelord rebuild", ident);
                            }
                        }
                    }
                } else {
                    _trigger = "selecting timelords";
                    if ENABLE_DEBUG_LOGGING {
                        debug!("[{:?}] Selecting new timelords", ident);
                    }

                    // Queue futures to select new timelords from all actors.
                    scan.iter().for_each(|f| {
                        futures_unordered.push(future_checking_avail(&*f.1, steady_config::CONSUMED_MESSAGES_BY_COLLECTOR));
                    });
                    let count = futures_unordered.len().min(MAX_TIMELORDS); // Cap timelord selection.
                    let mut pending_actors: Vec<usize> = scan.iter().map(|(id, _, _)| *id).collect(); // Actors still to check.
                    let mut timelord_collector = Vec::new(); // Selected timelords.
                    let mut non_responsive_actors = Vec::new(); // Actors that didn’t respond.

                    while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                        if full_frame_of_data {
                            let id = id.expect("Internal error: full frame should have an ID");
                            pending_actors.retain(|&x| x != id); // Remove responsive actor.
                            scan.iter().find(|f| f.0 == id).iter().for_each(|rx| {
                                timelord_collector.push(&rx.1); // Add to timelords.
                            });
                            if ENABLE_DEBUG_LOGGING {
                                debug!("[{:?}] Selected timelord: actor id={}", ident, id);
                            }
                            if timelord_collector.len() >= count {
                                break; // Stop once we have enough timelords.
                            }
                        } else if id.is_none() {
                            non_responsive_actors.extend(pending_actors.iter().copied()); // Mark all remaining as non-responsive.
                            break;
                        }
                    }

                    timelords = if timelord_collector.is_empty() { None } else { Some(timelord_collector) };
                    if timelords.is_none() && !non_responsive_actors.is_empty() {
                        let non_responsive_ids = non_responsive_actors
                            .iter()
                            .flat_map(|id| scan.iter().find(|f| f.0 == *id).map(|f| f.2.clone()))
                            .collect::<Vec<_>>();
                        if ENABLE_DEBUG_LOGGING {
                            debug!("[{:?}] No actors responded within {}ms: {:?}", ident, ctrl.frame_rate_ms, non_responsive_ids);
                        }
                    }
                }
            } else {
                _trigger = "no actors to scan";
                yield_now().await; // Yield control if there’s nothing to do.
            }
        }

        // Collect telemetry data and detect structural changes.
        let (nodes, to_pop) = {
            let guard = dynamic_senders_vec.read();
            let dynamic_senders = guard.deref();
            let structure_unchanged = dynamic_senders.len() == state.actor_count;
            if structure_unchanged && !trying_to_shutdown {
                (None, collect_channel_data(&mut state, dynamic_senders)) // No structural update needed.
            } else {
                rebuild_scan_requested = true; // Trigger rebuild on actor count change.
                let is_building = ctrl.is_liveliness_in(&[GraphLivelinessState::Building]);
                if let Some(n) = gather_node_details(&mut state, dynamic_senders, is_building) {
                    (Some(n), collect_channel_data(&mut state, dynamic_senders)) // New structure and data.
                } else {
                    (None, Vec::new()) // Structure update failed, retry next iteration.
                }
            }
        };

        if ENABLE_DEBUG_LOGGING {
            let data: Vec<_> = state.total_take_send.iter().filter(|(take, send)| *take != 0 || *send != 0).collect();
            info!("[{:?}] Collected channel data: {:?}", ident, data);
        }

        // Clean up consumed telemetry data from the queue.
        if !to_pop.is_empty() {
            let mut guard = dynamic_senders_vec.write();
            to_pop.iter().for_each(|ident| {
                guard.iter_mut().for_each(|f| {
                    if f.ident == *ident {
                        f.telemetry_take.pop_front(); // Remove processed telemetry.
                    }
                });
            });
        }

        // Send structural updates if the actor topology has changed.
        if let Some(nodes) = nodes {
            if ENABLE_DEBUG_LOGGING {
                info!("[{:?}] Sending structure details for {} nodes", ident, nodes.len());
            }
            send_structure_details(ident, &mut locked_servers, nodes).await;
        }

        // Produce and send a frame if enough data is collected or shutdown is in progress.
        if state.fill >= 2 || trying_to_shutdown {
            if ENABLE_DEBUG_LOGGING && state.fill >= 2 {
                info!("[{:?}] Sending data details for sequence {}", ident, state.sequence);
            }
            let warn = steady_config::TELEMETRY_SERVER && !trying_to_shutdown; // Warn if server lags.
            let next_frame = send_data_details(ident, &mut locked_servers, &state, warn).await;
            if next_frame {
                state.sequence += 1; // Increment sequence for the next frame.
                state.fill = 0;      // Reset fill state.
                #[cfg(debug_assertions)]
                {
                    state.last_instant = Some(Instant::now()); // Record frame time for debugging.
                }
            }
        }
    }

    if ENABLE_DEBUG_LOGGING {
        debug!("[{:?}] Exited loop: shutdown={}", ident, trying_to_shutdown);
    }
    Ok(())
}

/// Creates a future to check if an actor’s telemetry channel has data available or is closed.
#[inline]
pub(crate) fn future_checking_avail<T: Send + Sync>(steady_rx: &SteadyRx<T>, count: usize) -> BoxFuture<'_, (bool, Option<usize>)> {
    async move {
        let mut guard = steady_rx.lock().await;
        let is_closed = guard.deref_mut().is_closed();
        if !is_closed {
            let result = guard.deref_mut().shared_wait_shutdown_or_avail_units(count).await; // Wait for data or shutdown.
            (result, Some(guard.deref().id())) // Return availability and actor ID.
        } else {
            (false, None) // Channel closed, no data available.
        }
    }.boxed()
}

/// Waits for a full frame of data from actors or times out to prevent frame misses.
async fn full_frame_or_timeout(
    futures_unordered: &mut FuturesUnordered<BoxFuture<'_, (bool, Option<usize>)>>,
    timeout_ms: u64,
) -> Option<(bool, Option<usize>)> {
    debug_assert!(!timeout_ms.is_zero(), "Timeout must be at least 1ms to avoid busy-waiting.");
    select! {
        x = futures_unordered.next().fuse() => x, // Return first available result.
        _ = Delay::new(Duration::from_millis(timeout_ms >> 1)).fuse() => None, // Timeout to ensure progress.
    }
}

/// Verifies if all telemetry channels are empty and closed, indicating safe shutdown.
fn is_all_empty_and_closed(m_channels: LockResult<RwLockReadGuard<'_, Vec<CollectorDetail>>>) -> bool {
    match m_channels {
        Ok(channels) => {
            for c in channels.iter() {
                //last one is the only one which gets marked closed because it is current
                if !c.telemetry_take.is_empty() {
                    let mut idx = c.telemetry_take.len() - 1;
                    if let Some(t) = c.telemetry_take.get(idx) {
                        if !t.is_empty_and_closed() {
                            return false; // Found an active channel, shutdown not complete.
                        } else {
                            while idx > 0 {
                                idx -= 1;
                                if let Some(t) = c.telemetry_take.get(idx) {
                                    if !t.is_empty() {
                                        return false; // Found an active data, shutdown not complete.
                                    }
                                }
                            }
                        }
                    };
                };
            }
            true // All channels are empty and closed.
        }
        Err(_) => false, // Lock error, assume not safe to shut down.
    }
}

/// Builds a list of actors to scan for telemetry, filtering by instance ID and excluding self.
fn gather_valid_actor_telemetry_to_scan(
    version: u32,
    dynamic_senders_vec: &Arc<RwLock<Vec<CollectorDetail>>>,
) -> Option<Vec<(usize, Box<SteadyRx<ActorStatus>>, ActorIdentity)>> {
    let guard = dynamic_senders_vec.read();
    let dynamic_senders = guard.deref();
    let v: Vec<(usize, Box<SteadyRx<ActorStatus>>, ActorIdentity)> = dynamic_senders
        .iter()
        .filter(|f| f.ident.label.name != metrics_collector::NAME) // Exclude metrics collector itself.
        .flat_map(|f| {
            let ident = f.ident.clone();
            f.telemetry_take.iter().filter_map(move |g| {
                g.actor_rx(version).map(|rx| (rx, ident.clone())) // Pair receiver with actor identity.
            })
        })
        .map(|(rx, ident)| (rx.meta_data().id, rx, ident)) // Include actor ID for reference.
        .collect();

    if !v.is_empty() {
        Some(v) // Return list if actors are found.
    } else {
        None // No actors to scan.
    }
}

/// Collects telemetry data from actors, updating state and identifying data to clean up.
fn collect_channel_data(state: &mut RawDiagramState, dynamic_senders: & [CollectorDetail]) -> Vec<ActorIdentity> {
    state.fill = (state.fill + 1) & 0xF; // Increment fill, capped at 15 to avoid overflow.
    let working: Vec<(bool, &CollectorDetail)> = dynamic_senders
        .iter()
        .map(| f| {
            let has_data = if let Some(x) = f.telemetry_take.iter().find(|f|!f.is_empty()) {
                x.consume_send_into(&mut state.total_take_send, &mut state.future_send)
            } else {
                false
            };
            (has_data, f)
        })
        .collect();

    let working: Vec<(bool, &CollectorDetail)> = working
        .iter()
        .map(|(has_data_in, f)| {
            let has_data = if let Some(x) = f.telemetry_take.iter().find(|f|!f.is_empty()) {
                x.consume_take_into(
                    &mut state.total_take_send,
                    &mut state.future_take,
                    &mut state.future_send,
                )
            } else {
                false
            };

            (has_data || *has_data_in, *f) // Combine send and take data availability.
        })
        .collect();

    // Ensure actor_last_update matches actor_count to track responsiveness.
    if state.actor_last_update.len() < state.actor_count {
        state.actor_last_update.resize(state.actor_count, 0);
    }

    let to_pop: Vec<ActorIdentity> = working
        .iter()
        .filter(|(has_data_in, f)| {
            if let Some(act) = f.telemetry_take[0].consume_actor() {
                let actor_id = f.ident.id;
                if actor_id < state.actor_status.len() {
                    state.actor_status[actor_id] = act; // Update existing actor status.
                } else {
                    state.actor_status.resize(actor_id + 1, ActorStatus::default());
                    state.actor_status[actor_id] = act; // Expand and update.
                }
                if actor_id >= state.actor_last_update.len() {
                    state.actor_last_update.resize(actor_id + 1, 0);
                }
                state.actor_last_update[actor_id] = state.sequence; // Mark actor as updated.
                f.telemetry_take.len().gt(&1) && f.telemetry_take[0].is_empty() 
            } else {
                !has_data_in && f.telemetry_take.len().gt(&1) // Clean up if no data and extra telemetry exists.
            }
        })
        .map(|(_, c)| c.ident)
        .collect();
    to_pop // Return identities of telemetry to remove.
}

/// Gathers structural details for new or changed actors, ensuring consistency.
fn gather_node_details(
    state: &mut RawDiagramState,
    dynamic_senders: &[CollectorDetail],
    is_building: bool,
) -> Option<Vec<DiagramData>> {
    let mut matches: Vec<u8> = Vec::new(); // Tracks channel pairing (1=RX, 2=TX, 3=both).
    dynamic_senders.iter().for_each(|x| {
        x.telemetry_take.iter().for_each(|f| {
            f.rx_channel_id_vec().iter().for_each(|meta| {
                if meta.id >= matches.len() {
                    matches.resize(meta.id + 1, 0);
                }
                matches[meta.id] |= 1; // Mark RX channel.
            });
            f.tx_channel_id_vec().iter().for_each(|meta| {
                if meta.id >= matches.len() {
                    matches.resize(meta.id + 1, 0);
                }
                matches[meta.id] |= 2; // Mark TX channel.
            });
        });
    });

    // Check for unpaired channels only when not building, to avoid false positives.
    if !is_building && !matches.iter().all(|x| *x == 3 || *x == 0) {
        matches.iter()
            .enumerate()
            .filter(|(_, x)| **x != 3 && **x != 0)
            .for_each(|(i, x)| {
                dynamic_senders.iter().for_each(|sender| {
                    sender.telemetry_take.iter().for_each(|f| {
                        if x.is_one() {
                            f.rx_channel_id_vec().iter().for_each(|meta| {
                                if meta.id == i {
                                    let msg = format!("Possible missing TX for actor {:?} RX {:?} Channel:{:?}", sender.ident, meta.show_type, i);
                                    let count = state.error_map.entry(msg.clone()).and_modify(|c| *c += 1).or_insert(0);
                                    if *count == 21 {
                                        warn!("{}", msg); // Warn after repeated occurrences.
                                    }
                                }
                            });
                        } else {
                            f.tx_channel_id_vec().iter().for_each(|meta| {
                                if meta.id == i {
                                    let msg = format!("Possible missing RX for actor {:?} TX {:?} Channel:{:?}", sender.ident, meta.show_type, i);
                                    let count = state.error_map.entry(msg.clone()).and_modify(|c| *c += 1).or_insert(0);
                                    if *count == 21 {
                                        warn!("{}", msg);
                                    }
                                }
                            });
                        }
                    });
                });
            });
        return None; // Retry next iteration if errors are found.
    }

    state.error_map.clear(); // Clear errors once structure is valid.

    let max_channels_len = matches.len();
    state.total_take_send.resize(max_channels_len, (0, 0));
    state.future_take.resize(max_channels_len, 0);
    state.future_send.resize(max_channels_len, 0);
    state.actor_last_update.resize(dynamic_senders.len(), 0); // Match actor count.

    let nodes: Vec<DiagramData> = dynamic_senders
        .iter()
        .skip(state.actor_count) // Only process new actors.
        .map(|details| {
            let tt = &details.telemetry_take[0];
            let metadata = tt.actor_metadata();
            let rx_vec = tt.rx_channel_id_vec();
            let tx_vec = tt.tx_channel_id_vec();
            DiagramData::NodeDef(state.sequence, Box::new((
                metadata.clone(),
                rx_vec.into_boxed_slice(),
                tx_vec.into_boxed_slice(),
            )))
        })
        .collect();
    state.actor_count = dynamic_senders.len(); // Update actor count.
    Some(nodes) // Return new node definitions.
}

/// Sends updated structural data to downstream consumers.
async fn send_structure_details(
    ident: ActorIdentity,
    consumer_vec: &mut [MutexGuard<'_, Tx<DiagramData>>],
    nodes: Vec<DiagramData>,
) {
    for consumer in consumer_vec.iter_mut() {
        let to_send = nodes.clone().into_iter();
        for send_me in to_send {
            match consumer.shared_send_async(send_me, ident, SendSaturation::AwaitForRoom).await {
                Success => {}
                Blocked(e) => {
                    error!("Error sending node data {:?}", e); // Log if send fails.
                }
            }
        }
    }
}

/// Sends collected telemetry data as a frame to consumers, ensuring timely delivery.
async fn send_data_details(
    ident: ActorIdentity,
    consumer_vec: &mut [MutexGuard<'_, Tx<DiagramData>>],
    state: &RawDiagramState,
    warn: bool,
) -> bool {
    let mut next_frame = false;
    for consumer in consumer_vec.iter_mut() {
        if consumer.vacant_units().ge(&2) { // Ensure space for at least two messages.
            if !state.actor_status.is_empty() {
                next_frame = true;
                let modified_status: Vec<ActorStatus> = state.actor_status.iter().enumerate().map(|(id, status)| {
                    let frames_missed = if id < state.actor_last_update.len() {
                        state.sequence.saturating_sub(state.actor_last_update[id])
                    } else {
                        0 // New actor, no history yet.
                    };
                    if frames_missed >= 2 {
                        let mut new_status = *status;
                        new_status.await_total_ns = if status.bool_blocking { new_status.unit_total_ns } else { 0 }; // Proxy for stalled state.
                        new_status
                    } else {
                        *status
                    }
                }).collect();

                if let Blocked(e) = consumer.shared_send_async(
                    DiagramData::NodeProcessData(state.sequence, modified_status.into_boxed_slice()),
                    ident,
                    SendSaturation::DebugWarnThenAwait,
                ).await {
                    error!("Error sending node process data {:?}", e);
                }

                if let Blocked(e) = consumer.shared_send_async(
                    DiagramData::ChannelVolumeData(state.sequence, state.total_take_send.clone().into_boxed_slice()),
                    ident,
                    SendSaturation::DebugWarnThenAwait,
                ).await {
                    error!("Error sending channel volume data {:?}", e);
                }
            }
        } else if warn {
            #[cfg(not(test))]
            warn!(
                "{:?} is accumulating frames since consumer is not keeping up, consider increasing capacity of {:?}.",
                ident, consumer.capacity()
            );
        }
    }
    next_frame // Indicate if a frame was successfully sent.
}

/// Stores telemetry details for an actor, including its channels and identity.
pub struct CollectorDetail {
    pub(crate) telemetry_take: VecDeque<Box<dyn RxTel>>, // Queue of telemetry receivers.
    pub(crate) ident: ActorIdentity,                     // Unique actor identifier.
}


#[cfg(test)]
mod metric_collector_tests {
    use super::*;
    use futures_util::stream::FuturesUnordered;
    use std::sync::Arc;
    use parking_lot::RwLock;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::sleep;
    use futures::executor::block_on;
    use crate::{GraphBuilder, SoloAct};

    #[test]
    fn test_raw_diagram_state_default() {
        let state: RawDiagramState = Default::default();
        assert_eq!(state.sequence, 0);
        assert_eq!(state.actor_count, 0);
        assert!(state.actor_status.is_empty());
        assert!(state.total_take_send.is_empty());
        assert!(state.future_take.is_empty());
        assert!(state.future_send.is_empty());
        assert!(state.error_map.is_empty());
    }

    #[test]
    fn test_gather_valid_actor_telemetry_to_scan() {
        let dynamic_senders_vec = Arc::new(RwLock::new(vec![
            CollectorDetail {
                telemetry_take: VecDeque::new(),
                ident: ActorIdentity::new(0, "test_actor", None),
            }
        ]));
        let result = gather_valid_actor_telemetry_to_scan(1, &dynamic_senders_vec);
        assert!(result.is_none());
    }

    #[test]
    fn test_is_all_empty_and_closed() {
        let dynamic_senders_vec = Arc::new(RwLock::new(vec![
            CollectorDetail {
                telemetry_take: VecDeque::new(),
                ident: ActorIdentity::new(0, "test_actor", None),
            }
        ]));
        let result = is_all_empty_and_closed(Ok(dynamic_senders_vec.read()));
        assert!(result);
    }

    #[test]
    fn test_send_structure_details() {
        let ident = ActorIdentity::new(0, "test_actor", None);
        let mut consumer_vec: [MutexGuard<'_, Tx<DiagramData>>; 0] = [];
        let nodes = vec![
            DiagramData::NodeDef(0, Box::new((
                Arc::new(ActorMetaData::default()),
                Box::new([]),
                Box::new([]),
            )))
        ];
        block_on(send_structure_details(ident, &mut consumer_vec, nodes));
    }

    #[test]
    fn test_send_data_details() {
        let ident = ActorIdentity::new(0, "test_actor", None);
        let mut consumer_vec: [MutexGuard<'_, Tx<DiagramData>>; 0] = [];
        let state = RawDiagramState::default();
        let result = block_on(send_data_details(ident, &mut consumer_vec, &state, false));
        assert!(!result);
    }

    #[test]
    fn test_full_frame_or_timeout() {
        let mut futures_unordered = FuturesUnordered::new();
        let telemetry_rate_ms = 1000;
        let result = block_on(full_frame_or_timeout(&mut futures_unordered, telemetry_rate_ms));
        assert!(result.is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn test_actor() -> Result<(), Box<dyn std::error::Error>> {
        use isahc::AsyncReadResponseExt;

        //only run this locally where we can open the default port
        if !std::env::var("GITHUB_ACTIONS").is_ok() {
            // Smallest possible graph
            // It is not recommended to inline actors this way, but it is possible.
            let gen_count = Arc::new(AtomicUsize::new(0));

            // Special test graph which does NOT fail fast but instead shows the prod behavior of restarting actors.
            let mut graph = GraphBuilder::for_testing()
                .with_telemtry_production_rate_ms(40)
                .with_iouring_queue_length(8)
                .with_telemetry_metric_features(true)
                .build(());


            let (tx, rx) = graph.channel_builder()
                .with_capacity(300)
                .with_no_refresh_window()
                .build_channel();


            graph.actor_builder()
                .with_no_refresh_window()
                .with_name("generator")
                .build(move |mut context| {
                    let tx = tx.clone();
                    let count = gen_count.clone();
                    async move {
                        let mut tx = tx.lock().await;
                        while context.is_running(&mut || tx.mark_closed()) {
                            let x = count.fetch_add(1, Ordering::SeqCst);
                            //info!("attempted sent: {:?}", count.load(Ordering::SeqCst));

                            if x >= 10 {
                                context.request_shutdown().await;
                                continue;
                            }
                            let _ = context.send_async(&mut tx, x.to_string(), SendSaturation::AwaitForRoom).await;
                        }
                        Ok(())
                    }
                },SoloAct);

            let consume_count = Arc::new(AtomicUsize::new(0));
            //let check_count = consume_count.clone();

            graph.actor_builder()
                .with_name("consumer")
                .build(move |mut context| {
                    let rx = rx.clone();
                    let count = consume_count.clone();
                    async move {
                        let mut rx = rx.lock().await;
                        while context.is_running(&mut || rx.is_closed() && rx.is_empty()) {
                            if let Some(_packet) = rx.shared_try_take() {
                                count.fetch_add(1, Ordering::SeqCst);
                                // info!("received: {:?}", count.load(Ordering::SeqCst));
                            }
                        }
                        Ok(())
                    }
                }, SoloAct);

            graph.start();

            //// now confirm we can see the telemetry collected into metrics_collector
            //wait for one page of telemetry
            sleep(Duration::from_millis(graph.telemetry_production_rate_ms() * 40));

            //hit the telemetry site and validate if it returns
            // this test will only work if the feature is on
            // hit 127.0.0.1:9100/metrics using isahc
            block_on(async {
                match isahc::get_async("http://127.0.0.1:9100/metrics").await {
                    Ok(mut response) => {
                        assert_eq!(200, response.status().as_u16());
                        // warn!("ok metrics");
                        let body = response.text().await;
                        info!("metrics: {:?}", body); //TODO: add more checks
                    }
                    Err(e) => {
                        warn!("failed to get metrics: {:?}", e);
                        // //this is only an error if the feature is not on
                        // #[cfg(feature = "prometheus_metrics")]
                        // {
                        //     panic!("failed to get metrics: {:?}", e);
                        // }
                    }
                };
                match isahc::get_async("http://127.0.0.1:9100/graph.dot").await {
                    Ok(response) => {
                        assert_eq!(200, response.status().as_u16());
                        warn!("ok graph");

                        //let body = response.text().await;
                        //info!("graph: {}", body); //TODO: add more checks
                    }
                    Err(e) => {
                        warn!("failed to get metrics: {:?}", e);
                        // //this is only an error if the feature is not on
                        #[cfg(any(
                            feature = "telemetry_server_builtin",
                            feature = "telemetry_server_cdn"
                        ))]
                        {
                            //panic!("failed to get metrics: {:?}", e);
                        }
                    }
                };
            });

            graph.request_shutdown();
            graph.block_until_stopped(Duration::from_secs(3))
        } else {
            Ok(())
        }
    }

}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use parking_lot::RwLock;
    use std::collections::{VecDeque};
    use std::sync::Arc;
    use futures::executor::block_on;
    use futures::future::BoxFuture;
    use futures::stream::FuturesUnordered;

    #[test]
    fn test_raw_diagram_state_default() {
        let mut state = RawDiagramState::default();
        // Defaults
        assert_eq!(state.sequence, 0);
        assert_eq!(state.fill, 0);
        assert_eq!(state.actor_count, 0);
        assert!(state.actor_status.is_empty());
        assert!(state.total_take_send.is_empty());
        assert!(state.future_take.is_empty());
        assert!(state.future_send.is_empty());
        assert!(state.error_map.is_empty());
        assert!(state.actor_last_update.is_empty());
    }

    #[test]
    fn test_is_all_empty_and_closed_ok() {
        // single CollectorDetail with no telemetry entries => empty and closed
        let vec: Arc<RwLock<Vec<CollectorDetail>>> = Arc::new(RwLock::new(vec![
            CollectorDetail {
                telemetry_take: VecDeque::new(),
                ident: ActorIdentity::new(1, "a", None),
            }
        ]));
        let lock = Ok(vec.read());
        assert!(is_all_empty_and_closed(lock));
    }


    #[test]
    fn test_full_frame_or_timeout_empty() {
        let mut fu: FuturesUnordered<BoxFuture<'_, (bool, Option<usize>)>> = FuturesUnordered::new();
        let res = block_on(full_frame_or_timeout(&mut fu, 100)); //for testing
        assert!(res.is_none(), "Timeout with no futures returns None");
    }

    #[test]
    fn test_collect_channel_data_empty() {
        let mut state = RawDiagramState::default();
        // two rounds to advance fill
        let to_pop = collect_channel_data(&mut state, &[]);
        assert_eq!(to_pop.len(), 0);
        assert_eq!(state.fill, 1);
        let to_pop2 = collect_channel_data(&mut state, &[]);
        assert_eq!(to_pop2.len(), 0);
        assert_eq!(state.fill, 2);
    }

    #[test]
    fn test_gather_node_details_building() {
        let mut state = RawDiagramState::default();
        let senders: Vec<CollectorDetail> = Vec::new();
        // when building (is_building=true), even mismatched channels should yield Some(empty)
        let nodes = gather_node_details(&mut state, &senders, true);
        assert!(nodes.is_some());
        assert!(nodes.expect("internal error").is_empty());
        // actor_count updated
        assert_eq!(state.actor_count, 0);
    }
}
