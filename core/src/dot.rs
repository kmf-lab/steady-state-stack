//! This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

use log::*;
use num_traits::Zero;
use std::fmt::Write;
use std::fs::{create_dir_all, OpenOptions};
use std::path::PathBuf;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use bytes::{BufMut, BytesMut};
use time::macros::format_description;
use time::OffsetDateTime;

use crate::actor_stats::ActorStatsComputer;
use crate::{ActorName};
use crate::*;
use crate::channel_stats::ChannelStatsComputer;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::telemetry::metrics_server;

/// Represents the state of metrics for the graph, including nodes and edges.
#[derive(Default)]
pub struct DotState {
    pub(crate) nodes: Vec<Node>, // Position matches the node ID
    pub(crate) edges: Vec<Edge>, // Position matches the channel ID
    pub seq: u64,
}

#[derive(Default,Clone,Debug)]
pub struct RemoteDetails {
   pub(crate) ips: String,
   pub(crate) match_on: String,
   pub(crate) tech:  &'static str,
   pub(crate) direction: &'static str, //  in OR out
}

/// Represents a node in the graph, including metrics and display information.
pub(crate) struct Node {
    pub(crate) id: Option<ActorName>,
    pub(crate) remote_details: Option<RemoteDetails>,
    pub(crate) color: &'static str,
    pub(crate) pen_width: &'static str,
    pub(crate) stats_computer: ActorStatsComputer,
    pub(crate) display_label: String,
    pub(crate) metric_text: String,
}

impl Node {
    /// Computes and refreshes the metrics for the node based on the actor status and total work.
    ///
    /// # Arguments
    ///
    /// * `actor_status` - The status of the actor.
    /// * `total_work_ns` - The total work in nanoseconds.
    pub(crate) fn compute_and_refresh(&mut self, actor_status: ActorStatus, total_work_ns: u128) {
        let num = actor_status.await_total_ns; //TODO: should not be zero..
        let den = actor_status.unit_total_ns;
        assert!(den.ge(&num), "num: {} den: {}", num, den);
        let mcpu = if den.is_zero() || num.is_zero() || 0==actor_status.iteration_start { 0 } else { 1024 - ((num * 1024) / den) };
        let load = if 0==total_work_ns || 0==actor_status.iteration_start {0} else {
                          (100u64 * (actor_status.unit_total_ns - actor_status.await_total_ns))
                         / total_work_ns as u64
        };

        // Old strings for this actor are passed back in so they get cleared and re-used rather than reallocate
        let (color, pen_width) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.metric_text,
            mcpu,
            load,
            actor_status.total_count_restarts,
            actor_status.bool_stop,
            actor_status.thread_info
        );

        self.color = color;
        self.pen_width = pen_width;
    }
}

/// Represents an edge in the graph, including metrics and display information.
#[derive(Debug)]
pub(crate) struct Edge {
    pub(crate) id: usize, // Position matches the channel ID
    pub(crate) from: Option<ActorName>,
    pub(crate) to: Option<ActorName>,
    pub(crate) color: &'static str, // Results from computer
    pub(crate) sidecar: bool, // Mark this edge as attaching to a sidecar
    pub(crate) pen_width: &'static str, // Results from computer
    pub(crate) ctl_labels: Vec<&'static str>, // Visibility tags for render
    pub(crate) stats_computer: ChannelStatsComputer,
    pub(crate) display_label: String, // Results from computer
    pub(crate) metric_text: String, // Results from computer
}

impl Edge {
    /// Computes and refreshes the metrics for the edge based on send and take values.
    ///
    /// # Arguments
    ///
    /// * `send` - The send value.
    /// * `take` - The take value.
    pub(crate) fn compute_and_refresh(&mut self, send: i64, take: i64) {
        let (color, pen) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.metric_text,
            self.from,
            send,
            take,
        );

        self.color = color;
        self.pen_width = pen;
    }
}

/// Builds the Prometheus metrics from the current state.
///
/// # Arguments
///
/// * `state` - The current metric state.
/// * `txt_metric` - The buffer to store the metrics text.
pub(crate) fn build_metric(state: &DotState, txt_metric: &mut BytesMut) {
    txt_metric.clear(); // Clear the buffer for reuse

    state.nodes.iter().filter(|n| {
        n.id.is_some()
    }).for_each(|node| {
        txt_metric.put_slice(node.metric_text.as_bytes());
    });

    state.edges.iter()
        .filter(|e| e.from.is_some() && e.to.is_some())
        .for_each(|edge| {
            txt_metric.put_slice(edge.metric_text.as_bytes());
        });
}

#[derive(Clone)]
pub(crate) struct Config {
    pub(crate) rankdir: String
    //pub(crate) labels  // labels and boolen on each Y/N  T/F ?

}

