
use std::ops::DerefMut;
use std::sync::Arc;
use futures::lock::Mutex;
use futures_timer::Delay;

use time::Instant;
use crate::steady::*;
use crate::steady::channel::SteadyTx;
use crate::steady::monitor::{ChannelMetaData, RxTel, SteadyMonitor};

#[derive(Clone)]
pub enum DiagramData {
    //only allocates new space when new telemetry children are added.
    Node(u64, & 'static str, usize, Arc<Vec<Arc<ChannelMetaData>>>, Arc<Vec<Arc<ChannelMetaData>>>),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.
    Edge(u64, Arc<Vec<i128>>, Arc<Vec<i128>>),
        //, total_take, consumed_this_cycle, ma_consumed_per_second, in_flight_this_cycle, ma_inflight_per_second

}

pub(crate) struct RawDiagramState {
    pub(crate) sequence: u64,
    pub(crate) actor_count: usize,
    pub(crate) running_total_sent: Vec<i128>,
    pub(crate) running_total_take: Vec<i128>,
}


pub(crate) async fn run(monitor: SteadyMonitor
       , dynamic_senders_vec: Arc<Mutex<Vec< CollectorDetail >>>
       , optional_server: Option<Arc<Mutex<SteadyTx<DiagramData>>>>
) -> Result<(),()> {

    let mut state = RawDiagramState {
        sequence: 0,
        actor_count: 0,
        running_total_take: Vec::new(), //running totals
        running_total_sent: Vec::new(), //running totals
    };

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
            for x in dynamic_senders.iter() {
                x.telemetry_take.consume_into(  &mut state.running_total_take
                                              , &mut state.running_total_sent);
            }
            nodes
        }; //dropped senders guard so list can be updated with new nodes if needed

        if let Some(nodes) = nodes {//only happens when we have new nodes
            send_node_details(&optional_server, &mut state, nodes).await;
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


fn gather_node_details(mut state: &mut RawDiagramState, dynamic_senders: &&mut Vec<CollectorDetail>) -> Vec<DiagramData> {
//get the max channel ids for the new actors
    let (max_rx, max_tx): (usize, usize) = dynamic_senders.iter()
        .skip(state.actor_count).map(|x| {
        (x.telemetry_take.biggest_rx_id(), x.telemetry_take.biggest_tx_id())
    }).fold((0, 0), |mut acc, x| {
        if x.0 > acc.0 { acc.0 = x.0; }
        if x.1 > acc.1 { acc.1 = x.1; }
        acc
    });
    //grow our vecs as needed for the max ids found
    state.running_total_take.resize(max_rx, 0);
    state.running_total_sent.resize(max_tx, 0);

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

async fn send_node_details(consumer: &Option<Arc<Mutex<SteadyTx<DiagramData>>>>
                           , state: &mut RawDiagramState
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

async fn send_edge_details(consumer: &Option<Arc<Mutex<SteadyTx<DiagramData>>>>, state: &mut RawDiagramState) {
    //info!("compute send_edge_details {:?} {:?}",state.running_total_sent,state.running_total_take);

    let send_me = DiagramData::Edge(state.sequence
                                    , Arc::new(state.running_total_take.clone())
                                    , Arc::new(state.running_total_sent.clone())
    );

    if let Some(c) = consumer {
        let mut c_guard = c.lock().await;
        let c = c_guard.deref_mut();
        let _ = c.send_async(send_me.clone()).await;
    }
}



pub struct CollectorDetail {
    pub(crate) telemetry_take: Box<dyn RxTel>,
    pub(crate) name: &'static str,
    pub(crate) monitor_id: usize
}


