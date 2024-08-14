use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, LockResult};
use std::sync::{RwLock, RwLockReadGuard};
use std::time::Duration;

#[allow(unused_imports)]
use log::*; // Allow unused import

use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
#[allow(unused_imports)]
use crate::{steady_config, SendSaturation, SteadyContext, SteadyTxBundle, yield_now, into_monitor};

use futures::future::*;
use futures_timer::Delay;
use futures_util::{select, StreamExt};
use futures_util::lock::MutexGuard;
use futures_util::stream::FuturesUnordered;
use num_traits::One;
use crate::graph_liveliness::ActorIdentity;
use crate::GraphLivelinessState;
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
        Box<(
            Arc<ActorMetaData>,
            Box<[Arc<ChannelMetaData>]>,
            Box<[Arc<ChannelMetaData>]>,
        )>,
    ),
    /// Channel volume data, containing tuples of (take, send) values.
    ChannelVolumeData(u64, Box<[(i128, i128)]>),
    /// Node process data, containing the status of actors.
    NodeProcessData(u64, Box<[ActorStatus]>),
}

/// Maintains the raw state of the diagram, including sequence, actor count, status, and channel data.
#[derive(Default)]
struct RawDiagramState {
    sequence: u64,
    actor_count: usize,
    actor_status: Vec<ActorStatus>,
    total_take_send: Vec<(i128, i128)>,
    future_take: Vec<i128>, // These are for the next frame since we do not have the matching sent yet.
    future_send: Vec<i128>, // These are for the next frame if we ended up with too many items.
    error_map:HashMap<String,u32>
}

/// Locks an optional `SteadyTx` and returns a future resolving to a `MutexGuard`.
// #[allow(clippy::type_complexity)]
// fn lock_if_some<'a, T: Send + 'a + Sync>(
//     opt_lock: &'a Option<SteadyTx<T>>,
// ) -> Pin<Box<dyn Future<Output = Option<MutexGuard<'a, Tx<T>>>> + Send + 'a>> {
//     Box::pin(async move {
//         match opt_lock {
//             Some(lock) => Some(lock.lock().await),
//             None => None,
//         }
//     })
// }