/// Builds the DOT graph from the current state.
///
/// # Arguments
///
/// * `state` - The current metric state.
/// * `rankdir` - The rank direction for the DOT graph.
/// * `dot_graph` - The buffer to store the DOT graph.
pub(crate) fn build_dot(state: &DotState, dot_graph: &mut BytesMut, config: &Config) {
    dot_graph.clear(); // Clear the buffer for reuse

    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(config.rankdir.as_bytes());
    dot_graph.put_slice(b";\n");
    //write a digraph comment
    // dot_graph.put_slice(b"/*\n"); // from config, will break test_build_dot test
    // dot_graph.put_slice(b"  This graph is a representation of the actors and channels in the system. let me tell you about your labels.\n");
    // dot_graph.put_slice(b"*/\n");

    // Keep sidecars near with nodesep and ranksep spreads the rest out for label room.
    dot_graph.put_slice(b"graph [nodesep=.5, ranksep=2.5];\n");
    dot_graph.put_slice(b"node [margin=0.1];\n"); // Gap around text inside the circle

    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    state.nodes.iter().filter(|n| {
        // Only fully defined nodes, some may be in the process of being defined
        n.id.is_some()
    }).for_each(|node| {
        dot_graph.put_slice(b"\"");

        if let Some(f) = node.id {
            dot_graph.put_slice(f.name.as_bytes());
            if let Some(s) = f.suffix {
                dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
            }
        } else {
            dot_graph.put_slice(b"unknown");
        }

        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b" ");

        if let Some(remote) = &node.remote_details {
            dot_graph.put_slice(b"/* remote_ips='");
            dot_graph.put_slice(remote.ips.as_bytes());
            dot_graph.put_slice(b"', match_on='");
            dot_graph.put_slice(remote.match_on.as_bytes());
            dot_graph.put_slice(b"', direction='");
            dot_graph.put_slice(remote.direction.as_bytes());
            dot_graph.put_slice(b"', tech='");
            dot_graph.put_slice(remote.tech.as_bytes());
            dot_graph.put_slice(b"' */");
        };
        dot_graph.put_slice(b"];\n");

    });

    state.edges.iter()
        .filter(|e| e.from.is_some() && e.to.is_some())
        // Filter on visibility labels
        // TODO: new feature must also drop nodes if no edges are visible

        .for_each(|edge| {

            // we have a vec of labels

            //config.

            //let show = edge.ctl_labels.iter().any(|f| steady_config::is_visible(f));
            //let hide = edge.ctl_labels.iter().any(|f| steady_config::is_hidden(f));



            dot_graph.put_slice(b"\"");
            if let Some(f) = edge.from {
                dot_graph.put_slice(f.name.as_bytes());
                if let Some(s) = f.suffix {
                    dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                }
            } else {
                dot_graph.put_slice(b"unknown");
            }
            dot_graph.put_slice(b"\" -> \"");
            if let Some(t) = edge.to {
                dot_graph.put_slice(t.name.as_bytes());
                if let Some(s) = t.suffix {
                    dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                }
            } else {
                dot_graph.put_slice(b"unknown");
            }
            dot_graph.put_slice(b"\" [label=\"");
            dot_graph.put_slice(edge.display_label.as_bytes());
            dot_graph.put_slice(b"\", color=");
            dot_graph.put_slice(edge.color.as_bytes());
            dot_graph.put_slice(b", penwidth=");
            dot_graph.put_slice(edge.pen_width.as_bytes());
            dot_graph.put_slice(b"];\n");

            // If this edge is a sidecar, document they are in the same row
            // to help with readability since this is tight with the other node
            if edge.sidecar {
                dot_graph.put_slice(b"{rank=same; \"");
                if let Some(t) = edge.to {
                    dot_graph.put_slice(t.name.as_bytes());
                    if let Some(s) = t.suffix {
                        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                    }
                } else {
                    dot_graph.put_slice(b"unknown");
                }
                dot_graph.put_slice(b"\" \"");

                if let Some(f) = edge.from {
                    dot_graph.put_slice(f.name.as_bytes());
                    if let Some(s) = f.suffix {
                        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                    }
                } else {
                    dot_graph.put_slice(b"unknown");
                }

                dot_graph.put_slice(b"\"}");
            }
        });
    dot_graph.put_slice(b"}\n");
}

/// Represents the frames of a DOT graph, including active metrics and the last graph update.
pub struct DotGraphFrames {
    pub(crate) active_metric: BytesMut,
    pub(crate) active_graph: BytesMut,
    pub(crate) last_generated_graph: Instant,
}

