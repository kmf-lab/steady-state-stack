
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
use time::{format_description, OffsetDateTime};
use uuid::Uuid;
use crate::monitor::ChannelMetaData;
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::stats::ChannelStatsComputer;

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

pub(crate) struct Edge {
    pub(crate) id: usize, //position matches the channel id
    pub(crate) from: usize, //monitor actor id of the sender
    pub(crate) to: usize, //monitor actor id of the receiver
    pub(crate) color: & 'static str, //results from computer
    pub(crate) pen_width: & 'static str, //results from computer
    pub(crate) display_label: String, //results from computer
    pub(crate) ctl_labels: Vec<&'static str>, //visibility tags for render
    stats_computer: ChannelStatsComputer
}
impl Edge {
    pub(crate) fn compute_and_refresh(&mut self, send: i128, take: i128) {
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
    });

    dot_graph.put_slice(b"}\n");
}



pub struct DotGraphFrames {
    pub(crate) active_graph: BytesMut,

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

fn define_unified_edges(local_state: &mut &mut DotState, node_id: usize, mdvec: Arc<Vec<Arc<ChannelMetaData>>>, set_to: bool) {
    mdvec.iter()
        .for_each(|meta| {
            if meta.id.ge(&local_state.edges.len()) {
                local_state.edges.resize_with(meta.id + 1, || {
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
                local_state.edges[idx].to = node_id;
            } else {
                local_state.edges[idx].from = node_id;
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


    pub fn apply_edge(&mut self, total_take:Vec<i128>, total_send: Vec<i128>) {
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
            info!("attempt to write history");
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
                info!("wrote to hist log now {} bytes",self.file_bytes_written+self.buffer_bytes_count);
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

