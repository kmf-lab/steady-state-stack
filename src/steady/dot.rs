use std::fmt::Write;
use std::fs;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;
use bastion::run;

use bytes::{BufMut, BytesMut};
use log::{error, info};
use time::{format_description, OffsetDateTime};
use uuid::Uuid;
use crate::steady::config;
use crate::steady::serialize::byte_buffer_packer::PackedVecWriter;
use crate::steady::serialize::fast_protocol_packed::write_long_unsigned;
use crate::steady::telemetry::metrics_collector::DiagramData::*;

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
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) color: & 'static str,
    pub(crate) pen_width: & 'static str,
    pub(crate) display_label: String,
    pub(crate) id: usize,
    pub(crate) ctl_labels: Vec<&'static str>
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


pub fn refresh_structure(mut local_state: &mut DotState, name: &str
                         , id: usize
                         , channels_in: Arc<Vec<(usize, Vec<&'static str>)>>
                         , channels_out: Arc<Vec<(usize, Vec<&'static str>)>>) {
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

    channels_in.iter()
        .for_each(|(cin,labels)| {
            if cin.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(id + 1, || {
                    Edge {
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        id: usize::MAX,
                        ctl_labels: Vec::new(),
                    }
                });
            }
            let idx = *cin;
            local_state.edges[idx].id = idx;
            local_state.edges[idx].to = id;
            // Collect the labels that need to be added
            let labels_to_add: Vec<_> = labels.iter()
                .filter(|f| !local_state.edges[idx].ctl_labels.contains(f))
                .cloned()  // Clone the items to be added
                .collect();

// Now, append them to `ctl_labels`
            for label in labels_to_add {
                local_state.edges[idx].ctl_labels.push(label);
            }
        });

    channels_out.iter()
        .for_each(|(cout,labels)| {
            if cout.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(id + 1, || {
                    Edge {
                        from: usize::MAX,
                        to: usize::MAX,
                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                        id: usize::MAX,
                        ctl_labels: Vec::new(),
                    }
                });
            }
            let idx = *cout;
            local_state.edges[idx].id = idx;
            local_state.edges[idx].from = id;
            // Collect the labels that need to be added
            let labels_to_add: Vec<_> = labels.iter()
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
            packed_sent_writer: PackedVecWriter {
                previous: Vec::new(),
                delta_limit: usize::MAX,
                write_count: 0
            },
            packed_take_writer: PackedVecWriter {
                previous: Vec::new(),
                delta_limit: usize::MAX,
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
        run!(async {let _ = fs::create_dir_all(&result.output_log_path);});
        result
    }

    pub fn mark_position(&mut self) {
        self.buffer_bytes_count = self.history_buffer.len();
    }

    pub fn apply_node(&mut self, name: & 'static str, id:usize
                      , chin: Arc<Vec<(usize, Vec<&'static str>)>>
                      , chout: Arc<Vec<(usize, Vec<&'static str>)>>) {

        write_long_unsigned(REC_NODE, &mut self.history_buffer); //message type
        write_long_unsigned(id as u64, &mut self.history_buffer); //message type

        write_long_unsigned(name.len() as u64,&mut self.history_buffer);
        self.history_buffer.write_str(name).expect("internal error writing to ByteMut");

            write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
            chin.iter().for_each(|(id,labels)|{
                write_long_unsigned(*id as u64, &mut self.history_buffer);
                write_long_unsigned(labels.len() as u64, &mut self.history_buffer);
                labels.iter().for_each(|s| {
                    write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                    self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
                });
            });

            write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
            chout.iter().for_each(|(id,labels)|{
                write_long_unsigned(*id as u64, &mut self.history_buffer);
                write_long_unsigned(labels.len() as u64, &mut self.history_buffer);
                labels.iter().for_each(|s| {
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
        // Offloading the blocking I/O operation using `run!`
        let mut file:File = OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(path)?;
        run!(async {
                    use std::io::Write;
                    let _ = file.write_all(data);
                    let _ = file.flush();
                    Result::<(), std::io::Error>::Ok(())
        })
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