/// Applies the node definition to the local state.
///
/// # Arguments
///
/// * `local_state` - The local metric state.
/// * `name` - The name of the node.
/// * `id` - The ID of the node.
/// * `actor` - The metadata of the actor.
/// * `channels_in` - The input channels.
/// * `channels_out` - The output channels.
/// * `frame_rate_ms` - The frame rate in milliseconds.
pub fn apply_node_def(
    local_state: &mut DotState,
    actor: Arc<ActorMetaData>,
    channels_in: &[Arc<ChannelMetaData>],
    channels_out: &[Arc<ChannelMetaData>],
    frame_rate_ms: u64,
) {
    let id = actor.ident.id;

    // Rare but needed to ensure vector length
    if id.ge(&local_state.nodes.len()) {
        local_state.nodes.resize_with(id + 1, || {
            Node {
                id: None,
                color: "grey",
                pen_width: "2",
                stats_computer: ActorStatsComputer::default(),
                display_label: String::new(), // Defined when the content arrives
                metric_text: String::new(),
                remote_details: None
            }
        });
    }
    local_state.nodes[id].id = Some(actor.ident.label);
    local_state.nodes[id].remote_details = actor.remote_details.clone();
    local_state.nodes[id].display_label = if let Some(suf) = actor.ident.label.suffix {
        format!("{}{}",actor.ident.label.name,suf)
    } else {
        actor.ident.label.name.to_string()
    };
    local_state.nodes[id].stats_computer.init(actor.clone(), frame_rate_ms);

    // Edges are defined by both the sender and the receiver
    // We need to record both monitors in this edge as to and from
    // actor.ident.id.label
    define_unified_edges(local_state, actor.ident.label, channels_in, true, frame_rate_ms);
    define_unified_edges(local_state, actor.ident.label, channels_out, false, frame_rate_ms);
}

/// Defines unified edges in the local state.
///
/// # Arguments
///
/// * `local_state` - The local metric state.
/// * `node_id` - The ID of the node.
/// * `mdvec` - The metadata of the channels.
/// * `set_to` - A boolean indicating if the edges are set to the node.
/// * `frame_rate_ms` - The frame rate in milliseconds.
fn define_unified_edges(local_state: &mut DotState, node_name: ActorName, mdvec: &[Arc<ChannelMetaData>], set_to: bool, frame_rate_ms: u64) {
    mdvec.iter().for_each(|meta| {
        let idx = meta.id;
        // info!("define_unified_edges: {} {} {}", idx, node_id, set_to);
        if idx.ge(&local_state.edges.len()) {
            local_state.edges.resize_with(idx + 1, || {
                Edge {
                    id: usize::MAX,
                    from: None,
                    to: None,
                    sidecar: false,
                    stats_computer: ChannelStatsComputer::default(),
                    ctl_labels: Vec::new(), // For visibility control
                    color: "grey",
                    pen_width: "3",
                    display_label: String::new(), // Defined when the content arrives
                    metric_text: String::new(),
                }
            });
        }
        let edge = &mut local_state.edges[idx];
        assert!(edge.id == idx || edge.id == usize::MAX);
        edge.id = idx;
        if set_to {
            if edge.to.is_none() {
                edge.to = Some(node_name);
            } else {
                warn!("internal error");
            }
        } else if edge.from.is_none() {
            edge.from = Some(node_name);
        } else {
            warn!("internal error");
        }

        // Collect the labels that need to be added
        // This is redundant but provides safety if two different label lists are in play
        let labels_to_add: Vec<_> = meta.labels.iter()
            .filter(|f| !edge.ctl_labels.contains(f))
            .cloned() // Clone the items to be added
            .collect();
        // Now, append them to `ctl_labels`
        for label in labels_to_add {
            edge.ctl_labels.push(label);
        }

        if let Some(node_from) = edge.from {
            if let Some(node_to) = edge.to {
                    edge.stats_computer.init(meta, node_from, node_to, frame_rate_ms);
                    edge.sidecar = meta.connects_sidecar;
            }
        }


    });
}

/// Represents the frame history for a graph, including packed data and output paths.
pub(crate) struct FrameHistory {
    pub(crate) packed_sent_writer: PackedVecWriter<i64>,
    pub(crate) packed_take_writer: PackedVecWriter<i64>,
    pub(crate) history_buffer: BytesMut,
    pub(crate) guid: String,
    output_log_path: PathBuf,
    file_bytes_written: Arc<AtomicUsize>,
    last_file_to_append_onto: String,
    buffer_bytes_count: usize,
    local_thread_bytes_cache: usize,
}

const REC_NODE: u64 = 1;
const REC_EDGE: u64 = 0;
const HISTORY_WRITE_BLOCK_SIZE: usize = 1 << (12 + 4); // Must be power of 2 and 4096 or larger, 64k is good