/// Runs the metrics collector actor, collecting telemetry data from all actors and consolidating it for sharing and logging.
pub(crate) async fn run<const GIRTH: usize>(
    context: SteadyContext,
    dynamic_senders_vec: Arc<RwLock<Vec<CollectorDetail>>>,
    optional_servers: SteadyTxBundle<DiagramData, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    let ident = context.identity();

    let ctrl = context;
    // #[cfg(all(feature = "telemetry_on_telemetry", any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
    // let mut ctrl = {
    //     info!("should not happen");
    //     into_monitor!(ctrl, [], optional_servers)
    // };

    let mut state = RawDiagramState::default();
    let mut all_actors_to_scan: Option<Vec<Box<dyn RxDef>>> = None;
    let mut timelords: Option<Vec<usize>> = None;

    let mut locked_servers = optional_servers.lock().await;

    let mut rebuild_scan_requested: bool = false;
    let mut is_shutting_down = false;



    loop {

        let confirm_shutdown = &mut || {
            is_shutting_down = true;
            is_all_empty_and_closed(dynamic_senders_vec.read())
                && locked_servers.mark_closed()
        };

        // #[cfg(feature = "telemetry_on_telemetry")]
        // ctrl.relay_stats_smartly();

        //  ctrl.is_running(&mut || rxg.is_empty() && rxg.is_closed())
        //let _clean = wait_for_all!(ctrl.wait_vacant_units(&mut tick_counts_tx,1)  ).await;
        if !ctrl.is_running(confirm_shutdown) {
            break;
        }
        let instance_id = ctrl.instance_id;

        //warn!("all actors to scan {:?}",all_actors_to_scan);

        if all_actors_to_scan.is_none() || rebuild_scan_requested {
            all_actors_to_scan = gather_valid_actor_telemetry_to_scan(instance_id, &dynamic_senders_vec);


            timelords = None;
            rebuild_scan_requested = false;
        }


        if let Some(ref scan) = all_actors_to_scan {
            let mut futures_unordered = FuturesUnordered::new();
            if let Some(ref timelords) = timelords {
                timelords.iter().for_each(|i| scan.get(*i).iter().for_each(|f| futures_unordered.push(f.wait_avail_units(steady_config::CONSUMED_MESSAGES_BY_COLLECTOR))));
                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    if full_frame_of_data {
                        break;
                    } else if id.is_none() {
                        rebuild_scan_requested = true;
                    }
                }
            } else {
                scan.iter().for_each(|f| futures_unordered.push(f.wait_avail_units(steady_config::CONSUMED_MESSAGES_BY_COLLECTOR)));
                let count = 5;
                let mut timelord_collector = Vec::new();
                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    if full_frame_of_data {
                        timelord_collector.push(id.expect("internal error, every true has an index"));
                        if timelord_collector.len() >= count {
                            break;
                        }
                    }
                }
                timelords = if timelord_collector.is_empty() { None } else { Some(timelord_collector) };
            }
        } else {
            yield_now().await;
        }


        let (nodes, to_pop) = {
            match dynamic_senders_vec.read() {
                Ok(guard) => {
                    let dynamic_senders = guard.deref();

                    let structure_unchanged = dynamic_senders.len() == state.actor_count;
                    if structure_unchanged {
                        (None, collect_channel_data(&mut state, dynamic_senders))
                    } else {
                        all_actors_to_scan = None;
                        rebuild_scan_requested = true;

                        //when we are still building we will not report missing channel errors
                        let is_building = ctrl.is_liveliness_in(&[GraphLivelinessState::Building], false);

                        if let Some(n) = gather_node_details(&mut state, dynamic_senders, is_building) {
                            (Some(n), collect_channel_data(&mut state, dynamic_senders))
                        } else {
                            (None, Vec::new())
                        }
                    }
                }
                Err(_) => (None, Vec::new()),
            }
        };

        if !to_pop.is_empty() {
            match dynamic_senders_vec.write() {
                Ok(mut guard) => {
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
                Err(_) => {
                    continue;
                }
            }
        }


        if let Some(nodes) = nodes {
            send_structure_details(ident, &mut locked_servers, nodes).await;
        }
        let warn = !is_shutting_down && steady_config::TELEMETRY_SERVER;
        let next_frame = send_data_details(ident, &mut locked_servers, &state, warn).await;
        if next_frame {
            state.sequence += 1;
        }
    }
    Ok(())
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
) -> Option<Vec<Box<dyn RxDef>>> {
    if let Ok(guard) = dynamic_senders_vec.read() {
        let dynamic_senders = guard.deref();
        let v: Vec<Box<dyn RxDef>> = dynamic_senders
            .iter()
            .filter(|f| f.ident.label.name != metrics_collector::NAME)
            .flat_map(|f| f.telemetry_take.iter().filter_map(|g| g.actor_rx(version)))
            .collect();

        if !v.is_empty() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    }
}

