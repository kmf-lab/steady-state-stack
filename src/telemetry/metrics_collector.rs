use std::collections::VecDeque;

use std::ops::DerefMut;
use std::sync::Arc;
use futures::lock::Mutex;
use futures_timer::Delay;

#[allow(unused_imports)]
use log::*; //allow unused import

use time::Instant;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
use crate::{config, SteadyContext, SteadyTx, Tx};

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



pub(crate) async fn run(_context: SteadyContext
       , dynamic_senders_vec: Arc<Mutex<Vec< CollectorDetail >>>
       , optional_server: Option<SteadyTx<DiagramData>>
) -> Result<(),()> {

    if let Some(c) = &optional_server {

        if config::SHOW_TELEMETRY_ON_TELEMETRY {
            //NOTE: this line makes this node monitored on the telemetry only when the server is monitored.
            _context.into_monitor([], [c]);

            //TODO: add monitoring of the gathering channels. We have support for replaced
            //    actors which rebuild the monitor channels on the fly matching the tx,rx count.
            //    Something similar needs to be done here. The channels can be rebuilt each time
            //    we discover the dynamic_senders_vec length has changed. This is a bit tricky

        }

    }


    let mut state = RawDiagramState::default();

    let mut last_instant = Instant::now();
    loop {
        let nodes:Option<Vec<DiagramData>> = {
            //we want to drop this as soon as we can
            //collect all the new data out of it release the guard then send the data
            let mut dynamic_senders_guard = dynamic_senders_vec.lock().await;
            let dynamic_senders = dynamic_senders_guard.deref_mut();

            //only done when we have new nodes, nodes can never be removed
            let nodes: Option<Vec<DiagramData>> = if dynamic_senders.len() == state.actor_count {
                None
            } else {
                Some(gather_node_details(&mut state, dynamic_senders))
            };

            //collect all volume data AFTER we update the node data first

            //we then consume all the send data available for this frame
            for x in dynamic_senders.iter_mut() {
                let has_data = x.telemetry_take[0].consume_send_into(&mut state.total_take_send
                                                                     ,&mut state.future_send);
                x.temp_barrier = has_data; //first so we just set it

                #[cfg(debug_assertions)]
                if x.telemetry_take.len()>1 { trace!("can see new telemetry channel")}

            }
            //now we can consume all the take data available for this frame
            //but some of this data may be for the next frame so we will
            //consume it into the future_take vec
            for x in dynamic_senders.iter_mut() {
                let has_data = x.telemetry_take[0].consume_take_into(&mut state.total_take_send
                                                                     , &mut state.future_take
                                                                     , &mut state.future_send
                                                                    );
                x.temp_barrier |= has_data; //if we have data here or in the previous
            }

            for x in dynamic_senders.iter_mut() {

                if let Some(act) = x.telemetry_take[0].consume_actor() {
                    let id = x.monitor_id;
                    if id<state.actor_status.len() {
                        state.actor_status[id] = act;
                    } else {
                        state.actor_status.resize(id+1, ActorStatus::default());
                        state.actor_status[id] = act;
                    }
                } else {
                    //we have no data here so if we had non on the others and we have a waiting
                    //new set of channels then we should remove the old one in favor of the new.
                    //this logic is key for a smooth hand off when an actor is restarted.
                    if !x.temp_barrier && x.telemetry_take.len().gt(&1) {
                        #[cfg(debug_assertions)]
                        trace!("swapping to new telemetry channels for {}", x.name);
                        x.telemetry_take.pop_front();
                    }
                }
            }
            nodes
        }; //dropped senders guard so list can be updated with new nodes if needed

        if let Some(nodes) = nodes {//only happens when we have new nodes
             send_structure_details(&optional_server, nodes).await;
        }
        send_data_details(&optional_server, &state).await;

        //NOTE: target 32ms updates for 30FPS, with a queue of 8 so writes must be no faster than 4ms
        //      we could double this speed if we have real animation needs but that is unlikely

        let duration_micros = last_instant.elapsed().whole_microseconds();
        let sleep_duration = (1000i128*config::TELEMETRY_PRODUCTION_RATE_MS as i128)-duration_micros;
        if sleep_duration.is_positive() { //only sleep as long as we need
            Delay::new(std::time::Duration::from_micros(sleep_duration as u64)).await;
        }
        state.sequence += 1; //increment next frame
        last_instant = Instant::now();

    }
}

// Async function to append data to a file


fn gather_node_details(state: &mut RawDiagramState, dynamic_senders: &mut [CollectorDetail]) -> Vec<DiagramData> {
//get the max channel ids for the new actors
    let (max_rx, max_tx): (usize, usize) = dynamic_senders.iter()
        .skip(state.actor_count).map(|x| {
        (x.telemetry_take[0].biggest_rx_id(), x.telemetry_take[0].biggest_tx_id())
    }).fold((0, 0), |mut acc, x| {
        if x.0 > acc.0 { acc.0 = x.0; }
        if x.1 > acc.1 { acc.1 = x.1; }
        acc
    });
    //either send or take that id will define the size of the vecs we need to grow
    let max_channels_len = std::cmp::max(max_rx, max_tx)+1; //add one to convert from index to length

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

                    DiagramData::NodeDef(state.sequence
                                         , dd
                    )
        }).collect();
    state.actor_count = dynamic_senders.len();
    nodes
}

async fn send_structure_details(consumer: &Option<Arc<Mutex<Tx<DiagramData>>>>
                                , nodes: Vec<DiagramData>
                           ) {

    if let Some(c) = consumer {
            let mut c_guard = c.lock().await;
            let c = c_guard.deref_mut();

           // info!("sending count of {} ", nodes.len());
            let mut to_send = nodes.into_iter();
            let _count = c.send_iter_until_full(&mut to_send);
          //  info!("bulk count {} remaining {} ", _count, to_send.len());
            for send_me in to_send {
                let _ = c.send_async(send_me).await;
            }
    }
}

async fn send_data_details(consumer: &Option<Arc<Mutex<Tx<DiagramData>>>>, state: &RawDiagramState) {
    //info!("compute send_edge_details {:?} {:?}",state.running_total_sent,state.running_total_take);

    if let Some(consumer) = consumer {

        let mut c_guard = consumer.lock().await;
        let consumer = c_guard.deref_mut();

        let _ = consumer.send_async(DiagramData::NodeProcessData(state.sequence
                                                                 , state.actor_status.clone().into_boxed_slice()
                )).await;


        let _ = consumer.send_async(DiagramData::ChannelVolumeData(state.sequence
                                                                 ,  state.total_take_send.clone().into_boxed_slice()

                )).await;

    }
}



pub struct CollectorDetail {
    pub(crate) telemetry_take: VecDeque<Box<dyn RxTel>>,
    pub(crate) temp_barrier: bool,
    pub(crate) name: &'static str,
    pub(crate) monitor_id: usize,

    //pub(crate) telemetry_send: VecDeque<Box<dyn TxTel>>,
    //TODO: add the send end here so we can determine dead locks?

}


