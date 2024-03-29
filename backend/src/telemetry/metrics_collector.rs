use std::collections::VecDeque;
use std::error::Error;
use std::ops::{ Deref, DerefMut};
use std::pin::Pin;
use std::sync::{Arc, RwLock};

#[allow(unused_imports)]
use log::*; //allow unused import

use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
use crate::{config, RxDef, SteadyContext, SteadyTx, Tx, util};

use futures::future::*;
use futures_util::lock::{MutexGuard, MutexLockFuture};

use crate::graph_liveliness::ActorIdentity;
use crate::telemetry::{metrics_collector, metrics_server};

pub const NAME: &str = "metrics_collector";




#[derive(Clone, Debug)]
pub enum DiagramData {
    #[allow(clippy::type_complexity)]
    //uses heap but only when a new actor is added so we can define it and its channels
    NodeDef(u64    ,Box<(Arc<ActorMetaData>
                        ,Box<[Arc<ChannelMetaData>]>
                        ,Box<[Arc<ChannelMetaData>]>)>
    ),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.

    ChannelVolumeData(u64, Box<[(i128,i128)]>),
    NodeProcessData(u64, Box<[ActorStatus]>),

}

#[derive(Default)]
struct RawDiagramState {
    sequence: u64,
    actor_count: usize,
    actor_status: Vec<ActorStatus>,
    total_take_send: Vec<(i128,i128)>,
    future_take: Vec<i128>, //these are for the next frame since we do not have the matching sent yet.
    future_send: Vec<i128>, //these are for the next frame if we ended up with too many items.
}


fn lock_if_some<'a, T: Send + 'a + Sync>(opt_lock: &'a Option<SteadyTx<T>>)
    -> Pin<Box<dyn Future<Output = Option<MutexGuard<'a, Tx<T>>>> + Send + 'a>> {
    Box::pin(async move {
        match opt_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        }
    })
}

//TODO: the collector is starting up after the others, this is not good. We need to start it first
//      the init should be done at graph construction.
//      the start should be after we build teh graph.
//     shutdown needs to be finished.