/// Collects channel data from the state and dynamic senders.
fn collect_channel_data(state: &mut RawDiagramState, dynamic_senders: &[CollectorDetail]) -> Vec<ActorIdentity> {
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
                                      if *count ==5 {
                                          warn!("{}",msg);
                                      }

                                  }
                              });
                          } else {
                              f.tx_channel_id_vec().iter().for_each(|meta| {
                                  if meta.id == i {
                                      let msg = format!("Possible missing RX for actor {:?} TX {:?} Channel:{:?} ", sender.ident, meta.show_type, i);
                                      let count = state.error_map.entry(msg.clone()).and_modify(|c| {*c += 1u32;}).or_insert(0u32);
                                      if *count==5 {
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

    let nodes: Vec<DiagramData> = dynamic_senders
        .iter()
        .skip(state.actor_count)
        .map(|details| {
            let tt = &details.telemetry_take[0];
            let metadata = tt.actor_metadata();
            let rx_vec = tt.rx_channel_id_vec();
            let tx_vec = tt.tx_channel_id_vec();

            let dd = Box::new((
                metadata.clone(),
                rx_vec.into_boxed_slice(),
                tx_vec.into_boxed_slice(),
            ));

            DiagramData::NodeDef(state.sequence, dd)
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
            match consumer.shared_send_async(send_me, ident, SendSaturation::IgnoreAndWait).await {
                Ok(_) => {}
                Err(e) => {
                    error!("error sending node data {:?}", e);
                }
            }
        }
    }
}

/// Sends data details to the consumer.
async fn send_data_details(
    ident: ActorIdentity,
    consumer_vec: &mut [MutexGuard<'_, Tx<DiagramData>>],
    state: &RawDiagramState,
    warn: bool,
) -> bool {
    let mut next_frame = false;
    for consumer in consumer_vec.iter_mut() {

        if consumer.vacant_units().ge(&2) {

            //do not write if we have no actors
            if !state.actor_status.is_empty() {
                next_frame = true; //only increase frame when we have real actors in play
                if let Err(e) = consumer.shared_send_async(
                    DiagramData::NodeProcessData(state.sequence, state.actor_status.clone().into_boxed_slice()),
                    ident,
                    SendSaturation::IgnoreInRelease,
                )
                    .await
                {
                    error!("error sending node process data {:?}", e);
                }

                if let Err(e) = consumer.shared_send_async(
                    DiagramData::ChannelVolumeData(state.sequence, state.total_take_send.clone().into_boxed_slice()),
                    ident,
                    SendSaturation::IgnoreInRelease,
                )
                    .await
                {
                    error!("error sending node process data {:?}", e);
                }
            }

        } else if warn {
            #[cfg(not(test))]
            warn!(
                "{:?} is accumulating frames since consumer is not keeping up, perhaps capacity of {:?} may need to be increased.",
                ident, consumer.capacity()
            );

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
mod tests {
    use super::*;
    use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
    use crate::{steady_config, SendSaturation, SteadyContext, SteadyTxBundle};
    use futures_util::stream::FuturesUnordered;
    use std::sync::Arc;
    use std::sync::{RwLock, Mutex as StdMutex};
    use std::collections::VecDeque;
    use std::error::Error;
    use futures::executor::block_on;
    use futures_util::lock::Mutex;
    use std::time::Duration;

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

    // #[test]
    // fn test_collect_channel_data() {
    //     let mut state = RawDiagramState::default();
    //     let dynamic_senders = vec![
    //         CollectorDetail {
    //             telemetry_take: VecDeque::new(),
    //             ident: ActorIdentity { id: 0, name: "test_actor" },
    //         }
    //     ];
    //     let to_pop = collect_channel_data(&mut state, &dynamic_senders);
    //     assert!(to_pop.is_empty());
    // }

    #[test]
    fn test_gather_valid_actor_telemetry_to_scan() {
        let dynamic_senders_vec = Arc::new(RwLock::new(vec![
            CollectorDetail {
                telemetry_take: VecDeque::new(),
                ident: ActorIdentity::new(0, "test_actor", None ),
            }
        ]));
        let result = gather_valid_actor_telemetry_to_scan(1, &dynamic_senders_vec);
        assert!(result.is_none());
    }

    // #[test]
    // fn test_gather_node_details() {
    //     let mut state = RawDiagramState::default();
    //     let dynamic_senders = vec![
    //         CollectorDetail {
    //             telemetry_take: VecDeque::new(),
    //             ident: ActorIdentity { id: 0, name: "test_actor" },
    //         }
    //     ];
    //     let result = gather_node_details(&mut state, &dynamic_senders);
    //     assert!(result.is_none());
    // }

    #[test]
    fn test_is_all_empty_and_closed() {
        let dynamic_senders_vec = Arc::new(RwLock::new(vec![
            CollectorDetail {
                telemetry_take: VecDeque::new(),
                ident: ActorIdentity::new(0, "test_actor", None ),
            }
        ]));
        let result = is_all_empty_and_closed(dynamic_senders_vec.read());
        assert!(result);
    }

    #[test]
    fn test_send_structure_details() {
        let ident = ActorIdentity::new(0, "test_actor", None );
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
        let ident = ActorIdentity::new( 0,  "test_actor", None );
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

    // #[test]
    // fn test_confirm_shutdown() {
    //     let dynamic_senders_vec = Arc::new(RwLock::new(vec![
    //         CollectorDetail {
    //             telemetry_take: VecDeque::new(),
    //             ident: ActorIdentity { id: 0, name: "test_actor" },
    //         }
    //     ]));
    //     let mut locked_servers = nuclei::block_on(SteadyTxBundle::<DiagramData, 1>::default().lock());
    //     let result = is_all_empty_and_closed(dynamic_senders_vec.read());
    //     assert!(result);
    // }

    // #[test]
    // fn test_run() {
    //     let context = SteadyContext::default();
    //     let dynamic_senders_vec = Arc::new(RwLock::new(vec![
    //         CollectorDetail {
    //             telemetry_take: VecDeque::new(),
    //             ident: ActorIdentity { id: 0, name: "test_actor" },
    //         }
    //     ]));
    //     let optional_servers = SteadyTxBundle::<DiagramData, 1>::default();
    //     let result = block_on(run(context, dynamic_senders_vec, optional_servers));
    //     assert!(result.is_ok());
    // }
}