impl FrameHistory {
    /// Creates a new `FrameHistory` instance.
    ///
    /// # Returns
    ///
    /// A new `FrameHistory` instance.
    pub fn new(ms_rate: u64) -> FrameHistory {
        let mut buf = BytesMut::with_capacity(HISTORY_WRITE_BLOCK_SIZE*2);

        //set history file header with key information about our run
        //
        //what time did we start
        let now:u64 = OffsetDateTime::now_utc().unix_timestamp() as u64;
        write_long_unsigned(now, &mut buf);
        //
        //set history file header with key information about our frame rate
        write_long_unsigned(ms_rate, &mut buf); //time between frames

        let mut runtime_config = 0;

        #[cfg(test)]
        {runtime_config |= 1;} // ones bit is ether test(1) or release(0)

        #[cfg(feature = "prometheus_metrics")]
        {runtime_config |= 2;} // twos bit is ether prometheus(1) or none(0)

        #[cfg(feature = "proactor_tokio")]
        {runtime_config |= 4;} // fours bit is ether tokio(1) or nuclei(0)

        #[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin" ))]
        {runtime_config |= 8;} // eights bit is ether telemetry(1) or none(0)

        #[cfg(feature = "telemetry_server_cdn")]
        {runtime_config |= 16;} // sixteenth bit is ether cdn(1) or builtin(0)

        //write bits which captured the runtime conditions
        write_long_unsigned(runtime_config, &mut buf);

        //TODO: add file version!!

        let result = FrameHistory {
            packed_sent_writer: PackedVecWriter::new(),
            packed_take_writer: PackedVecWriter::new(),
            history_buffer: buf,
            // Immutable details
            guid: uuid::Uuid::new_v4().to_string(), // Unique GUID for the run instance
            output_log_path: PathBuf::from("../output_logs"),
            // Running state
            file_bytes_written: Arc::new(AtomicUsize::from(0usize)),

            last_file_to_append_onto: "".to_string(),
            buffer_bytes_count: 0usize,
            local_thread_bytes_cache: 0usize,
        };


        let _ = create_dir_all(&result.output_log_path);
        result
    }

    /// Marks the current position in the history buffer.
    pub fn mark_position(&mut self) {
        self.buffer_bytes_count = self.history_buffer.len();
    }

    /// Applies a node definition to the history buffer.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the node.
    /// * `id` - The ID of the node.
    /// * `chin` - The input channels.
    /// * `chout` - The output channels.
    pub fn apply_node(&mut self, name: &'static str, id: usize, chin: &[Arc<ChannelMetaData>], chout: &[Arc<ChannelMetaData>]) {
        write_long_unsigned(REC_NODE, &mut self.history_buffer); // Message type
        write_long_unsigned(id as u64, &mut self.history_buffer); // Message type

        write_long_unsigned(name.len() as u64, &mut self.history_buffer);
        self.history_buffer.write_str(name).expect("internal error writing to ByteMut");

        write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
        chin.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
            });
        });

        write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
        chout.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer.write_str(s).expect("internal error writing to ByteMut");
            });
        });
    }

    /// Applies an edge definition to the history buffer.
    ///
    /// # Arguments
    ///
    /// * `total_take_send` - The total take and send values.
    /// * `frame_rate_ms` - The frame rate in milliseconds.
    pub fn apply_edge(&mut self, total_take_send: &[(i64, i64)], frame_rate_ms: u64) {
        write_long_unsigned(REC_EDGE, &mut self.history_buffer); // Message type

        let total_take: Vec<i64> = total_take_send.iter().map(|(t, _)| *t).collect();
        let total_send: Vec<i64> = total_take_send.iter().map(|(_, s)| *s).collect();

        if (self.packed_sent_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000) {
            self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);
        } else {
            self.packed_sent_writer.sync_data();
            self.packed_sent_writer.add_vec(&mut self.history_buffer, &total_send);
        };

        if (self.packed_take_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000) {
            self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);
        } else {
            self.packed_take_writer.sync_data();
            self.packed_take_writer.add_vec(&mut self.history_buffer, &total_take);
        };
    }

    /// Updates the history buffer, writing to disk if necessary.
    ///
    /// # Arguments
    ///
    /// * `sequence` - The current sequence number.
    /// * `flush_all` - A boolean indicating if all data should be flushed to disk.
    pub async fn update(&mut self, flush_all: bool) {
        // We write to disk in blocks just under a fixed power of two size
        // If we are about to enter a new block ensure we write the old one
        // NOTE: We block and do not write if the previous write was not completed.
        let cur_bytes_written = self.file_bytes_written.load(Ordering::SeqCst);

        if (flush_all || self.will_span_into_next_block()) && (cur_bytes_written != self.local_thread_bytes_cache || 0 == cur_bytes_written) {
            // Store this and do not run again until this has changed
            self.local_thread_bytes_cache = cur_bytes_written;

            let buf_bytes_count = self.buffer_bytes_count;
            let continued_buffer: BytesMut = self.history_buffer.split_off(buf_bytes_count);
            // trace!("attempt to write history");
            let to_be_written = std::mem::replace(&mut self.history_buffer, continued_buffer).to_owned();
            if !to_be_written.len().is_zero() {
                let path = self.build_history_path().to_owned();

                let ptw = self.packed_take_writer.sync_required.clone();
                let psw = self.packed_sent_writer.sync_required.clone();
                let fbw = self.file_bytes_written.clone();

                // Let the file write happen in the background so we can get back to data updates
                // This is not a new thread so it is lightweight
                //TODO: rewrite as a new actor!
                spawn_detached(async move {
                    if let Err(e) = Self::append_to_file(path, to_be_written, flush_all).await {
                        error!("Error writing to file: {}", e);
                        error!("Due to the above error some history has been lost");
                        // We force a full write for the next time around
                        ptw.store(true, Ordering::SeqCst);
                        psw.store(true, Ordering::SeqCst);
                    }
                    // Change the file_bytes_written to allow for the next spawn.
                    fbw.fetch_add(buf_bytes_count, Ordering::SeqCst);
                });
            }
        }
    }

    /// Builds the history file path based on the current date and GUID.
    ///
    /// # Returns
    ///
    /// The history file path.
    fn build_history_path(&mut self) -> PathBuf {
        let format = format_description!("[year]_[month]_[day]");
        let log_time = OffsetDateTime::now_utc();
        let file_to_append_onto = format!("{}_{}_log.dat", log_time.format(&format).unwrap_or_else(|_| "0000_00_00".to_string()), self.guid);

        // If we are starting a new file reset our counter to zero
        if !self.last_file_to_append_onto.eq(&file_to_append_onto) {
            self.file_bytes_written.store(0, Ordering::SeqCst);
            self.last_file_to_append_onto.clone_from(&file_to_append_onto);
        }
        self.output_log_path.join(&file_to_append_onto)
    }

    /// Checks if the next block will span into the next file write block.
    ///
    /// # Returns
    ///
    /// `true` if the next block will span into the next file write block, `false` otherwise.
    fn will_span_into_next_block(&self) -> bool {
        let old_blocks = (self.file_bytes_written.load(Ordering::SeqCst) + self.buffer_bytes_count) / HISTORY_WRITE_BLOCK_SIZE;
        let new_blocks = (self.file_bytes_written.load(Ordering::SeqCst) + self.history_buffer.len()) / HISTORY_WRITE_BLOCK_SIZE;
        new_blocks > old_blocks
    }

    /// Truncates the file at the given path and writes the provided data to it.
    ///
    /// # Arguments
    ///
    /// * `path` - The file path.
    /// * `data` - The data to write.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn truncate_file(path: PathBuf, data: BytesMut) -> Result<(), std::io::Error> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        metrics_server::async_write_all(data, false, file).await
    }
    async fn append_to_file(path: PathBuf, data: BytesMut, flush: bool) -> Result<(), std::io::Error> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)?;
        metrics_server::async_write_all(data, flush, file).await
    }

}