pub(crate) async fn run(context: SteadyContext
       , dynamic_senders_vec: Arc<RwLock<Vec< CollectorDetail >>>
       , optional_server: Option<SteadyTx<DiagramData>>
) -> Result<(),Box<dyn Error>> {

    let ident = context.identity();

    //todo: rather than options we may want to use a three way enum for this?
    let mut optional_monitor:Option<_> = None;
    let mut optional_context:Option<_> = None;

    if config::SHOW_TELEMETRY_ON_TELEMETRY {
        optional_monitor = if let Some(c) = &optional_server {
            optional_context = None;
            Some(context.into_monitor([], [c]))
        } else {
            optional_context = Some(context);
            None
        };
    } else {
        optional_monitor = None;
        optional_context = Some(context);
    };

    let mut state = RawDiagramState::default();

    let mut scan: Option<Vec<Box<dyn RxDef>>> = None;


let mut svr = lock_if_some(&optional_server).await;

loop {
    //we do this here instead at the while because monitor is optional
        if let Some(ref mut monitor) = optional_monitor {
            monitor.relay_stats_smartly().await;
            //TODO: keep running if dynamic_senders_vec are no empty and still open
            //      this requires us to add the closed logic on the other end.
            if !monitor.is_running(&mut || svr.iter_mut().all(|mut s| s.mark_closed()) ) {
                break;
            }
        } else {
            if let Some(ref mut context) = optional_context {
                if !context.is_running(&mut || svr.iter_mut().all(|mut s| s.mark_closed()) ) {
                    break;
                }
            }
        }

        //we wait here without holding the large lock
        if let Some(ref scan) = scan {
            let futures = collect_futures_for_one_frame(scan);
            let (_result, _index, _remaining) = select_all(futures).await;
        } else {
            scan = gather_scan_rx(&dynamic_senders_vec);
            if let Some(ref scan) = scan {
                let futures = collect_futures_for_one_frame(scan);
                let (_result, _index, _remaining) = select_all(futures).await;
            } else {
                util::async_yield_now().await;
            }
        }

        let (nodes, to_pop) = {
            //we want to drop this as soon as we can
            //collect all the new data out of it release the guard then send the data
            //warn!("blocking on read");
            match dynamic_senders_vec.read() {
                Ok(guard) => {
                    //warn!("ok on read lock");
                    let dynamic_senders = guard.deref();

                    let structure_unchanged = dynamic_senders.len() == state.actor_count;
                    //only done when we have new nodes, nodes can never be removed
                    if structure_unchanged {
                        (None, collect_channel_data(&mut state, dynamic_senders))
                    } else {
                        scan = None;
                        if let Some(n) = gather_node_details(&mut state, dynamic_senders) {
                            (Some(n), collect_channel_data(&mut state, dynamic_senders))
                        } else {
                            //we have a channel that is not connected
                            //do not send data yet
                            (None, Vec::new())
                        }
                    }
                },
                Err(_) => { //unable to get read lock
                    //warn!("err on read lock");
                    (None, Vec::new())
                }
            }
        }; //dropped senders guard so list can be updated with new nodes if needed

        if !to_pop.is_empty() {
            //NOTE: this is the only place where collector holds write lock
            warn!("block on write lock");
            match dynamic_senders_vec.write() { //this is blocking
                Ok(mut guard) => {
                    //warn!("ok from write lock");
                    let dynamic_senders = guard.deref_mut();
                    //to_pop is assumed to be a very short list in almost all cases
                    //it is the specific actors which restarted and are ready for channel swap
                    to_pop.iter().for_each(|ident| {
                        #[cfg(debug_assertions)]
                        trace!("swapping to new telemetry channels for {:?}", ident);
                        dynamic_senders.iter_mut()
                            .for_each(|f| {
                                if f.ident == *ident {
                                    f.telemetry_take.pop_front();
                                }
                            });
                    });
                }
                Err(_) => {
                    //warn!("err from write lock");
                    //unable to get write lock
                    //we will try again next time
                    continue;
                }
            }
        }
        if let Some(nodes) = nodes {//only happens when we have new nodes
             send_structure_details(ident, &mut svr, nodes).await;
        }
        send_data_details(ident, &mut svr, &state).await;

    state.sequence += 1; //increment next frame
    }
    Ok(())
}

fn collect_futures_for_one_frame(scan: &[Box<dyn RxDef>]) -> Vec<BoxFuture<()>> {

    //we do not need to check every channel every time
    let skip_interval = if scan.len() > 61 {
        scan.len() / 31
    } else {
        2
    };

    scan.iter()
        .enumerate()
        .filter(|(index, _)| 0 == index % skip_interval )
        .map(|(_, item)| {
            item.wait_avail_units(config::CONSUMED_MESSAGES_BY_COLLECTOR)
        })
        .collect()
}

