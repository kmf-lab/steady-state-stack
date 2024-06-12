use std::collections::VecDeque;
use std::error::Error;
use std::ops::{ Deref, DerefMut};
use std::pin::Pin;
use std::sync::{Arc, LockResult};
use std::sync::{RwLock, RwLockReadGuard};
use std::time::Duration;

#[allow(unused_imports)]
use log::*; //allow unused import

use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
use crate::{TxDef, RxDef, config, into_monitor, SendSaturation, SteadyContext, SteadyTx, Tx, yield_now, SteadyTxBundle, SteadyTxBundleTrait, TxBundleTrait};


use futures::future::*;
use futures_timer::Delay;
use futures_util::{ select, StreamExt};
use futures_util::lock::MutexGuard;
use futures_util::stream::FuturesUnordered;

use crate::graph_liveliness::ActorIdentity;
use crate::telemetry::{metrics_collector};

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

#[allow(clippy::type_complexity)]
fn lock_if_some<'a, T: Send + 'a + Sync>(opt_lock: &'a Option<SteadyTx<T>>)
    -> Pin<Box<dyn Future<Output = Option<MutexGuard<'a, Tx<T>>>> + Send + 'a>> {
    Box::pin(async move {
        match opt_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        }
    })
}

pub(crate) async fn run<const GIRTH: usize>(context: SteadyContext
       , dynamic_senders_vec: Arc<RwLock<Vec< CollectorDetail >>>
       , optional_servers: SteadyTxBundle<DiagramData, GIRTH>
) -> Result<(),Box<dyn Error>> {
    //trace!("running {:?} {:?}",context.id(),context.name());

    let ident = context.identity();

    let ctrl = context;
    #[cfg(all(feature = "telemetry_on_telemetry", any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
    let mut ctrl = {
                     into_monitor!(ctrl,[], optional_servers)
                   };

    let mut state = RawDiagramState::default();
    let mut all_actors_to_scan: Option<Vec<Box<dyn RxDef>>> = None;
    let mut timelords: Option<Vec<usize>> = None;

    let mut locked_servers = optional_servers.lock().await;

    let mut rebuild_scan_requested:bool = false;
    let mut is_shutting_down = false;
loop {

    let confirm_shutdown = &mut ||
        {
            is_shutting_down = true;
            is_all_empty_and_closed(dynamic_senders_vec.read())
                && locked_servers.mark_closed()
        };


        #[cfg(feature = "telemetry_on_telemetry")]
        ctrl.relay_stats_smartly();

        if !ctrl.is_running(confirm_shutdown) {
            break;
        }
        let instance_id = ctrl.instance_id;



         if all_actors_to_scan.is_none() || rebuild_scan_requested {
             all_actors_to_scan = gather_valid_actor_telemetry_to_scan(instance_id, &dynamic_senders_vec);
             timelords = None;
             rebuild_scan_requested = false;
         }

        if let Some(ref scan) = all_actors_to_scan {
            let mut futures_unordered = FuturesUnordered::new();
            if let Some(ref timelords) = timelords {
                timelords.iter().for_each(|i| scan.get(*i).iter().for_each(|f| futures_unordered.push(f.wait_avail_units(config::CONSUMED_MESSAGES_BY_COLLECTOR))));
                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    if full_frame_of_data {
                        break;
                    } else if id.is_none() {
                            //no data and no id, some actor probably restarted or we are having lock issues for both cases we must
                            //rebuild our scan list for the next iteration
                            rebuild_scan_requested = true;
                    }

                };
            } else {

                //wait for time signal AND build a new timelord collection for faster checks next time
                scan.iter().for_each(|f| futures_unordered.push(f.wait_avail_units(config::CONSUMED_MESSAGES_BY_COLLECTOR)));
                //collect N or whatever we can find for very small graphs
                let count = 5; //max actors we monitor to find the first which a a full frame of data.
                //since we are building the cache list now we will be forced to wait for as many as 7 (rare case)
                //these are the first n responders which are best to keep reviewing vs all or some random set.
                let mut timelord_collector = Vec::new();
                while let Some((full_frame_of_data, id)) = full_frame_or_timeout(&mut futures_unordered, ctrl.frame_rate_ms).await {
                    //we need to find just one with a full frame of data
                    if full_frame_of_data {
                        timelord_collector.push(id.expect("internal error, every true has an index"));
                        if timelord_collector.len() >= count {
                            break;
                        }
                    }
                }
                timelords = if timelord_collector.is_empty() {None} else {Some(timelord_collector)};
            }

        } else {
            yield_now().await;
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
                        all_actors_to_scan = None;
                        rebuild_scan_requested = true;
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
            send_structure_details(ident, &mut locked_servers, nodes).await;
        }
        let warn = !is_shutting_down && config::TELEMETRY_SERVER;
        send_data_details(ident, &mut locked_servers, &state, warn).await;

        state.sequence += 1; //increment next frame
    }
    Ok(())
}

async fn full_frame_or_timeout(futures_unordered: &mut FuturesUnordered<BoxFuture<'_, (bool,Option<usize>)>>, telemetry_rate_ms:u64) -> Option<(bool,Option<usize>)> {
    select! {
        x = futures_unordered.next().fuse() => {
            //we need to find just one with a full frame of data
            x
        },
        _ = Delay::new(Duration::from_millis( (telemetry_rate_ms>>1) as u64 )).fuse() => {
            None
        }
    }
}

