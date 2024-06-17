//! This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

use log::*;
use num_traits::Zero;
use std::fmt::Write;
use std::fs::{create_dir_all, File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use bytes::{BufMut, BytesMut};
use futures::{AsyncWriteExt};
use nuclei::{self, Handle};
use time::macros::format_description;
use time::OffsetDateTime;

use crate::actor_stats::ActorStatsComputer;
use crate::channel_stats::{ChannelStatsComputer};
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;

/// Represents the state of metrics for the graph, including nodes and edges.
#[derive(Default)]
pub struct MetricState {
    pub(crate) nodes: Vec<Node>, // Position matches the node ID
    pub(crate) edges: Vec<Edge>, // Position matches the channel ID
    pub seq: u64,
}

/// Represents a node in the graph, including metrics and display information.
pub(crate) struct Node {
    pub(crate) id: usize,
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
        let num = actor_status.await_total_ns;
        let den = actor_status.unit_total_ns;
        assert!(den.ge(&num), "num: {} den: {}", num, den);
        let mcpu = if den.is_zero() { 0 } else { 1024 - ((num * 1024) / den) };
        let work = (100u64 * (actor_status.unit_total_ns - actor_status.await_total_ns)) / total_work_ns as u64;

        // Old strings for this actor are passed back in so they get cleared and re-used rather than reallocate
        let (color, pen_width) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.metric_text,
            mcpu,
            work,
            actor_status.total_count_restarts,
            actor_status.bool_stop,
        );
        self.color = color;
        self.pen_width = pen_width;
    }
}