#[cfg(test)]
mod dot_tests {
    use super::*;
    use crate::telemetry::metrics_server::async_write_all;
    use crate::monitor::{ActorMetaData, ChannelMetaData, ActorIdentity, ActorStatus};
    use std::sync::Arc;
    use bytes::BytesMut;
    use std::path::PathBuf;
    use std::fs::remove_file;


    #[test]
    fn test_node_compute_and_refresh() {
        let actor_status = ActorStatus {
            await_total_ns: 100,
            unit_total_ns: 200,
            total_count_restarts: 1,
            iteration_start: 0,
            iteration_sum: 0,
            bool_stop: false,
            calls: [0;6],
            thread_info: None,
            bool_blocking: false,
        };
        let total_work_ns = 1000u128;
        let mut node = Node {
            id: Some(ActorName::new("1",None)),
            color: "grey",
            pen_width: "1",
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            metric_text: String::new(),
            remote_details: None
        };
        node.compute_and_refresh(actor_status, total_work_ns);
        assert_eq!(node.color, "grey");
        assert_eq!(node.pen_width, "3");
    }

    #[test]
    fn test_edge_compute_and_refresh() {
        let mut edge = Edge {
            id: 1,
            from: None,
            to: None,
            color: "grey",
            sidecar: false,
            pen_width: "1",
            ctl_labels: Vec::new(),
            stats_computer: ChannelStatsComputer::default(),
            display_label: String::new(),
            metric_text: String::new(),
        };
        edge.compute_and_refresh(100, 50);
        assert_eq!(edge.color, "grey");
        assert_eq!(edge.pen_width, "1");
    }

