
// new Proactive IO
use nuclei::*;
use futures::io::SeekFrom;
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use std::fs::{create_dir_all, File, OpenOptions};

use std::path::PathBuf;

use std::sync::Arc;
use bastion::run;

use std::fmt::Write;
use bytes::{BufMut, BytesMut};
use log::{error, info};
use time::{format_description, OffsetDateTime};
use uuid::Uuid;
use crate::steady::channel::ChannelBound;
use crate::steady::{config, util};
use crate::steady::monitor::ChannelMetaData;
use crate::steady::serialize::byte_buffer_packer::PackedVecWriter;
use crate::steady::serialize::fast_protocol_packed::write_long_unsigned;

pub struct DotState {
    pub(crate) nodes: Vec<Node<>>, //position matches the node id
    pub(crate) edges: Vec<Edge<>>, //position matches the channel id
    pub seq: u64,
}

pub(crate) struct Node {
    pub id: usize,
    pub color: & 'static str,
    pub pen_width: & 'static str,
    pub display_label: String, //label may also have (n) for replicas
}

pub(crate) struct Edge<> {
    pub(crate) id: usize, //position matches the channel id
    pub(crate) from: usize, //monitor actor id of the sender
    pub(crate) to: usize, //monitor actor id of the receiver
    pub(crate) color: & 'static str, //results from computer
    pub(crate) pen_width: & 'static str, //results from computer
    pub(crate) display_label: String, //results from computer
    pub(crate) ctl_labels: Vec<&'static str>, //visibility tags for render
    stats_computer: ChannelStatsComputer
}
impl<> Edge<> {
    pub(crate) fn compute_and_refresh(&mut self, send: &i128, take: &i128) {
        let (label,color,pen) = self.stats_computer.compute(send, take);
        self.display_label = label;
        self.color = color;
        self.pen_width = pen;
    }
}


