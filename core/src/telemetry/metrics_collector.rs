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
use num_traits::One;
use crate::graph_liveliness::ActorIdentity;
use crate::{GraphLivelinessState, SteadyRx};
use crate::commander::SteadyCommander;
#[allow(unused_imports)]
use crate::commander_context::SteadyContext;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::SendOutcome::{Blocked, Success};
use crate::steady_rx::*;
use crate::steady_tx::*;
use crate::telemetry::metrics_collector;

pub const NAME: &str = "metrics_collector";

/// Represents different types of data that can be sent to the diagram.
#[derive(Clone, Debug)]
pub enum DiagramData {
    /// Node definition data, containing metadata about the actor and its channels.
    #[allow(clippy::type_complexity)]
    NodeDef(
        u64,
        Box<(   //NOTE: this box is here so all the enums are the same size as they share a channel
            Arc<ActorMetaData>,
            Box<[Arc<ChannelMetaData>]>,
            Box<[Arc<ChannelMetaData>]>,
        )>,
    ),
    /// Channel volume data, containing tuples of (take, send) values.
    ChannelVolumeData(u64, Box<[(i64, i64)]>),
    /// Node process data, containing the status of actors.
    NodeProcessData(u64, Box<[ActorStatus]>),
}

/// Maintains the raw state of the diagram, including sequence, actor count, status, and channel data.
#[derive(Default)]
struct RawDiagramState {
    sequence: u64,
    fill: u8,   // 0-new, 1-half, 2-full
    actor_count: usize,
    actor_status: Vec<ActorStatus>,
    total_take_send: Vec<(i64, i64)>,
    future_take: Vec<i64>, // These are for the next frame since we do not have the matching sent yet.
    future_send: Vec<i64>, // These are for the next frame if we ended up with too many items.
    error_map:HashMap<String,u32>,
    actor_last_update: Vec<u64>, //last sequence number for each actor to know if it stops responding
    #[cfg(debug_assertions)]
    last_instant: Option<Instant> //for testing do not remove

}



/// Runs the metrics collector actor, collecting telemetry data from all actors and consolidating it for sharing and logging.
pub(crate) async fn run<const GIRTH: usize>(
    context: SteadyContext,
    dynamic_senders_vec: Arc<RwLock<Vec<crate::telemetry::metrics_collector::CollectorDetail>>>,
    optional_servers: SteadyTxBundle<DiagramData, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    internal_behavior(context, dynamic_senders_vec, optional_servers).await
}