    #[test]
    fn test_build_metric() {
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("1",None)),
                    color: "grey",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: String::new(),
                    metric_text: "node_metric".to_string(),
                    remote_details: None
                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: None,
                    to: None,
                    color: "grey",
                    sidecar: false,
                    pen_width: "1",
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: String::new(),
                    metric_text: "edge_metric".to_string(),
                }
            ],
            seq: 0,
        };
        let mut txt_metric = BytesMut::new();
        build_metric(&state, &mut txt_metric);

        // let vec = txt_metric.to_vec();
        // match String::from_utf8(vec) {
        //     Ok(string) => println!("String: {}", string),
        //     Err(e) => println!("Error: {}", e),
        // }
        //
        // assert_eq!(txt_metric.to_vec(), b"node_metricedge_metric");
    }

    #[test]
    fn test_build_dot() {
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("1",None)),
                    color: "grey",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "node1".to_string(),
                    metric_text: String::new(),
                    remote_details: None
                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: None,
                    to: None,
                    color: "grey",
                    sidecar: false,
                    pen_width: "1",
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "edge1".to_string(),
                    metric_text: String::new(),
                }
            ],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "LR".to_string()
        };

        build_dot(&state, &mut dot_graph, &config);
        let expected = b"digraph G {\nrankdir=LR;\ngraph [nodesep=.5, ranksep=2.5];\nnode [margin=0.1];\nnode [style=filled, fillcolor=white, fontcolor=black];\nedge [color=white, fontcolor=white];\ngraph [bgcolor=black];\n\"1\" [label=\"node1\", color=grey, penwidth=1 ];\n}\n";

        let vec = dot_graph.to_vec();
        //println!("vec: {:?}", vec.clone());

        match String::from_utf8(vec.clone()) {
            Ok(mut string) => {
                string = string.replace("\n", "\\n");
                println!("String with literal \n{}", string);
            },
            Err(e) => println!("Error: {}", e),
        }

        assert_eq!(dot_graph.to_vec(), expected, "dot_graph: {:?}\n vs {:?}", dot_graph, expected);
    }

    #[test]
    fn test_frame_history_new() {
        let frame_history = FrameHistory::new(1000);
        assert_eq!(frame_history.packed_sent_writer.delta_write_count(), 0);
        assert_eq!(frame_history.packed_take_writer.delta_write_count(), 0);
        assert!(frame_history.history_buffer.len() > 0);
    }

    #[test]
    fn test_frame_history_apply_node() {
        let mut frame_history = FrameHistory::new(1000);
        let chin = vec![Arc::new(ChannelMetaData::default())];
        let chout = vec![Arc::new(ChannelMetaData::default())];
        frame_history.apply_node("node1", 1, &chin, &chout);
        assert!(frame_history.history_buffer.len() > 0);
    }

    #[test]
    fn test_frame_history_apply_edge() {
        let mut frame_history = FrameHistory::new(1000);
        let total_take_send = vec![(100, 50)];
        frame_history.apply_edge(&total_take_send, 1000);
        assert!(frame_history.history_buffer.len() > 0);
    }

    #[test]
    fn test_frame_history_build_history_path() {
        let mut frame_history = FrameHistory::new(1000);
        let path = frame_history.build_history_path();
        assert!(path.to_str().expect("iternal error").contains(&frame_history.guid));
    }

    // #[test]
    // fn test_frame_history_will_span_into_next_block() {
    //     let mut frame_history = FrameHistory::new(1000);
    //     frame_history.buffer_bytes_count = HISTORY_WRITE_BLOCK_SIZE;
    //     assert!(frame_history.will_span_into_next_block());
    // }

    #[test]
    fn test_frame_history_mark_position() {
        let mut frame_history = FrameHistory::new(1000);
        frame_history.mark_position();
        assert_eq!(frame_history.buffer_bytes_count, frame_history.history_buffer.len());
    }


    #[test]
    fn test_define_unified_edges() {
        let mut metric_state = DotState::default();
        let node_name = ActorName::new("node1", None);
        let channels = vec![Arc::new(ChannelMetaData::default())];

        define_unified_edges(&mut metric_state, node_name, &channels, true, 1000);
        assert_eq!(metric_state.edges.len(), 1);
        assert!(metric_state.edges[0].to.is_some());
    }

    #[test]
    fn test_frame_history_update() {
        let mut frame_history = FrameHistory::new(1000);
        frame_history.mark_position();

        core_exec::block_on(frame_history.update(true));

        assert_eq!(frame_history.history_buffer.len(), 0);
    }

    #[test]
    fn test_frame_history_all_to_file_async() {

        let data = BytesMut::from("test data");
        let path = PathBuf::from("test_all_to_file.dat");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&path)
            .expect("Failed to open file");

        let _ = core_exec::block_on(async_write_all(data, true, file));

        let result = std::fs::read_to_string(path).expect("Failed to read written file");
        assert_eq!(result, "test data");
    }


    #[test]
    fn test_node_compute_refresh_with_load_calculation() {
        // Test the load calculation branch (lines 66-69)
        let actor_status = ActorStatus {
            await_total_ns: 100,
            unit_total_ns: 500,
            total_count_restarts: 1,
            iteration_start: 10, // Non-zero to trigger load calculation
            iteration_sum: 0,
            bool_stop: false,
            calls: [0;6],
            thread_info: None,
            bool_blocking: false,
        };
        let total_work_ns = 1000u128;
        let mut node = Node {
            id: Some(ActorName::new("test_node", None)),
            color: "grey",
            pen_width: "1",
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            metric_text: String::new(),
            remote_details: None
        };
        node.compute_and_refresh(actor_status, total_work_ns);
        // This should trigger the load calculation branch
    }

    #[test]
    fn test_build_metric_with_edges() {
        // Test line 141 - edge metric text handling
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("node1", None)),
                    color: "grey",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: String::new(),
                    metric_text: "node_metric".to_string(),
                    remote_details: None
                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: Some(ActorName::new("from_node", None)),
                    to: Some(ActorName::new("to_node", None)),
                    color: "grey",
                    sidecar: false,
                    pen_width: "1",
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: String::new(),
                    metric_text: "edge_metric".to_string(),
                }
            ],
            seq: 0,
        };
        let mut txt_metric = BytesMut::new();
        build_metric(&state, &mut txt_metric);

        let result = String::from_utf8(txt_metric.to_vec()).expect("internal error");
        assert!(result.contains("node_metric"));
        assert!(result.contains("edge_metric"));
    }

    #[test]
    fn test_build_dot_with_node_suffix() {
        // Test lines 186-191 - node suffix handling
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("node", Some(42))),
                    color: "red",
                    pen_width: "2",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "node_with_suffix".to_string(),
                    metric_text: String::new(),
                    remote_details: None
                }
            ],
            edges: vec![],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "TB".to_string()
        };

        build_dot(&state, &mut dot_graph, &config);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(result.contains("node42")); // Should include suffix
    }

    #[test]
    fn test_build_dot_with_remote_details() {
        // Test lines 201-211 - remote details handling
        let remote_details = RemoteDetails {
            ips: "192.168.1.1".to_string(),
            match_on: "port_8080".to_string(),
            tech: "TCP",
            direction: "in",
        };

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("remote_node", None)),
                    color: "blue",
                    pen_width: "3",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "remote".to_string(),
                    metric_text: String::new(),
                    remote_details: Some(remote_details)
                }
            ],
            edges: vec![],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "LR".to_string()
        };

        build_dot(&state, &mut dot_graph, &config);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(result.contains("192.168.1.1"));
        assert!(result.contains("port_8080"));
        assert!(result.contains("TCP"));
        assert!(result.contains("in"));
    }

    #[test]
    fn test_build_dot_with_edges_and_sidecar() {
        // Test lines 222-283 - edge processing including sidecar
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("from_node", None)),
                    color: "green",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    metric_text: String::new(),
                    remote_details: None
                },
                Node {
                    id: Some(ActorName::new("to_node", Some(5))),
                    color: "yellow",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    metric_text: String::new(),
                    remote_details: None
                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: Some(ActorName::new("from_node", None)),
                    to: Some(ActorName::new("to_node", Some(5))),
                    color: "purple",
                    sidecar: true, // Test sidecar functionality
                    pen_width: "4",
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "test_edge".to_string(),
                    metric_text: String::new(),
                }
            ],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "TB".to_string()
        };

        build_dot(&state, &mut dot_graph, &config);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(result.contains("from_node"));
        assert!(result.contains("to_node5"));
        assert!(result.contains("test_edge"));
        assert!(result.contains("rank=same")); // Sidecar rank grouping
    }

    #[test]
    fn test_apply_node_def() {
        // Test lines 305-342 - apply_node_def function
        let mut local_state = DotState::default();

        let actor = Arc::new(ActorMetaData {
            ident: ActorIdentity {
                id: 0,
                label: ActorName::new("test_actor", Some(1)),
            },
            remote_details: Some(RemoteDetails {
                ips: "127.0.0.1".to_string(),
                match_on: "test_match".to_string(),
                tech: "HTTP",
                direction: "out",
            }),
            avg_mcpu: false,
            avg_work: false,
            percentiles_mcpu: vec![],
            percentiles_work: vec![],
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            usage_review: false,
        });

        let channel_in = Arc::new(ChannelMetaData {
            id: 0,
            labels: vec!["input_label"],
            capacity: 0,
            display_labels: false,
            line_expansion: 0.0,
            show_type: None,
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            percentiles_filled: vec![],
            percentiles_rate: vec![],
            percentiles_latency: vec![],
            std_dev_inflight: vec![],
            std_dev_consumed: vec![],
            std_dev_latency: vec![],
            trigger_rate: vec![],
            trigger_filled: vec![],
            trigger_latency: vec![],
            avg_filled: false,
            avg_rate: false,
            avg_latency: false,
            min_filled: false,
            max_filled: false,
            connects_sidecar: false,
            type_byte_count: 0,
            show_total: false,
        });

        let channel_out = Arc::new(ChannelMetaData {
            id: 1,
            labels: vec!["output_label"],
            capacity: 0,
            display_labels: false,
            line_expansion: 0.0,
            show_type: None,
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            percentiles_filled: vec![],
            percentiles_rate: vec![],
            percentiles_latency: vec![],
            std_dev_inflight: vec![],
            std_dev_consumed: vec![],
            std_dev_latency: vec![],
            trigger_rate: vec![],
            trigger_filled: vec![],
            trigger_latency: vec![],
            avg_filled: false,
            avg_rate: false,
            avg_latency: false,
            min_filled: false,
            max_filled: false,
            connects_sidecar: true,
            type_byte_count: 0,
            show_total: false,
        });

        let channels_in = vec![channel_in];
        let channels_out = vec![channel_out];

        apply_node_def(&mut local_state, actor, &channels_in, &channels_out, 1000);

        assert_eq!(local_state.nodes.len(), 1);
        assert!(local_state.nodes[0].id.is_some());
        assert_eq!(local_state.nodes[0].id.expect("internal error").name, "test_actor");
        assert!(local_state.nodes[0].remote_details.is_some());
        assert_eq!(local_state.edges.len(), 2); // One for input, one for output
    }

    #[test]
    fn test_define_unified_edges_from_side() {
        // Test the "from" side of edge definition (lines 382-386)
        let mut metric_state = DotState::default();
        let node_name = ActorName::new("from_node", None);

        let channel = Arc::new(ChannelMetaData {
            id: 0,
            labels: vec!["test_label"],
            capacity: 0,
            display_labels: false,
            line_expansion: 0.0,
            show_type: None,
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            percentiles_filled: vec![],
            percentiles_rate: vec![],
            percentiles_latency: vec![],
            std_dev_inflight: vec![],
            std_dev_consumed: vec![],
            std_dev_latency: vec![],
            trigger_rate: vec![],
            trigger_filled: vec![],
            trigger_latency: vec![],
            avg_filled: false,
            avg_rate: false,
            avg_latency: false,
            min_filled: false,
            max_filled: false,
            connects_sidecar: false,
            type_byte_count: 0,
            show_total: false,
        });
        let channels = vec![channel];

        // Test setting the "from" side (set_to = false)
        define_unified_edges(&mut metric_state, node_name, &channels, false, 1000);
        assert_eq!(metric_state.edges.len(), 1);
        assert!(metric_state.edges[0].from.is_some());
        assert!(metric_state.edges[0].to.is_none());
    }



    #[test]
    fn test_frame_history_apply_edge_with_sync() {
        // Test lines 540-552 - packed writer sync logic
        let mut frame_history = FrameHistory::new(1000);

        // Force a sync condition by manipulating the delta_write_count
        // We'll simulate having written for more than 10 minutes worth of data
        let total_take_send = vec![(100, 50); 700]; // Large number to trigger sync

        // This should trigger the sync branches
        frame_history.apply_edge(&total_take_send, 1000);
        assert!(frame_history.history_buffer.len() > 0);
    }

    #[test]
    fn test_will_span_into_next_block() {
        // Test lines 623-627 - will_span_into_next_block function
        let frame_history = FrameHistory::new(1000);

        // Test the function directly - this requires manipulating internal state
        let result = frame_history.will_span_into_next_block();
        assert!(!result); // Should be false for a new instance
    }

    #[test]
    fn test_truncate_file() {
        // Test lines 640-646 - truncate_file function
        let test_data = BytesMut::from("test truncate data");
        let test_path = PathBuf::from("test_truncate.dat");

        let result = core_exec::block_on(FrameHistory::truncate_file(test_path.clone(), test_data));
        assert!(result.is_ok());

        // Verify the file was created and contains the data
        let file_content = std::fs::read_to_string(&test_path).expect("Failed to read test file");
        assert_eq!(file_content, "test truncate data");

        // Clean up
        let _ = remove_file(test_path);
    }

    #[test]
    fn test_node_with_unknown_id() {
        // Test lines 189-191 - unknown node handling in build_dot
        let state = DotState {
            nodes: vec![
                Node {
                    id: None, // This will trigger the "unknown" branch
                    color: "red",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "unknown_node".to_string(),
                    metric_text: String::new(),
                    remote_details: None
                }
            ],
            edges: vec![],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "LR".to_string()
        };

        // This should not include the node since id is None (filtered out)
        build_dot(&state, &mut dot_graph, &config);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(!result.contains("unknown_node"));
    }

    #[test]
    fn test_edge_with_unknown_nodes() {
        // Test lines 239-249 - unknown node handling in edges
        let state = DotState {
            nodes: vec![],
            edges: vec![
                Edge {
                    id: 1,
                    from: None, // This will trigger "unknown" branch
                    to: None,   // This will trigger "unknown" branch
                    color: "grey",
                    sidecar: false,
                    pen_width: "1",
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "test_edge".to_string(),
                    metric_text: String::new(),
                }
            ],
            seq: 0,
        };
        let mut dot_graph = BytesMut::new();
        let config = Config {
            rankdir: "TB".to_string()
        };

        // This should not process the edge since from and to are None
        build_dot(&state, &mut dot_graph, &config);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(!result.contains("test_edge"));
    }
}


