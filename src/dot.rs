
// new Proactive IO
use nuclei::*;
#[allow(unused_imports)]
use futures::io::SeekFrom;
#[allow(unused_imports)]
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use std::fs::{create_dir_all, File, OpenOptions};

use std::path::PathBuf;

use std::sync::Arc;
use bastion::run;

use std::fmt::Write;
use bytes::{BufMut, BytesMut};
use log::*;
use num_traits::Zero;
use time::{format_description, OffsetDateTime};
use uuid::Uuid;
use crate::actor_stats::ActorStatsComputer;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::channel_stats::ChannelStatsComputer;

pub struct DotState {
    pub(crate) nodes: Vec<Node<>>, //position matches the node id
    pub(crate) edges: Vec<Edge<>>, //position matches the channel id
    pub seq: u64,
}

impl Default for DotState {
    fn default() -> Self {
        DotState {
            nodes: Vec::new(),
            edges: Vec::new(),
            seq: 0,
        }
    }
}

pub(crate) struct Node {
    pub id: usize,
    pub color: & 'static str,
    pub name: & 'static str,
    pub pen_width: & 'static str,
    pub stats_computer: ActorStatsComputer,
    pub display_label: String, //label may also have (n) for replicas
}

impl Node {
    pub(crate) fn compute_and_refresh(&mut self, actor_status: ActorStatus, total_work_ns: u128) {

        //smooth these values out. TODO: add moving avg??

        //  with_metrics   // Metrics:  20% Workload  256 mCPU  (window ma + std dev, 80th percentile)
        let num = actor_status.await_total_ns;
        let den = actor_status.unit_total_ns;
        assert!(den.ge(&num), "num: {} den: {}",num,den);
        let m_cpu = if den.is_zero() {0} else {1024 - ( (num * 1024) / den )};
        //we need sum of all unit-await value

        let workload = (actor_status.unit_total_ns-actor_status.await_total_ns) as f32 / total_work_ns as f32;


        //we could build a stats computer similar to the channel.
        //   with_color_trigger()

        let name = self.name;

/*
        //  with_restarts
        //  with_instance_count  and is stopped
        println!("{} {} {}mCPU {}/{} {:.1}%Workload  restart:{} stop:{} redundancy:{}"
                  ,self.id
                 , self.display_label
                 , m_cpu, num, den, 100f32*workload
                 , actor_status.total_count_restarts
                 , actor_status.bool_stop
                 , actor_status.redundancy);// confirm
*/


        //  with_wait_upon // time/r/wsingle/r/wbatch/other
        //actor_status.batch_write_calls
        //actor.status.single_write_calls


    }
}

pub(crate) struct Edge {
    pub(crate) id: usize, //position matches the channel id
    pub(crate) from: usize, //monitor actor id of the sender
    pub(crate) to: usize, //monitor actor id of the receiver
    pub(crate) color: & 'static str, //results from computer
    pub(crate) sidecar: bool, //mark this edge as attaching to a sidecar
    pub(crate) pen_width: & 'static str, //results from computer
    pub(crate) display_label: String, //results from computer
    pub(crate) ctl_labels: Vec<&'static str>, //visibility tags for render
    stats_computer: ChannelStatsComputer
}
impl Edge {
    pub(crate) fn compute_and_refresh(&mut self, send: i128, take: i128) {

            let (label, color, pen) = self.stats_computer.compute(self.from, send, take);
            // info!("computed edge {} {} {} take:{}",label,color,pen, take);
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

    state.nodes.iter().filter(|n| {
        //only fully defined nodes some
        //may be in the process of being defined
        n.id != usize::MAX
    }).for_each(|node| {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(itoa::Buffer::new()
            .format(node.id)
            .as_bytes()
        );
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    });

    state.edges.iter()
                .filter(|e| {
                       //only fully defined edges some
                       //may be in the process of being defined
                       e.from != usize::MAX && e.to != usize::MAX
                       //filter on visibility labels
                    // TODO: new feature must also drop nodes if no edges are visible
                    // let show = e.ctl_labels.iter().any(|f| config::is_visible(f));
                    // let hide = e.ctl_labels.iter().any(|f| config::is_hidden(f));
                })
                .for_each(|edge| {
                    dot_graph.put_slice(b"\"");
                    dot_graph.put_slice(itoa::Buffer::new()
                        .format(edge.from)
                        .as_bytes());
                    dot_graph.put_slice(b"\" -> \"");
                    dot_graph.put_slice(itoa::Buffer::new()
                        .format(edge.to)
                        .as_bytes());
                    dot_graph.put_slice(b"\" [label=\"");
                    dot_graph.put_slice(edge.display_label.as_bytes());
                    dot_graph.put_slice(b"\", color=");
                    dot_graph.put_slice(edge.color.as_bytes());
                    dot_graph.put_slice(b", penwidth=");
                    dot_graph.put_slice(edge.pen_width.as_bytes());
                    dot_graph.put_slice(b"];\n");

                    //if this edge is a sidecar we document they are in the same row
                    //to help with readability since this is tight with the other node
                    if edge.sidecar {
                        dot_graph.put_slice(b"{rank=same; \"");
                        dot_graph.put_slice(itoa::Buffer::new()
                            .format(edge.to)
                            .as_bytes());
                        dot_graph.put_slice(b"\" \"");
                        dot_graph.put_slice(itoa::Buffer::new()
                            .format(edge.from)
                            .as_bytes());
                        dot_graph.put_slice(b"\"}");
                    }

                });

    dot_graph.put_slice(b"}\n");
}



pub struct DotGraphFrames {
    pub(crate) active_graph: BytesMut,

}





pub fn refresh_structure(local_state: &mut DotState
                         , name: &'static str
                         , id: usize
                         , actor: Arc<ActorMetaData>
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
                name: name,
                stats_computer: ActorStatsComputer::default(),
                display_label: "".to_string(), //defined when the content arrives
            }
        });
    }
    local_state.nodes[id].id = id;
    local_state.nodes[id].display_label = name.to_string(); //temp will be replaced when data arrives.
    local_state.nodes[id].stats_computer.init(actor);

    //edges are defined by both the sender and the receiver
    //we need to record both monitors in this edge as to and from
    define_unified_edges(local_state, id, channels_in, true);
    define_unified_edges(local_state, id, channels_out, false);

}

