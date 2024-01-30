
use std::ops::DerefMut;
use std::sync::Arc;
use futures::lock::Mutex;
use futures_timer::Delay;

#[allow(unused_imports)]
use log::*; //allow unused import

use time::Instant;
use crate::steady_state::*;
use crate::steady_state::Tx;
use crate::steady_state::monitor::{ChannelMetaData, RxTel};
use crate::steady_state::SteadyContext;

#[derive(Clone)]
pub enum DiagramData {
    //only allocates new space when new telemetry children are added.
    Node(u64, & 'static str, usize, Arc<Vec<Arc<ChannelMetaData>>>, Arc<Vec<Arc<ChannelMetaData>>>),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.
    Edge(u64, Vec<i128>, Vec<i128>),
        //, total_take, consumed_this_cycle, ma_consumed_per_second, in_flight_this_cycle, ma_inflight_per_second

}

struct RawDiagramState {
    sequence: u64,
    actor_count: usize,
    total_sent: Vec<i128>,
    total_take: Vec<i128>,
    future_take: Vec<i128>, //these are for the next frame since we do not have the matching sent yet
}

impl RawDiagramState {
    pub(crate) fn new() -> Self {
        RawDiagramState {
            sequence: 0,
            actor_count: 0,
            total_sent: Vec::new(),
            total_take: Vec::new(),
            future_take: Vec::new(),
        }
    }

}


pub(crate) async fn run(_context: SteadyContext
       , dynamic_senders_vec: Arc<Mutex<Vec< CollectorDetail >>>
       , optional_server: Option<Arc<Mutex<Tx<DiagramData>>>>
) -> Result<(),()> {

    let mut state = RawDiagramState::new();

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
                Some(gather_node_details(&mut state, &dynamic_senders))
            };

            //collect all volume data AFTER we update the node data first

            //we then consume all the send data available for this frame
            for x in dynamic_senders.iter() {
                x.telemetry_take.consume_send_into(&mut state.total_sent);
            }
            //now we can consume all the take data available for this frame
            //but some of this data may be for the next frame so we will
            //consume it into the future_take vec
            for x in dynamic_senders.iter() {
                x.telemetry_take.consume_take_into(&mut state.total_sent, &mut state.total_take, &mut state.future_take);
            }

            nodes
        }; //dropped senders guard so list can be updated with new nodes if needed

        if let Some(nodes) = nodes {//only happens when we have new nodes
            send_node_details(&optional_server, nodes).await;
        }
        send_edge_details(&optional_server, &mut state).await;

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


fn gather_node_details(state: &mut RawDiagramState, dynamic_senders: &&mut Vec<CollectorDetail>) -> Vec<DiagramData> {
//get the max channel ids for the new actors
    let (max_rx, max_tx): (usize, usize) = dynamic_senders.iter()
        .skip(state.actor_count).map(|x| {
        (x.telemetry_take.biggest_rx_id(), x.telemetry_take.biggest_tx_id())
    }).fold((0, 0), |mut acc, x| {
        if x.0 > acc.0 { acc.0 = x.0; }
        if x.1 > acc.1 { acc.1 = x.1; }
        acc
    });
    //either send or take that id will define the size of the vecs we need to grow
    let max_channels_len = std::cmp::max(max_rx, max_tx)+1; //add one to convert from index to length

    //grow our vecs as needed for the max ids found
    state.total_take.resize(max_channels_len, 0); //index to length so we add 1
    state.total_sent.resize(max_channels_len, 0); //index to length so we add 1
    state.future_take.resize(max_channels_len, 0);

    assert_eq!(state.total_take.len(), state.total_sent.len());

    let nodes: Vec<DiagramData> = dynamic_senders.iter()
            .skip(state.actor_count).map(|details| {
                    let tt = &details.telemetry_take;
                    DiagramData::Node(state.sequence
                                     , details.name
                                     , details.monitor_id
                                     , Arc::new(tt.rx_channel_id_vec())
                                     , Arc::new(tt.tx_channel_id_vec()))
        }).collect();
    state.actor_count = dynamic_senders.len();
    nodes
}

async fn send_node_details(consumer: &Option<Arc<Mutex<Tx<DiagramData>>>>
                           , nodes: Vec<DiagramData>
                           ) {

    if let Some(c) = consumer {
            let mut c_guard = c.lock().await;
            let c = c_guard.deref_mut();
            let mut to_send = nodes.into_iter();
            let _ = c.send_iter_until_full(&mut to_send);
            for send_me in to_send {
                let _ = c.send_async(send_me).await;
            }
    }
}

async fn send_edge_details(consumer: &Option<Arc<Mutex<Tx<DiagramData>>>>, state: &mut RawDiagramState) {
    //info!("compute send_edge_details {:?} {:?}",state.running_total_sent,state.running_total_take);
    assert_eq!(state.total_take.len(), state.total_sent.len());

    if let Some(c) = consumer {
        let send_me = DiagramData::Edge(state.sequence
                                        , state.total_take.clone()
                                        , state.total_sent.clone()
        );
        let mut c_guard = c.lock().await;
        let c = c_guard.deref_mut();
        let _ = c.send_async(send_me).await;
    }
}



pub struct CollectorDetail {
    pub(crate) telemetry_take: Box<dyn RxTel>,
    pub(crate) name: &'static str,
    pub(crate) monitor_id: usize
}