pub fn build_dot(state: &DotState, rankdir: &str, dot_graph: &mut BytesMut) {
    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");
    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    for node in &state.nodes {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(node.id.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    for edge in &state.edges {

        // TODO: new feature must also drop nodes if no edges are visible
       // let show = edge.ctl_labels.iter().any(|f| config::is_visible(f));


        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(edge.from.to_string().as_bytes());
        dot_graph.put_slice(b"\" -> \"");
        dot_graph.put_slice(edge.to.to_string().as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(edge.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(edge.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(edge.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    }

    dot_graph.put_slice(b"}\n");
}



pub struct DotGraphFrames {
    pub(crate) active_graph: BytesMut,

}

struct ChannelStatsComputer {

}

const DOT_RED: & 'static str = "red";
const DOT_GREEN: & 'static str = "green";
const DOT_YELLOW: & 'static str = "yellow";
const DOT_WHITE: & 'static str = "white";
const DOT_GREY: & 'static str = "grey";
const DOT_BLACK: & 'static str = "black";

static DOT_PEN_WIDTH: [&'static str; 16]
= ["1", "2", "3", "5", "8", "13", "21", "34", "55", "89", "144", "233", "377", "610", "987", "1597"];


impl ChannelStatsComputer {

    pub(crate) fn new(meta: &Arc<ChannelMetaData>) -> ChannelStatsComputer {

        //TODO: rethink, inflight, persecond and latency display!!

        let labels = &meta.labels;
        let display_labels = &meta.display_labels;
        let line_expansion = &meta.line_expansion;
        let show_type = &meta.show_type;
        let window_in_seconds = &meta.window_in_seconds;
        let percentiles = &meta.percentiles;
        let std_dev = &meta.std_dev;
        let red: Option<ChannelBound> = meta.red.clone();
        let yellow: Option<ChannelBound> = meta.yellow.clone();

        //TODO: build only the structures we need to support the above

        ChannelStatsComputer {




        }
    }

    pub(crate) fn compute(&mut self, send: &i128, take: &i128)
        -> (String, & 'static str, & 'static str) {

//info!("compute {} {}",send,take);
        //DOIT


        (format!("{}",take),"red","5")



    }
}


pub fn refresh_structure(mut local_state: &mut DotState
                         , name: &str
                         , id: usize
                         , channels_in: Arc<Vec<Arc<ChannelMetaData>>>
                         , channels_out: Arc<Vec<Arc<ChannelMetaData>>>
) {
//rare but needed to ensure vector length
    if id.ge(&local_state.nodes.len()) {
        local_state.nodes.resize_with(id + 1, || {
            Node {
                id: usize::MAX,
                color: "grey",
                pen_width: "2",
                display_label: "".to_string(),
            }
        });
    }
    local_state.nodes[id].id = id;
    local_state.nodes[id].display_label = name.to_string();

    //edges are defined by both the sender and the receiver
    //we need to record both monitors in this edge as to and from
    define_unified_edges(&mut local_state, id, channels_in, true);
    define_unified_edges(&mut local_state, id, channels_out, false);


}

fn define_unified_edges(mut local_state: &mut &mut DotState, id: usize, mdvec: Arc<Vec<Arc<ChannelMetaData>>>, set_to: bool) {
    mdvec.iter()
        .for_each(|meta| {
            if meta.id.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(id + 1, || {
                    Edge {
                        id: usize::MAX,
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        stats_computer: ChannelStatsComputer::new(meta),
                        ctl_labels: Vec::new(), //for visibility control
                    }
                });
            }
            let idx = meta.id;
            assert!(local_state.edges[idx].id == idx || local_state.edges[idx].id == usize::MAX);
            local_state.edges[idx].id = idx;
            if set_to {
                local_state.edges[idx].to = id;
            } else {
                local_state.edges[idx].from = id;
            }
            // Collect the labels that need to be added
            // This is redundant but provides safety if two dif label lists are in play
            let labels_to_add: Vec<_> = meta.labels.iter()
                .filter(|f| !local_state.edges[idx].ctl_labels.contains(f))
                .cloned()  // Clone the items to be added
                .collect();
            // Now, append them to `ctl_labels`
            for label in labels_to_add {
                local_state.edges[idx].ctl_labels.push(label);
            }
        });
}

//////////////////////////
/////////////////////////

pub(crate) struct FrameHistory {
    pub(crate) packed_sent_writer: PackedVecWriter<i128>,
    pub(crate) packed_take_writer: PackedVecWriter<i128>,
    pub(crate) history_buffer: BytesMut,
    pub(crate) guid: String,
    pub(crate) format: Vec<format_description::FormatItem<'static>>,
    output_log_path: PathBuf,
    file_bytes_written: usize,
    file_buffer_last_seq: u64,
    last_file_to_append_onto: String,
    buffer_bytes_count: usize,
}

const REC_NODE:u64 = 1;
const REC_EDGE:u64 = 0;

const HISTORY_WRITE_BLOCK_SIZE:usize = 1 << 13; //13 is 8K NOTE: must be power of 2 and 4096 or larger

impl FrameHistory {

    pub async fn new() -> FrameHistory {
        let result = FrameHistory {
            packed_sent_writer: PackedVecWriter { //TODO: use a constructor
                previous: Vec::new(),
                sync_required: true, //first must be full
                write_count: 0
            },
            packed_take_writer: PackedVecWriter {//TODO: use a constructor
                previous: Vec::new(),
                sync_required: true, //first must be full
                write_count: 0
            },
            history_buffer: BytesMut::new(),
            //immutable details
            guid: Uuid::new_v4().to_string(), // Unique GUID for the run instance
            format: format_description::parse("[year]_[month]_[day]").expect("Invalid format description"),
            output_log_path: PathBuf::from("output_logs"),
            //running state
            file_bytes_written: 0usize,
            file_buffer_last_seq: 0u64,
            last_file_to_append_onto: "".to_string(),
            buffer_bytes_count: 0usize,
        };
        run!(async {let _ = create_dir_all(&result.output_log_path);});
        result
    }

    pub fn mark_position(&mut self) {
        self.buffer_bytes_count = self.history_buffer.len();
    }

    pub fn apply_node(&mut self, name: & 'static str, id:usize
                      , chin: Arc<Vec<Arc<ChannelMetaData>>>
                      , chout: Arc<Vec<Arc<ChannelMetaData>>>) {

        write_long_unsigned(REC_NODE, &mut self.history_buffer); //message type
        write_long_unsigned(id as u64, &mut self.history_buffer); //message type

        write_long_unsigned(name.len() as u64,&mut self.history_buffer);
        self.history_buffer.write_str(name).expect("internal error writing to ByteMut");

            write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
            chin.iter().for_each(|meta|{
                write_long_unsigned(meta.id as u64, &mut self.history_buffer);
                write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
                meta.labels.iter().for_each(|s| {
                    write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                    self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                });
            });

            write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
            chout.iter().for_each(|meta|{
                write_long_unsigned(meta.id as u64, &mut self.history_buffer);
                write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
                meta.labels.iter().for_each(|s| {
                    write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                    self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                });
            });
    }


    pub fn apply_edge(&mut self, total_take:Arc<Vec<i128>>, total_send: Arc<Vec<i128>>) {
             write_long_unsigned(REC_EDGE, &mut self.history_buffer); //message type
            self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);
            self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);

    }


    pub async fn update(&mut self, sequence: u64, flush_all: bool) {
        //we write to disk in blocks just under a fixed power of two size
        //if we are about to enter a new block ensure we write the old one
        if flush_all || self.will_span_into_next_block()  {
            let continued_buffer:BytesMut = self.history_buffer.split_off(self.buffer_bytes_count);
            let log_time = OffsetDateTime::now_utc();
            let file_to_append_onto = format!("{}_{}_log.dat"
                                              , log_time
                                                  .format(&self.format)
                                                  .unwrap_or("0000_00_00".to_string())
                                              , self.guid);

            //if we are starting a new file reset our counter to zero
            if !self.last_file_to_append_onto.eq(&file_to_append_onto) {
                self.file_bytes_written = 0;
                self.last_file_to_append_onto=file_to_append_onto.clone();
            }
            if let Err(e) = Self::append_to_file(self.output_log_path.join(&file_to_append_onto)
                                                , &self.history_buffer) {
                error!("Error writing to file: {}", e);
                error!("Due to the above error some history has been lost");

                //we force a full write for the next time around
                self.packed_take_writer.sync_data();
                self.packed_sent_writer.sync_data();

                //NOTE: we could hold for write later but we may run out of memory
                //      so we will just drop the data and hope for the best

            } else {
                info!("wrote to hist log now {} bytes",self.file_bytes_written+self.buffer_bytes_count);
            }
            self.file_bytes_written += self.buffer_bytes_count;
            self.history_buffer = continued_buffer;
        }
        //set after in both cases right now the data matches this sequence
        self.file_buffer_last_seq = sequence;
    }

    fn will_span_into_next_block(&self) -> bool {
            let old_blocks = (self.file_bytes_written + self.buffer_bytes_count) / HISTORY_WRITE_BLOCK_SIZE;
            let new_blocks = (self.file_bytes_written + self.history_buffer.len()) / HISTORY_WRITE_BLOCK_SIZE;
            new_blocks > old_blocks
    }


    fn append_to_file(path: PathBuf, data: &[u8]) -> Result<(), std::io::Error> {
        let file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)?;
        drive(util::all_to_file_async(file, data))
    }

}



/*
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



       //keep current take for next time around
        state.previous_total_take.clear();
        state.total_take.iter().for_each(|f| state.previous_total_take.push(*f) );

        //next index
        state.ma_index = (1+state.ma_index) & MA_BITMASK;
 */