async fn internal_behavior<const GIRTH: usize>(
    context: SteadyContext,
    dynamic_senders_vec: Arc<RwLock<Vec<CollectorDetail>>>,
    optional_servers: SteadyTxBundle<DiagramData, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    let ident = context.identity();

    let mut ctrl = context;
    // #[cfg(all(feature = "telemetry_on_telemetry", any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
    // let mut ctrl = {
    //     info!("should not happen");
    //     context.into_monitor((ctrl, [], optional_servers)
    // };

    let mut state = RawDiagramState::default();
    let mut all_actors_to_scan: Option<Vec<(usize, Box<SteadyRx<ActorStatus>>)>> = None;
    let mut timelords: Option<Vec<&Box<SteadyRx<ActorStatus>>>> = None;

    let mut locked_servers = optional_servers.lock().await;

    let mut rebuild_scan_requested: bool = false;
    let mut trying_to_shutdown = false;
    loop {
        if !ctrl.is_running(&mut || {
                            trying_to_shutdown = true;
                            is_all_empty_and_closed(Ok(dynamic_senders_vec.read())) 
                                &&  state.fill == 0
                                && locked_servers.mark_closed()
                        }) {
            break;
        }
        let instance_id = ctrl.instance_id;

        //warn!("all actors to scan {:?}",all_actors_to_scan);

        if all_actors_to_scan.is_none() || rebuild_scan_requested {
            //flat map used so the index is not the channel id
            all_actors_to_scan = gather_valid_actor_telemetry_to_scan(instance_id, &dynamic_senders_vec);
            timelords = None;
            state.fill=15;//force a frame push because we just defined new resources
            rebuild_scan_requested = false;
        }

        let mut _trigger = "none";
        let mut _tcount = 0;
        if let Some(ref scan) = all_actors_to_scan {

            //NOTE: this value is half the data points needed to make up a frame by design so we are
            //      only waiting on half the channel this means we will need to do this twice before
            //      we can send a full frame of data. This is tracked in RawDiagramState::fill
            let mut futures_unordered = FuturesUnordered::new();
            if let Some(ref timelords) = timelords {
                _trigger = "waiting for timelords";
                _tcount = timelords.len();
                assert!(!timelords.is_empty(), "internal error, timelords should not be empty");

                timelords.iter().for_each(|steady_rxf|
                    futures_unordered.push(future_checking_avail(steady_rxf, steady_config::CONSUMED_MESSAGES_BY_COLLECTOR))
                );

                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    _trigger = "we did loop in timelords";
                    if full_frame_of_data {
                        _trigger = "found a full frame in timelords";
                        break;
                    } else if id.is_none() {
                        rebuild_scan_requested = true;
                    }
                }
            } else {
                const MAX_TIMELORDS: usize = 7;
                _trigger = "selecting timelords";
                // reminder: scan is flat mapped so the index is NOT the channel id
                scan.iter().for_each(|f| futures_unordered.push(future_checking_avail(&*f.1, steady_config::CONSUMED_MESSAGES_BY_COLLECTOR))   );
                let count = futures_unordered.len().min(MAX_TIMELORDS);
                let mut timelord_collector = Vec::new();
                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    if full_frame_of_data {
                        let id = id.expect("internal error, every true has an index");
                        scan.iter().find(|f| f.0 == id)
                                   .iter()
                                   .for_each(|rx| timelord_collector.push(&rx.1));

                        if timelord_collector.len() >= count {
                                _trigger = "found count of full frames and new timelords list";
                                break;
                        }

                    }
                }
                timelords = if timelord_collector.is_empty() { None } else { Some(timelord_collector) };
            }
        } else {
            _trigger = "no actors to scan";
            yield_now().await;
        }


        let (nodes, to_pop) = {
            
            let guard = dynamic_senders_vec.read();                      
            let dynamic_senders = guard.deref();

            let structure_unchanged = dynamic_senders.len() == state.actor_count;
            if structure_unchanged {
                (None, collect_channel_data(&mut state, dynamic_senders))
            } else {
                rebuild_scan_requested = true;

                //when we are still building we will not report missing channel errors
                let is_building = ctrl.is_liveliness_in(&[GraphLivelinessState::Building]);

                if let Some(n) = gather_node_details(&mut state, dynamic_senders, is_building) {
                    (Some(n), collect_channel_data(&mut state, dynamic_senders))
                } else {
                    (None, Vec::new())
                }
            }
           
        };

        if !to_pop.is_empty() {
            let mut guard = dynamic_senders_vec.write();       
            let dynamic_senders = guard.deref_mut();
            to_pop.iter().for_each(|ident| {
                #[cfg(debug_assertions)]
                trace!("swapping to new telemetry channels for {:?}", ident);
                dynamic_senders.iter_mut().for_each(|f| {
                    if f.ident == *ident {
                        f.telemetry_take.pop_front();
                    }
                });
            });        
        }


        if let Some(nodes) = nodes {
            send_structure_details(ident, &mut locked_servers, nodes).await;
        }
        

        // we wait and fire only when we know we have a full frame which is two rounds of loading
        if state.fill>=2 || trying_to_shutdown {
            #[cfg(debug_assertions)]
            { //only check this when debug is on.
              //and we are not shutting down
                                
                if !ctrl.is_liveliness_stop_requested() {
                    if let Some(i) = state.last_instant {
                        let measured = i.elapsed().as_millis() as u64;
                        let margin = 1.max(1 + (ctrl.frame_rate_ms >> 1));

                        // only concerned with too fast because too slow is a normal case when
                        // all actors are waiting as they should for new work so we get no updates.
                        // at this time there is no way to distinguish this from all 7 selected
                        // timelords calling poorly written blocking code simultaneously (this is unlikely)
                        //TODO: in that case however we should drop CPU usage to zero for all with no actro state in this frame.

                        if measured < (ctrl.frame_rate_ms - (margin - (margin >> 2))) && _tcount > 0 {
                            warn!("frame rate is far too fast {:?}ms vs {:?}ms seq:{:?} fill:{:?} trigger:{:?}  other:{:?}"
                                , measured
                                , ctrl.frame_rate_ms
                                , state.sequence
                                , state.fill
                                , _trigger
                                , _tcount);
                        }
                    }
                }
            }

            
           let warn = steady_config::TELEMETRY_SERVER && !trying_to_shutdown;
            let next_frame = send_data_details(ident, &mut locked_servers, &state, warn).await;
            if next_frame {
                state.sequence += 1;
                state.fill = 0;
                #[cfg(debug_assertions)]
                {
                    state.last_instant = Some(Instant::now());
                }
            }
        } 
    }
    
    Ok(())
}