fn gather_scan_rx(dynamic_senders_vec: &Arc<RwLock<Vec<CollectorDetail>>>) -> Option<Vec<Box<dyn RxDef>>> {
    if let Ok(guard) = dynamic_senders_vec.read() {
        let dynamic_senders = guard.deref();
        let v: Vec<Box<dyn RxDef>> = dynamic_senders.iter()
            .filter(|f|  (f.ident.name != metrics_collector::NAME)
                      && (f.ident.name != metrics_server::NAME))
            .flat_map(|f| f.telemetry_take.iter().filter_map(|g| g.actor_rx())  )
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

fn collect_channel_data(state: &mut RawDiagramState, dynamic_senders: &[CollectorDetail]) -> Vec<ActorIdentity> {
    //collect all volume data AFTER we update the node data first
    //we still hold the dynamic senders lock so nothing new could be
    //added while we consume all the send and take data

    //we then consume all the send data available for this frame
    let working:Vec<(bool, &CollectorDetail )> = dynamic_senders.iter().map(|f| {
        let has_data = f.telemetry_take[0].consume_send_into( &mut state.total_take_send
                                                            , &mut state.future_send);
        #[cfg(debug_assertions)]
        if f.telemetry_take.len() > 1 { trace!("can see new telemetry channel") }
        (has_data,f)
    }).collect();

    //now we can consume all the take data available for this frame
    //but some of this data may be for the next frame so we will
    //consume it into the future_take vec
    let working:Vec<(bool, &CollectorDetail )> = working.iter().map(|(has_data_in, f)| {
        let has_data = f.telemetry_take[0].consume_take_into(&mut state.total_take_send
                                                             , &mut state.future_take
                                                             , &mut state.future_send
        );
        (has_data || *has_data_in,*f)
    }).collect();

    let to_pop:Vec<ActorIdentity> = working.iter().filter(|(has_data_in, f)| {
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
            //we have no data here so if we had non on the others and we have a waiting
            //new set of channels then we should remove the old one in favor of the new.
            //this logic is key for a smooth hand off when an actor is restarted.
            !has_data_in && f.telemetry_take.len().gt(&1)
        }
    }).map( |(_,c)| c.ident).collect();
    to_pop


}

// Async function to append data to a file


fn gather_node_details(state: &mut RawDiagramState, dynamic_senders: &[CollectorDetail]) -> Option<Vec<DiagramData>> {

    //error!("gather node structure details");
 //we must confirm every channel has a consumer and a producer, if not we must return
 //also we must find the max channel id to ensure our vec length is correct
 let mut matches:Vec<u8> = Vec::new();
 //collect each matching field
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


  if !matches.iter().all(|x| *x==3 || *x==0) {

          // this happens by design on startup at times.
      // TODO: if it happens long term we should report to the user
      //  because they have one side and not the other of a channel
          matches.iter().filter(|x| !*x==3 && !*x==0).for_each(|x| {
                trace!("can not get structure due to some bad value: {:?}",x);
          });

        return None;
  }
  //error!("all looks good");
  let max_channels_len = matches.len();


    //grow our vecs as needed for the max ids found
    state.total_take_send.resize(max_channels_len, (0,0)); //index to length so we add 1
    state.future_take.resize(max_channels_len, 0);
    state.future_send.resize(max_channels_len, 0);

    let nodes: Vec<DiagramData> = dynamic_senders.iter()
            .skip(state.actor_count).map(|details| {
                    let tt = &details.telemetry_take[0];
                    let dd =Box::new((tt.actor_metadata().clone()
                               ,tt.rx_channel_id_vec().clone().into_boxed_slice()
                               ,tt.tx_channel_id_vec().clone().into_boxed_slice()
                    ));
                    DiagramData::NodeDef(state.sequence, dd)
        }).collect();
    state.actor_count = dynamic_senders.len();
    Some(nodes)
}

async fn send_structure_details(ident: ActorIdentity, consumer: &mut Option<MutexGuard<'_, Tx<DiagramData>>>
                                , nodes: Vec<DiagramData>
                           ) {

    if let Some(ref mut c) = consumer {
           // info!("sending count of {} ", nodes.len());
            let mut to_send = nodes.into_iter();
            let _count = c.send_iter_until_full(&mut to_send);
          //  info!("bulk count {} remaining {} ", _count, to_send.len());
            for send_me in to_send {
                //TODO: should be true for release code
                let _ = c.send_async(ident, send_me, false).await;
            }
    }
}

async fn send_data_details(ident: ActorIdentity, consumer: &mut Option<MutexGuard<'_, Tx<DiagramData>>>, state: &RawDiagramState) {
    //info!("compute send_edge_details {:?} {:?}",state.running_total_sent,state.running_total_take);

    if let Some(ref mut consumer) = consumer {

        //TODO: should be false for debug and true for release.
        let _ = consumer.send_async(ident, DiagramData::NodeProcessData(state.sequence
                                                                 , state.actor_status.clone().into_boxed_slice()
                ),false).await;


        let _ = consumer.send_async(ident, DiagramData::ChannelVolumeData(state.sequence
                                                                 ,  state.total_take_send.clone().into_boxed_slice()

                ),false).await;

    }
}


pub struct CollectorDetail {
    pub(crate) telemetry_take: VecDeque<Box<dyn RxTel>>,
    pub(crate) ident: ActorIdentity,

    //pub(crate) telemetry_send: VecDeque<Box<dyn TxTel>>,
    //TODO: add the send end here so we can determine dead locks?

}