fn define_unified_edges(local_state: &mut DotState, node_id: usize, mdvec: Arc<Vec<Arc<ChannelMetaData>>>, set_to: bool) {
    mdvec.iter()
        .for_each(|meta| {
            let idx = meta.id;
            //info!("define_unified_edges: {} {} {}",idx, node_id, set_to);
            if idx.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(idx + 1, || {
                    Edge {
                        id: usize::MAX,
                        from: usize::MAX,
                        to: usize::MAX,
                        sidecar: false,
                        stats_computer: ChannelStatsComputer::default(),
                        ctl_labels: Vec::new(), //for visibility control

                        color: "white",
                        pen_width: "3",
                        display_label: "".to_string(),//defined when the content arrives
                    }
                });
            }
            assert!(local_state.edges[idx].id == idx || local_state.edges[idx].id == usize::MAX);
            local_state.edges[idx].id = idx;
            if set_to {
                local_state.edges[idx].to = node_id;
            } else {
                local_state.edges[idx].from = node_id;
            }
            local_state.edges[idx].stats_computer.init(meta);
            local_state.edges[idx].sidecar = meta.connects_sidecar;
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
                delta_write_count: 0
            },
            packed_take_writer: PackedVecWriter {//TODO: use a constructor
                previous: Vec::new(),
                sync_required: true, //first must be full
                delta_write_count: 0
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

    //once every 10 min we will write a full record
    const SAFE_WRITE_LIMIT:usize = (10* 60 * 1000) / super::config::TELEMETRY_PRODUCTION_RATE_MS;

    pub fn apply_edge(&mut self, total_take:Vec<i128>, total_send: Vec<i128>) {
             write_long_unsigned(REC_EDGE, &mut self.history_buffer); //message type

            if self.packed_sent_writer.delta_write_count() < Self::SAFE_WRITE_LIMIT {
                 self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);
            } else {
                self.packed_sent_writer.sync_data();
                self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);

            };

            if self.packed_take_writer.delta_write_count() < Self::SAFE_WRITE_LIMIT {
                self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);
            } else {
                self.packed_take_writer.sync_data();
                self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);
            };


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
            //trace!("attempt to write history");
            let to_be_written = std::mem::replace(&mut self.history_buffer, continued_buffer);
            if let Err(e) = Self::append_to_file(self.output_log_path.join(&file_to_append_onto)
                                                , to_be_written).await {
                error!("Error writing to file: {}", e);
                error!("Due to the above error some history has been lost");

                //we force a full write for the next time around
                self.packed_take_writer.sync_data();
                self.packed_sent_writer.sync_data();

                //NOTE: we could hold for write later but we may run out of memory
                //      so we will just drop the data and hope for the best

            } else {
                //trace!("wrote to hist log now {} bytes",self.file_bytes_written+self.buffer_bytes_count);
            }
            self.file_bytes_written += self.buffer_bytes_count;

        }
        //set after in both cases right now the data matches this sequence
        self.file_buffer_last_seq = sequence;
    }

    fn will_span_into_next_block(&self) -> bool {
            let old_blocks = (self.file_bytes_written + self.buffer_bytes_count) / HISTORY_WRITE_BLOCK_SIZE;
            let new_blocks = (self.file_bytes_written + self.history_buffer.len()) / HISTORY_WRITE_BLOCK_SIZE;
            new_blocks > old_blocks
    }


    async fn append_to_file(path: PathBuf, data: BytesMut) -> Result<(), std::io::Error> {

                    let file = OpenOptions::new()
                            .append(true)
                            .create(true)
                            .open(&path)?;

                    let h = Handle::<File>::new(file)?;
                    nuclei::spawn(Self::all_to_file_async(h, data)).await

    }
    pub(crate) async fn all_to_file_async(mut h: Handle<File>, data: BytesMut) -> Result<(), std::io::Error> {
        h.write_all(data.as_ref()).await?;
        Handle::<File>::flush(&mut h).await?;
        Ok(())
    }

}


/////////////////