#[inline]
pub(crate) fn future_checking_avail<T: Send + Sync>(steady_rx: &SteadyRx<T>, count: usize) -> BoxFuture<'_, (bool, Option<usize>)> {
    async move {
        let mut guard = steady_rx.lock().await;
        let is_closed = guard.deref_mut().is_closed();
        if !is_closed {
            let result: bool = guard.deref_mut().shared_wait_shutdown_or_avail_units(count).await;
            (result, Some(guard.deref().id()))
        } else {
            (false, None)
        }
    }
        .boxed()
}

/// Waits for either a full frame of data or a timeout.
async fn full_frame_or_timeout(
    futures_unordered: &mut FuturesUnordered<BoxFuture<'_, (bool, Option<usize>)>>,
    telemetry_rate_ms: u64,
) -> Option<(bool, Option<usize>)> {
    select! {
        x = futures_unordered.next().fuse() => x,
        _ = Delay::new(Duration::from_millis(telemetry_rate_ms >> 1)).fuse() => None,
    }
}

/// Checks if all telemetry channels are empty and closed.
fn is_all_empty_and_closed(m_channels: LockResult<RwLockReadGuard<'_, Vec<CollectorDetail>>>) -> bool {
    match m_channels {
        Ok(channels) => {
            for c in channels.iter() {
                for t in c.telemetry_take.iter() {
                    if !t.is_empty_and_closed() {
                        return false;
                    }
                }
            }
            true
        }
        Err(_) => false,
    }
}


/// Gathers valid actor telemetry to scan.
fn gather_valid_actor_telemetry_to_scan(
    version: u32,
    dynamic_senders_vec: &Arc<RwLock<Vec<CollectorDetail>>>,
) -> Option<Vec<(usize,Box<SteadyRx<ActorStatus>>)>> {
    let guard = dynamic_senders_vec.read();
    
    let dynamic_senders = guard.deref();
    let v: Vec<(usize, Box<SteadyRx<ActorStatus>>)> = dynamic_senders
        .iter()
        .filter(|f| f.ident.label.name != metrics_collector::NAME)
        .flat_map(|f| f.telemetry_take.iter().filter_map(|g| g.actor_rx(version)))
        .map(|f| (f.meta_data().id, f))
        .collect();

    if !v.is_empty() {
        //due to flat map the channel id is not the index position
        Some(v)
    } else {
        None
    }

}

/// Collects channel data from the state and dynamic senders.
fn collect_channel_data(state: &mut RawDiagramState, dynamic_senders: &[CollectorDetail]) -> Vec<ActorIdentity> {
    state.fill = (state.fill + 1) & 0xF;
    let working: Vec<(bool, &CollectorDetail)> = dynamic_senders
        .iter()
        .map(|f| {
            let has_data = f.telemetry_take[0].consume_send_into(&mut state.total_take_send, &mut state.future_send);
            #[cfg(debug_assertions)]
            if f.telemetry_take.len() > 1 {
                trace!("can see new telemetry channel")
            }
            (has_data, f)
        })
        .collect();

    let working: Vec<(bool, &CollectorDetail)> = working
        .iter()
        .map(|(has_data_in, f)| {
            let has_data = f.telemetry_take[0].consume_take_into(
                &mut state.total_take_send,
                &mut state.future_take,
                &mut state.future_send,
            );
            (has_data || *has_data_in, *f)
        })
        .collect();

    // Ensure actor_last_update matches actor_count
    if state.actor_last_update.len() < state.actor_count {
        state.actor_last_update.resize(state.actor_count, 0);
    }

    let to_pop: Vec<ActorIdentity> = working
        .iter()
        .filter(|(has_data_in, f)| {
            if let Some(act) = f.telemetry_take[0].consume_actor() {
                let actor_id = f.ident.id;
                if actor_id < state.actor_status.len() {
                    state.actor_status[actor_id] = act;
                } else {
                    state.actor_status.resize(actor_id + 1, ActorStatus::default());
                    state.actor_status[actor_id] = act;
                }
                // Record the sequence number when this actor sent data
                if actor_id >= state.actor_last_update.len() {
                    state.actor_last_update.resize(actor_id + 1, 0);
                }
                state.actor_last_update[actor_id] = state.sequence;
                false
            } else {
                !has_data_in && f.telemetry_take.len().gt(&1)
            }
        })
        .map(|(_, c)| c.ident)
        .collect();
    to_pop
}