fn is_all_empty_and_closed(m_channels: LockResult<RwLockReadGuard<'_, Vec<CollectorDetail>>>) -> bool {
   match m_channels {
         Ok(channels) => {
             for c in channels.iter() {
                    for t in c.telemetry_take.iter() {
                            if !t.is_empty_and_closed() {
                                return false;  // Early exit if any telemetry is not empty and closed
                            }
                        }
             }
             true
         },
         Err(_) => false
   }
}



fn gather_valid_actor_telemetry_to_scan(version:u32, dynamic_senders_vec: &Arc<RwLock<Vec<CollectorDetail>>>) -> Option<Vec<Box<dyn RxDef>>> {
    if let Ok(guard) = dynamic_senders_vec.read() {
        let dynamic_senders = guard.deref();
        let v: Vec<Box<dyn RxDef>> = dynamic_senders.iter()
            //do not consider collector since collector uses this same method to scan the others as a trigger
            .filter(|f|  f.ident.name != metrics_collector::NAME)
            .flat_map(|f| f.telemetry_take.iter().filter_map(|g| g.actor_rx(version))  )
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
        return None;
  }

    let max_channels_len = matches.len();


    //grow our vecs as needed for the max ids found
    state.total_take_send.resize(max_channels_len, (0,0)); //index to length so we add 1
    state.future_take.resize(max_channels_len, 0);
    state.future_send.resize(max_channels_len, 0);


    let nodes: Vec<DiagramData> = dynamic_senders.iter()
            .skip(state.actor_count).map(|details| {
                    let tt = &details.telemetry_take[0];
                    let metadata = tt.actor_metadata();
                    let rx_vec = tt.rx_channel_id_vec();
                    let tx_vec = tt.tx_channel_id_vec();

        // error!("new node {:?} {:?} in {:?} out {:?}",metadata.id,metadata.name,rx_vec.len(),tx_vec.len());
        // rx_vec.iter().for_each(|x| error!("          channel in {:?} to {:?}",x.id,metadata.id));
        // tx_vec.iter().for_each(|x| error!("         channel out {:?} from {:?}",x.id,metadata.id));


        //TODO:when sending these NodDefs they are broken and changed in flight??
                    let dd =Box::new((metadata.clone()
                                       ,rx_vec.into_boxed_slice()
                                       ,tx_vec.into_boxed_slice()  )
                    );



        DiagramData::NodeDef(state.sequence, dd)
        }).collect();
    state.actor_count = dynamic_senders.len();
    Some(nodes)
}

async fn send_structure_details(ident: ActorIdentity, consumer_vec: &mut Vec<MutexGuard<'_,Tx<DiagramData>>>
                                , nodes: Vec<DiagramData>
                           ) {

    for consumer in consumer_vec.iter_mut() {
        //trace!("sending count of {} ", nodes.len());
        let to_send = nodes.clone().into_iter();
        //let count = c.send_iter_until_full(&mut to_send);
        //trace!("bulk count {} remaining {} ", count, to_send.len());
        for send_me in to_send {
            if let Some(DiagramData::NodeDef(_seq, defs)) = Some(send_me.clone()) {
                let (_actor, _channels_in, _channels_out) = *defs;

                //confirm the data we get from the collector
                //info!("new node {:?} {:?} in {:?} out {:?}",actor.id,actor.name,channels_in.len(),channels_out.len());
                //channels_in.iter().for_each(|x| info!("          channel in {:?} to {:?}",x.id,actor.id));
                //channels_out.iter().for_each(|x| info!("         channel out {:?} from {:?}",x.id,actor.id));
            }
            match consumer.send_async(ident, send_me, SendSaturation::IgnoreAndWait).await {
                Ok(_) => {},
                Err(e) => {
                    error!("error sending node data {:?}",e);
                }

            }
        }
    }
}

async fn send_data_details(ident: ActorIdentity, consumer_vec: &mut Vec<MutexGuard<'_,Tx<DiagramData>>>, state: &RawDiagramState, warn:bool) {
    for consumer in consumer_vec.iter_mut() {
        //NOTE: if our outgoing channel is full we will NOT send this frame. our Vec data is rolling
        //      forever so no data is lost instead we just lost the specifics of that frame. This is not
        //      desirable and can be avoided if the consumer is fast and/or the channel is long enough.
        if consumer.vacant_units().ge(&2) {

            //all the actors in the graph with their stats for this frame
            match consumer.send_async(ident, DiagramData::NodeProcessData(state.sequence
                                             , state.actor_status.clone().into_boxed_slice()
                                        ), SendSaturation::IgnoreInRelease).await {
                Ok(_) => {}
                Err(_) => {}
            }

            //all the channels in the graph with their stats for this frame
            match consumer.send_async(ident, DiagramData::ChannelVolumeData(state.sequence
                                             , state.total_take_send.clone().into_boxed_slice()
                                          ), SendSaturation::IgnoreInRelease).await {
                Ok(_) => {}
                Err(_) => {}
            }

        } else if warn { //do not warn when shutting down or running tests.
            warn!("{:?} is accumulating frames since consumer is not keeping up, perhaps capacity of {:?} may need to be increased.",ident, consumer.capacity());
        }

    }
}


pub struct CollectorDetail {
    pub(crate) telemetry_take: VecDeque<Box<dyn RxTel>>,
    pub(crate) ident: ActorIdentity,

}


