use std::fmt::Write;
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;
use async_std::fs;
use async_std::fs::{File, OpenOptions};
use async_std::io::WriteExt;
use async_std::path::{Path, PathBuf};
use bytes::BytesMut;
use time::{OffsetDateTime, format_description, Instant};
use uuid::Uuid;
use crate::steady::*;
use crate::steady::serialize::fast_protocol_packed::write_long_unsigned;

#[derive(Clone, Debug)]
pub enum DiagramData {
    //only allocates new space when new telemetry children are added.
    Node(u64, & 'static str, usize, Arc<Vec<(usize, Vec<&'static str>)>>, Arc<Vec<(usize, Vec<&'static str>)>>),
    //all consumers will share the same seq vec and it is dropped when the last one consumed it
    //this copy was required so we can gather the next seq while the last gets rendered.
    Edge(u64, Arc<Vec<i128>>, Arc<Vec<i128>>),
        //, total_take, consumed_this_cycle, ma_consumed_per_second, in_flight_this_cycle, ma_inflight_per_second
}

pub(crate) struct RawDiagramState {
    pub(crate) sequence: u64,
    pub(crate) actor_count: usize,
    pub(crate) total_sent: Vec<i128>,
    pub(crate) total_take: Vec<i128>,
    pub(crate) packed_sent_writer: PackedVecWriter<i128>,
    pub(crate) packed_take_writer: PackedVecWriter<i128>,
    pub(crate) history_buffer: BytesMut,

}




const DO_STORE_HISTORY:bool = config::TELEMETRY_HISTORY;

const REC_NODE:u64 = 1;
const REC_EDGE:u64 = 0;

const HISTORY_WRITE_BLOCK_SIZE:usize = 1 << 13; //16 is 65536 NOTE: must be power of 2 and 4096 or larger

pub(crate) async fn run(monitor: SteadyMonitor
       , dynamic_senders_vec: Arc<Mutex<Vec< CollectorDetail >>>
       , optional_server: Option<Arc<Mutex<SteadyTx<DiagramData>>>>
) -> Result<(),()> {

    // set up our history log file

    let mut last_instant = Instant::now();
    // last_instant.

    let guid = Uuid::new_v4().to_string(); // Unique GUID for the run instance
    let format = format_description::parse("[year]_[month]_[day]").unwrap();
    // Define the directory path
    let output_log_path = PathBuf::from("output_logs");
    if DO_STORE_HISTORY & !output_log_path.exists().await {
        fs::create_dir_all(&output_log_path).await;
    }
    //running state for the log history
    let mut file_bytes_written:usize = 0;
    let mut last_file_to_append_onto:String = "".to_string();

    let mut state = RawDiagramState {
        sequence: 0,
        actor_count: 0,
        total_take: Vec::new(), //running totals
        total_sent: Vec::new(), //running totals

        packed_sent_writer: PackedVecWriter{
            previous: Vec::new(),
            delta_limit: usize::MAX,
            write_count: 0
        },
        packed_take_writer: PackedVecWriter{
            previous: Vec::new(),
            delta_limit: usize::MAX,
            write_count: 0
        },
        history_buffer: BytesMut::new()

    };

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
                x.telemetry_take.consume_into(  &mut state.total_take
                                              , &mut state.total_sent);
            }
            nodes
        }; //dropped senders guard so list can be updated with new nodes if needed

        let buffer_bytes_count = state.history_buffer.len();

        if let Some(nodes) = nodes {//only happens when we have new nodes
            send_node_details(&optional_server, &mut state, nodes).await;
        }
        send_edge_details(&optional_server, &mut state).await;

        let old_blocks = (file_bytes_written + buffer_bytes_count)/HISTORY_WRITE_BLOCK_SIZE;
        let new_blocks = (file_bytes_written + state.history_buffer.len())/HISTORY_WRITE_BLOCK_SIZE;

        //we write to disk in blocks just under a fixed power of two size
        //if we are about to enter a new block ensure we write the old one
        if new_blocks > old_blocks {
            let continued_buffer:BytesMut = state.history_buffer.split_off(buffer_bytes_count);

            let file_to_append_onto = format!("{}_{}_log.dat"
                                , OffsetDateTime::now_utc().format(&format).unwrap()
                                , guid);

            //if we are starting a new file reset our counter to zero
            if !last_file_to_append_onto.eq(&file_to_append_onto) {
                file_bytes_written = 0;
                last_file_to_append_onto=file_to_append_onto.clone();
            }


            if let Err(e) = append_to_file(output_log_path.join(&file_to_append_onto)
                                           , &state.history_buffer).await {
                error!("Error writing to file: {}", e);
             } else {
                info!("wrote to hist log now {} bytes",file_bytes_written+buffer_bytes_count);
            }
            file_bytes_written += buffer_bytes_count;
            state.history_buffer = continued_buffer;
        }


        //NOTE: target 32ms updates for 30FPS, with a queue of 8 so writes must be no faster than 4ms
        //      we could double this speed if we have real animation needs but that is unlikely