/// Represents an edge in the graph, including metrics and display information.
#[derive(Debug)]
pub(crate) struct Edge {
    pub(crate) id: usize, // Position matches the channel ID
    pub(crate) from: usize, // Monitor actor ID of the sender
    pub(crate) to: usize, // Monitor actor ID of the receiver
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
    pub(crate) fn compute_and_refresh(&mut self, send: i128, take: i128) {
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
pub(crate) fn build_metric(state: &MetricState, txt_metric: &mut BytesMut) {
    txt_metric.clear(); // Clear the buffer for reuse

    state.nodes.iter().filter(|n| {
        // Only fully defined nodes, some may be in the process of being defined
        n.id != usize::MAX
    }).for_each(|node| {
        txt_metric.put_slice(node.metric_text.as_bytes());
    });

    state.edges.iter()
        .filter(|e| e.from != usize::MAX && e.to != usize::MAX)
        // Filter on visibility labels
        // TODO: new feature must also drop nodes if no edges are visible
        // let show = e.ctl_labels.iter().any(|f| config::is_visible(f));
        // let hide = e.ctl_labels.iter().any(|f| config::is_hidden(f));

        .for_each(|edge| {
            txt_metric.put_slice(edge.metric_text.as_bytes());
        });
}

/// Builds the DOT graph from the current state.
///
/// # Arguments
///
/// * `state` - The current metric state.
/// * `rankdir` - The rank direction for the DOT graph.
/// * `dot_graph` - The buffer to store the DOT graph.
pub(crate) fn build_dot(state: &MetricState, rankdir: &str, dot_graph: &mut BytesMut) {
    // Only generate the graph if we have a fully valid graph.
    // If we find any half edges then do not build and wait for later.
    // if state.edges.iter()
    //     .any(|e| (e.from == usize::MAX) ^ (e.to == usize::MAX)) {
    //     return false;
    // };

    dot_graph.clear(); // Clear the buffer for reuse

    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice(rankdir.as_bytes());
    dot_graph.put_slice(b";\n");
    // Keep sidecars near with nodesep and ranksep spreads the rest out for label room.
    dot_graph.put_slice(b"graph [nodesep=.5, ranksep=2.5];\n");
    dot_graph.put_slice(b"node [margin=0.1];\n"); // Gap around text inside the circle

    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    state.nodes.iter().filter(|n| {
        // Only fully defined nodes, some may be in the process of being defined
        n.id != usize::MAX
    }).for_each(|node| {
        dot_graph.put_slice(b"\"");
        dot_graph.put_slice(itoa::Buffer::new().format(node.id).as_bytes());
        dot_graph.put_slice(b"\" [label=\"");
        dot_graph.put_slice(node.display_label.as_bytes());
        dot_graph.put_slice(b"\", color=");
        dot_graph.put_slice(node.color.as_bytes());
        dot_graph.put_slice(b", penwidth=");
        dot_graph.put_slice(node.pen_width.as_bytes());
        dot_graph.put_slice(b"];\n");
    });

    state.edges.iter()
        .filter(|e| e.from != usize::MAX && e.to != usize::MAX)
        // Filter on visibility labels
        // TODO: new feature must also drop nodes if no edges are visible
        // let show = e.ctl_labels.iter().any(|f| config::is_visible(f));
        // let hide = e.ctl_labels.iter().any(|f| config::is_hidden(f));

        .for_each(|edge| {
            dot_graph.put_slice(b"\"");
            dot_graph.put_slice(itoa::Buffer::new().format(edge.from).as_bytes());
            dot_graph.put_slice(b"\" -> \"");
            dot_graph.put_slice(itoa::Buffer::new().format(edge.to).as_bytes());
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
                dot_graph.put_slice(itoa::Buffer::new().format(edge.to).as_bytes());
                dot_graph.put_slice(b"\" \"");
                dot_graph.put_slice(itoa::Buffer::new().format(edge.from).as_bytes());
                dot_graph.put_slice(b"\"}");
            }
        });
    dot_graph.put_slice(b"}\n");
}

/// Represents the frames of a DOT graph, including active metrics and the last graph update.
pub struct DotGraphFrames {
    pub(crate) active_metric: BytesMut,
    pub(crate) active_graph: BytesMut,
    pub(crate) last_graph: Instant,
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
    local_state: &mut MetricState,
    name: &'static str, // TODO: remove if in ActorMetaData
    id: usize, // TODO: remove if in ActorMetaData
    actor: Arc<ActorMetaData>,
    channels_in: &[Arc<ChannelMetaData>],
    channels_out: &[Arc<ChannelMetaData>],
    frame_rate_ms: u64,
) {
    // Rare but needed to ensure vector length
    if id.ge(&local_state.nodes.len()) {
        local_state.nodes.resize_with(id + 1, || {
            Node {
                id: usize::MAX,
                color: "grey",
                pen_width: "2",
                stats_computer: ActorStatsComputer::default(),
                display_label: String::new(), // Defined when the content arrives
                metric_text: String::new(),
            }
        });
    }
    assert!(usize::MAX == local_state.nodes[id].id || id == local_state.nodes[id].id, "id: {} name: {}", id, name);
    local_state.nodes[id].id = id;
    local_state.nodes[id].display_label = name.to_string(); // Temp will be replaced when data arrives.
    local_state.nodes[id].stats_computer.init(actor, frame_rate_ms);

    // Edges are defined by both the sender and the receiver
    // We need to record both monitors in this edge as to and from
    define_unified_edges(local_state, id, channels_in, true, frame_rate_ms);
    define_unified_edges(local_state, id, channels_out, false, frame_rate_ms);
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
fn define_unified_edges(local_state: &mut MetricState, node_id: usize, mdvec: &[Arc<ChannelMetaData>], set_to: bool, frame_rate_ms: u64) {
    assert_ne!(usize::MAX, node_id);
    mdvec.iter().for_each(|meta| {
        let idx = meta.id;
        // info!("define_unified_edges: {} {} {}", idx, node_id, set_to);
        if idx.ge(&local_state.edges.len()) {
            local_state.edges.resize_with(idx + 1, || {
                Edge {
                    id: usize::MAX,
                    from: usize::MAX,
                    to: usize::MAX,
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
            if usize::MAX == edge.to {
                edge.to = node_id;
            } else {
                warn!("internal error");
            }
        } else if usize::MAX == edge.from {
            edge.from = node_id;
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

        if edge.from != usize::MAX && edge.to != usize::MAX {
            edge.stats_computer.init(meta, edge.from, edge.to, frame_rate_ms);
            edge.sidecar = meta.connects_sidecar;
        }
    });
}

/// Represents the frame history for a graph, including packed data and output paths.
pub(crate) struct FrameHistory {
    pub(crate) packed_sent_writer: PackedVecWriter<i128>,
    pub(crate) packed_take_writer: PackedVecWriter<i128>,
    pub(crate) history_buffer: BytesMut,
    pub(crate) guid: String,
    output_log_path: PathBuf,
    file_bytes_written: Arc<AtomicUsize>,
    file_buffer_last_seq: u64,
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
    pub fn new() -> FrameHistory {
        let result = FrameHistory {
            packed_sent_writer: PackedVecWriter::new(),
            packed_take_writer: PackedVecWriter::new(),
            history_buffer: BytesMut::new(),
            // Immutable details
            guid: uuid::Uuid::new_v4().to_string(), // Unique GUID for the run instance
            output_log_path: PathBuf::from("../output_logs"),
            // Running state
            file_bytes_written: Arc::new(AtomicUsize::from(0usize)),
            file_buffer_last_seq: 0u64,
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
    pub fn apply_edge(&mut self, total_take_send: &[(i128, i128)], frame_rate_ms: u64) {
        write_long_unsigned(REC_EDGE, &mut self.history_buffer); // Message type

        let total_take: Vec<i128> = total_take_send.iter().map(|(t, _)| *t).collect();
        let total_send: Vec<i128> = total_take_send.iter().map(|(_, s)| *s).collect();

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
    pub async fn update(&mut self, sequence: u64, flush_all: bool) {
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
                nuclei::spawn_local(async move {
                    if let Err(e) = Self::append_to_file(path, to_be_written, flush_all).await {
                        error!("Error writing to file: {}", e);
                        error!("Due to the above error some history has been lost");
                        // We force a full write for the next time around
                        ptw.store(true, Ordering::SeqCst);
                        psw.store(true, Ordering::SeqCst);
                    }
                    // Change the file_bytes_written to allow for the next spawn.
                    fbw.fetch_add(buf_bytes_count, Ordering::SeqCst);
                }).detach();
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

        let h = Handle::<File>::new(file)?;
        Self::all_to_file_async(h, data).await
    }

    /// Writes all provided data to the file asynchronously.
    ///
    /// # Arguments
    ///
    /// * `h` - The file handle.
    /// * `data` - The data to write.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    async fn all_to_file_async(mut h: Handle<File>, data: BytesMut) -> Result<(), std::io::Error> {
        h.write_all(data.as_ref()).await?;
        // Handle::<File>::flush(&mut h).await?; // TODO: add this as shutdown?
        Ok(())
    }

    /// Appends the provided data to the file at the given path asynchronously.
    ///
    /// # Arguments
    ///
    /// * `path` - The file path.
    /// * `data` - The data to write.
    /// * `flush` - A boolean indicating if the file should be flushed.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    async fn append_to_file(path: PathBuf, data: BytesMut, flush: bool) -> Result<(), std::io::Error> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)?;

        let mut h = Handle::<File>::new(file)?;
        h.write_all(data.as_ref()).await?;
        if flush {
            Handle::<File>::flush(&mut h).await?;
        }
        Ok(())
    }
}

