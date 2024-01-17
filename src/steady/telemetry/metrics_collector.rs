use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;
use async_std::fs::File;
use async_std::io::WriteExt;
use async_std::path::Path;
use ntex::util::BytesMut;
use crate::steady::*;
//TODO: write to disk if history/logging feature is on
//TODO: the u128 seq and count types should be moved out as type aliases based on use case need.

#[derive(Clone, Debug)]
pub enum DiagramData {
    //only allocates new space when new telemetry children are added.
    Node(u128, & 'static str, usize, Arc<Vec<(usize, Vec<&'static str>)>>, Arc<Vec<(usize, Vec<&'static str>)>>),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.
    Edge(u128, Arc<Vec<u128>>, Arc<Vec<u128>>, Arc<Vec<u128>>, Arc<Vec<u128>>, Arc<Vec<u128>>),
        //, total_take, consumed_this_cycle, ma_consumed_per_second, in_flight_this_cycle, ma_inflight_per_second
}

struct InternalState {
    total_sent: Vec<u128>,
    total_take: Vec<u128>,
    previous_total_take: Vec<u128>,
    actor_count: usize,
    sequence: u128,
    ma_index: usize,
    ma_consumed_buckets:Vec<[u128; MA_ADJUSTED_BUCKETS]>,
    ma_consumed_runner: Vec<u128>,
    ma_inflight_buckets:Vec<[u128; MA_ADJUSTED_BUCKETS]>,
    ma_inflight_runner: Vec<u128>,
}



macro_rules! count_bits {
    ($num:expr) => {{
        let mut num = $num;
        let mut bits = 0;
        while num > 0 {
            num >>= 1;
            bits += 1;
        }
        bits
    }};
}

//we make the buckets for MA a power of 2 for easy rotate math
const MA_MIN_BUCKETS: usize = 1000/ config::TELEMETRY_PRODUCTION_RATE_MS;
const MA_BITS: usize = count_bits!(MA_MIN_BUCKETS);
const MA_ADJUSTED_BUCKETS:usize = 1 << MA_BITS;
const MA_BITMASK: usize = MA_ADJUSTED_BUCKETS - 1;

const DO_STORE_HISTORY:bool = config::TELEMETRY_HISTORY;


pub(crate) async fn run(monitor: SteadyMonitor
     , dynamic_senders_vec: Arc<Mutex<Vec< CollectorDetail >>>
     , fixed_consumers: Vec<Arc<Mutex<SteadyTx<DiagramData>>>>
) -> Result<(),()> {

    let mut state = InternalState {
                total_take: Vec::new(), //running totals
                total_sent: Vec::new(), //running totals
                previous_total_take: Vec::new(), //for measuring cycle consumed
                actor_count: 0,
                sequence: 0,
                ma_index: 0,
                ma_consumed_buckets: Vec::new(),
                ma_consumed_runner: Vec::new(),
                ma_inflight_buckets: Vec::new(),
                ma_inflight_runner: Vec::new(),
    };

    loop {

        //TODO: if we have zero consumers we should be dropping messages.
        //    also warn that we send data but not using it. (in the init)

        //
        //the monitored channels per telemetry of set once on startup but we can add more actors if needed dynamically
        //
        let seq = state.sequence.clone(); //every frame has a unique sequence number
        state.sequence += 1;

        let mut dynamic_senders_guard = dynamic_senders_vec.lock().await; //we want to drop this as soon as we can
        let dynamic_senders = dynamic_senders_guard.deref_mut();

        if dynamic_senders.len() > state.actor_count {
            update_structure(&fixed_consumers, &mut state, seq, &dynamic_senders).await;
        }
        //collect all volume data
        for x in dynamic_senders.iter() {
            x.telemetry_take.consume_into(&mut state.total_take, &mut state.total_sent);
        }
        ////////////////////////////////////////////////
        //always compute our moving average values
        ////////////////////////////////////////////////
        //NOTE: at this point we must compute rates so its done just once

        //This is how many are in the channel now
        let total_in_flight_this_cycle:Vec<u128> = state.total_sent.iter()
                                                        .zip(state.total_take.iter())
                                                        .map(|(s,t)| s.saturating_sub(*t)).collect();
        //This is how many messages we have consumed this cycle
        let total_consumed_this_cycle:Vec<u128> = state.total_take.iter()
                                                       .zip(state.previous_total_take.iter())
                                                       .map(|(t,p)| t.saturating_sub(*p)).collect();

        //message per second is based on those TAKEN/CONSUMED from channels
        state.ma_consumed_buckets.iter_mut()
                .enumerate()
                .for_each(|(idx,buf)| {
                    state.ma_consumed_runner[idx] += buf[state.ma_index];
                    state.ma_consumed_runner[idx] = state.ma_consumed_runner[idx].saturating_sub(buf[(state.ma_index + MA_ADJUSTED_BUCKETS - MA_MIN_BUCKETS ) & MA_BITMASK]);
                    buf[state.ma_index] = total_consumed_this_cycle[idx];
                    //we do not divide because we are getting the one second total
                });

        //ma for the inflight
        let ma_inflight_runner:Vec<u128> = state.ma_inflight_buckets.iter_mut()
            .enumerate()
            .map(|(idx,buf)| {
                state.ma_inflight_runner[idx] += buf[state.ma_index];
                state.ma_inflight_runner[idx] = state.ma_inflight_runner[idx].saturating_sub(buf[(state.ma_index + MA_ADJUSTED_BUCKETS - MA_MIN_BUCKETS ) & MA_BITMASK]);
                buf[state.ma_index] = total_in_flight_this_cycle[idx];
                //we do divide to find the average held in channel over the second
                state.ma_inflight_runner[idx]/MA_MIN_BUCKETS as u128
            }).collect();

        //we may drop a seq no if backed up
        //warn!("building sequence {} in metrics_collector with {} consumers",seq,fixed_consumers.len());

        let all_room = true;//confirm_all_consumers_have_room(&fixed_consumers).await;   //TODO: can we make a better loop?
        if all_room && !fixed_consumers.is_empty() {
            //info!("send diagram edge data for seq {}",seq);
            /////////////// NOTE: if this consumes too much CPU we may need to re-evaluate
            let total_take              = Arc::new(state.total_take.clone());
            let consumed_this_cycle     = Arc::new(total_consumed_this_cycle.clone());
            let ma_consumed_per_second  = Arc::new(state.ma_consumed_runner.clone());
            let in_flight_this_cycle    = Arc::new(total_in_flight_this_cycle);
            let ma_inflight_per_second  = Arc::new(ma_inflight_runner.clone());

            let send_me = DiagramData::Edge(seq
                                            , total_take
                                            , consumed_this_cycle
                                            , ma_consumed_per_second
                                            , in_flight_this_cycle
                                            , ma_inflight_per_second );

            for c in fixed_consumers.iter() {
                let mut c_guard = c.lock().await;
                let c = c_guard.deref_mut();
                let _ = c.send_async(send_me.clone()).await;
            }
        }

        //keep current take for next time around
        state.previous_total_take.clear();
        state.total_take.iter().for_each(|f| state.previous_total_take.push(*f) );

        //next index
        state.ma_index = (1+state.ma_index) & MA_BITMASK;

        //NOTE: target 32ms updates for 30FPS, with a queue of 8 so writes must be no faster than 4ms
        //      we could double this speed if we have real animation needs but that is unlikely


//TODO: reduce timing based on work done...


        Delay::new(Duration::from_millis(config::TELEMETRY_PRODUCTION_RATE_MS as u64)).await;
        if false {
            break Ok(());
        }
    }
}

async fn update_structure(fixed_consumers: &Vec<Arc<Mutex<SteadyTx<DiagramData>>>>, mut state: &mut InternalState, seq: u128, dynamic_senders: &&mut Vec<CollectorDetail>) {
//get the max channel ids for the new actors
    let (max_rx, max_tx): (usize, usize) = dynamic_senders.iter().skip(state.actor_count).map(|x| {
        (x.telemetry_take.biggest_rx_id(), x.telemetry_take.biggest_tx_id())
    }).fold((0, 0), |mut acc, x| {
        if x.0 > acc.0 { acc.0 = x.0; }
        if x.1 > acc.1 { acc.1 = x.1; }
        acc
    });
    //grow our vecs as needed for the max ids found
    state.total_take.resize(max_rx, 0);
    state.total_sent.resize(max_tx, 0);
    state.previous_total_take.resize(max_rx, 0);
    state.ma_consumed_buckets.resize(max_rx, [0; MA_ADJUSTED_BUCKETS]);
    state.ma_inflight_buckets.resize(max_rx, [0; MA_ADJUSTED_BUCKETS]);
    state.ma_consumed_runner.resize(max_rx, 0);
    state.ma_inflight_runner.resize(max_rx, 0);

    //NOTE: sending data to consumers of the telemetry only happens once every 32ms or so
    //      it should be a s light weight as possible but not as critical as the collector
    //

    if fixed_consumers.len()>0 {
        //send new telemetry definitions to our listeners
        let skip_amount = state.actor_count;
        let total_length = dynamic_senders.len();
        //info!("send node struct for {} consumers",fixed_consumers.len());
        for i in skip_amount..total_length {
            let details = &dynamic_senders[i];
            let tt = &details.telemetry_take;
            let rx_channel_id: Arc<Vec<(usize, Vec<&'static str>)>> = Arc::new(tt.rx_channel_id_vec());
            let tx_channel_id: Arc<Vec<(usize, Vec<&'static str>)>> = Arc::new(tt.tx_channel_id_vec());

            let send_me = DiagramData::Node(seq
                                            , details.name
                                            , details.monitor_id
                                            , rx_channel_id.clone()
                                            , tx_channel_id.clone());

            //TODO: we may want to check first that all have room??
            for c in fixed_consumers.iter() {
                let mut c_guard = c.lock().await;
                let c = c_guard.deref_mut();
                //let _ = monitor.send_async(c, send_me.clone()).await;
                let _ = c.send_async(send_me.clone()).await;
            }
        };
    }


}

async fn confirm_all_consumers_have_room(fixed_consumers: &Vec<Arc<Mutex<SteadyTx<DiagramData>>>>) -> bool {
    let mut ok = true;
    for c in fixed_consumers.iter() {
        let mut c_guard = c.lock().await;
        let c = c_guard.deref_mut();
        if c.is_full() {
            ok = false;
            break;
        }
    }
    ok
}

pub struct CollectorDetail {
    pub(crate) telemetry_take: Box<dyn RxTel>,
    pub(crate) name: &'static str,
    pub(crate) monitor_id: usize
}


pub trait RxTel : Send + Sync {


    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)>;
    fn rx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)>;

    fn consume_into(&self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);

        //NOTE: we will do one dyn call per node every 32ms or so to build the image
    //      we only have 1 impl assuming the compiler will inline this if possible
    // TODO: in the future we could rewrite this to return a future that can be pinned and boxed
 //   fn consume_into(& mut self, take_target: &mut Vec<u128>, send_target: &mut Vec<u128>);
    fn biggest_tx_id(&self) -> usize;
    fn biggest_rx_id(&self) -> usize;

}


async fn write_to_file(output_dir: &Path, seq: u128, dot_graph: &mut BytesMut) {
    let file_name = format!("graph-{}.dot", seq);
    let file_path = output_dir.join(file_name);
    // Open a file in write mode
    match File::create(file_path).await {
        Ok(mut file) => {
            let _ = file.write_all(&dot_graph).await;
            let _ = file.flush().await;
        },
        Err(e) => {
            error!("logging dot {} skipped due to {} ",seq,e);
        }
    }
}