//TODO: the u128 seq and count types should be moved out as type aliases based on use case need.

        let duration = last_instant.elapsed();
        assert_eq!(true, duration.whole_milliseconds()>=0);
        let sleep_millis = config::TELEMETRY_PRODUCTION_RATE_MS - duration.whole_milliseconds() as usize;
        if sleep_millis > 0 { //only sleep as long as we need
            Delay::new(Duration::from_millis(config::TELEMETRY_PRODUCTION_RATE_MS as u64)).await;
        }
        state.sequence += 1; //increment next frame
        last_instant = Instant::now();

        if false {
            break Ok(());
        }
    }
}

// Async function to append data to a file
async fn append_to_file(path: PathBuf, data: &[u8]) -> async_std::io::Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .await?;

    file.write_all(data).await?;
    file.flush().await?;
    Ok(())
}

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
    state.total_take.resize(max_rx, 0);
    state.total_sent.resize(max_tx, 0);

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

    if DO_STORE_HISTORY {
        write_long_unsigned(REC_NODE, &mut state.history_buffer); //message type
        write_long_unsigned(nodes.len() as u64, &mut state.history_buffer); //message type
        for node in nodes.iter() {
              if let DiagramData::Node(seq,name,id,chin,chout) = node {
                  write_long_unsigned(*id as u64, &mut state.history_buffer); //message type

                  write_long_unsigned(name.len() as u64, &mut state.history_buffer);
                  state.history_buffer.write_str(name).expect("internal error writing to ByteMut");

                  write_long_unsigned(chin.len() as u64, &mut state.history_buffer);
                  chin.iter().for_each(|(id,labels)|{
                      write_long_unsigned(*id as u64, &mut state.history_buffer);
                      write_long_unsigned(labels.len() as u64, &mut state.history_buffer);
                      labels.iter().for_each(|s| {
                          write_long_unsigned(s.len() as u64, &mut state.history_buffer);
                          state.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                      });
                  });

                  write_long_unsigned(chout.len() as u64, &mut state.history_buffer);
                  chout.iter().for_each(|(id,labels)|{
                      write_long_unsigned(*id as u64, &mut state.history_buffer);
                      write_long_unsigned(labels.len() as u64, &mut state.history_buffer);
                      labels.iter().for_each(|s| {
                          write_long_unsigned(s.len() as u64, &mut state.history_buffer);
                          state.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                      });
                  });

              } else {
                  panic!("Internal error, expected iter of Nodes only");
              }

        }
    }

    if let Some(c) = consumer {
            let mut c_guard = c.lock().await;
            let c = c_guard.deref_mut();
            for send_me in nodes.into_iter() {
                let _ = c.send_async(send_me).await;
            }
    }
}

async fn send_edge_details(consumer: &Option<Arc<Mutex<SteadyTx<DiagramData>>>>, mut state: &mut RawDiagramState) {
    let send_me = DiagramData::Edge(state.sequence
                                    , Arc::new(state.total_take.clone())
                                    , Arc::new(state.total_sent.clone()));
    if DO_STORE_HISTORY {
        write_long_unsigned(REC_EDGE, &mut state.history_buffer); //message type
        state.packed_sent_writer.add_vec(&mut state.history_buffer, &state.total_sent);
        state.packed_take_writer.add_vec(&mut state.history_buffer, &state.total_take);
    }

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


pub trait RxTel : Send + Sync {


    //returns an iterator of usize channel ids
    fn tx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)>;
    fn rx_channel_id_vec(&self) -> Vec<(usize, Vec<&'static str>)>;

    fn consume_into(&self, take_target: &mut Vec<i128>, send_target: &mut Vec<i128>);

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