/// Gathers node details from the state and dynamic senders.
fn gather_node_details(
    state: &mut RawDiagramState,
    dynamic_senders: &[CollectorDetail],
    is_building: bool
) -> Option<Vec<DiagramData>> {
    let mut matches: Vec<u8> = Vec::new();
    dynamic_senders.iter().for_each(|x| {
        x.telemetry_take.iter().for_each(|f| {
            f.rx_channel_id_vec().iter().for_each(|meta| {
                if meta.id >= matches.len() {
                    matches.resize(meta.id + 1, 0);
                }
                matches[meta.id] |= 1;
            });
            f.tx_channel_id_vec().iter().for_each(|meta| {
                if meta.id >= matches.len() {
                    matches.resize(meta.id + 1, 0);
                }
                matches[meta.id] |= 2;
            });
        });
    });

   // only do error checking if the system is running.
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
                                      let msg = format!("Possible missing TX for actor {:?} RX {:?} Channel:{:?} ", sender.ident, meta.show_type, i);
                                      let count = state.error_map.entry(msg.clone()).and_modify(|c| {*c += 1u32;}).or_insert(0u32);
                                      if *count ==21 {
                                          warn!("{}",msg);
                                      }

                                  }
                              });
                          } else {
                              f.tx_channel_id_vec().iter().for_each(|meta| {
                                  if meta.id == i {
                                      let msg = format!("Possible missing RX for actor {:?} TX {:?} Channel:{:?} ", sender.ident, meta.show_type, i);
                                      let count = state.error_map.entry(msg.clone()).and_modify(|c| {*c += 1u32;}).or_insert(0u32);
                                      if *count==21 {
                                          warn!("{}",msg);
                                      }
                                  }
                              });
                          }
                      });
                  });

        });
       // Do not launch if any error is found instead try again.
        return None;
    }

    state.error_map.clear(); //no need to hold these counts since we are now clear

    let max_channels_len = matches.len();

    state.total_take_send.resize(max_channels_len, (0, 0));
    state.future_take.resize(max_channels_len, 0);
    state.future_send.resize(max_channels_len, 0);
    // Resize actor_last_update to match the number of dynamic senders
    state.actor_last_update.resize(dynamic_senders.len(), 0);

    let nodes: Vec<DiagramData> = dynamic_senders
        .iter()
        .skip(state.actor_count)
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
    state.actor_count = dynamic_senders.len();
    Some(nodes)
}

/// Sends structure details to the consumer.
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
                    error!("error sending node data {:?}", e);
                }
            }
        }
    }
}

/// Sends data details to the consumer(s).
async fn send_data_details(
    ident: ActorIdentity,
    consumer_vec: &mut [MutexGuard<'_, Tx<DiagramData>>],
    state: &RawDiagramState,
    warn: bool,
) -> bool {
    let mut next_frame = false;
    for consumer in consumer_vec.iter_mut() {
        if consumer.vacant_units().ge(&2) {
            if !state.actor_status.is_empty() {
                next_frame = true;
                // Create a modified actor_status with adjusted CPU usage
                let modified_status: Vec<ActorStatus> = state.actor_status.iter().enumerate().map(|(id, status)| {
                    let frames_missed = if id < state.actor_last_update.len() {
                        state.sequence.saturating_sub(state.actor_last_update[id])
                    } else {
                        0 // New actor, no updates yet
                    };
                    if frames_missed >= 2 {
                        let mut new_status = *status;
                        // Since cpu_usage isn’t in ActorStatus, use unit_total_ns as a proxy
                        new_status.await_total_ns = if status.bool_blocking { new_status.unit_total_ns } else { 0 };
                        new_status
                    } else {
                        *status
                    }
                }).collect();

                if let Blocked(e) = consumer.shared_send_async(
                    DiagramData::NodeProcessData(state.sequence, modified_status.into_boxed_slice()),
                    ident,
                    SendSaturation::DebugWarnThenAwait,
                )
                    .await
                {
                    error!("error sending node process data {:?}", e);
                }

                if let Blocked(e) = consumer.shared_send_async(
                    DiagramData::ChannelVolumeData(state.sequence, state.total_take_send.clone().into_boxed_slice()),
                    ident,
                    SendSaturation::DebugWarnThenAwait,
                )
                    .await
                {
                    error!("error sending node process data {:?}", e);
                }
            }
        } else if warn {
            #[cfg(not(test))]
            {
                warn!(
                    "{:?} is accumulating frames since consumer of collected frames is not keeping up, perhaps capacity of {:?} may need to be increased.",
                    ident, consumer.capacity()
                );
            }
        }
    }
    next_frame
}

/// Details of the telemetry collector.
pub struct CollectorDetail {
    pub(crate) telemetry_take: VecDeque<Box<dyn RxTel>>,
    pub(crate) ident: ActorIdentity,
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
    use crate::GraphBuilder;

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
                .build_spawn(move |mut context| {
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
                });

            let consume_count = Arc::new(AtomicUsize::new(0));
            //let check_count = consume_count.clone();

            graph.actor_builder()
                .with_name("consumer")
                .build_spawn(move |mut context| {
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
                });

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
    use std::collections::{VecDeque, HashMap};
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
        let res = block_on(full_frame_or_timeout(&mut fu, 100));
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
