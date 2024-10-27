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
use crate::ActorName;
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
    pub(crate) id: Option<ActorName>,
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
pub(crate) fn build_metric(state: &MetricState, txt_metric: &mut BytesMut) {
    txt_metric.clear(); // Clear the buffer for reuse

    state.nodes.iter().filter(|n| {
        // Only fully defined nodes, some may be in the process of being defined
        n.id.is_some()
    }).for_each(|node| {
        txt_metric.put_slice(node.metric_text.as_bytes());
    });

    state.edges.iter()
        .filter(|e| e.from.is_some() && e.to.is_some())
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
        dot_graph.put_slice(b"];\n");
    });

    state.edges.iter()
        .filter(|e| e.from.is_some() && e.to.is_some())
        // Filter on visibility labels
        // TODO: new feature must also drop nodes if no edges are visible
        // let show = e.ctl_labels.iter().any(|f| config::is_visible(f));
        // let hide = e.ctl_labels.iter().any(|f| config::is_hidden(f));

        .for_each(|edge| {
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
            }
        });
    }
    local_state.nodes[id].id = Some(actor.ident.label);
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
fn define_unified_edges(local_state: &mut MetricState, node_name: ActorName, mdvec: &[Arc<ChannelMetaData>], set_to: bool, frame_rate_ms: u64) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{ChannelMetaData};
    use std::sync::Arc;
    use bytes::BytesMut;
    use std::path::PathBuf;

    // #[test]
    // fn test_node_compute_and_refresh() {
    //     let actor_status = ActorStatus {
    //         await_total_ns: 100,
    //         unit_total_ns: 200,
    //         thread_id: None,
    //         total_count_restarts: 1,
    //         bool_stop: false,
    //         calls: [0;6],
    //     };
    //     let total_work_ns = 1000u128;
    //     let mut node = Node {
    //         id: 1,
    //         color: "grey",
    //         pen_width: "1",
    //         stats_computer: ActorStatsComputer::default(),
    //         display_label: String::new(),
    //         metric_text: String::new(),
    //     };
    //     node.compute_and_refresh(actor_status, total_work_ns);
    //     assert_eq!(node.color, "grey");
    //     assert_eq!(node.pen_width, "1");
    // }

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
        let state = MetricState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("1",None)),
                    color: "grey",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: String::new(),
                    metric_text: "node_metric".to_string(),
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
        let state = MetricState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("1",None)),
                    color: "grey",
                    pen_width: "1",
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "node1".to_string(),
                    metric_text: String::new(),
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
        build_dot(&state, "LR", &mut dot_graph);
        let expected = b"digraph G {\nrankdir=LR;\ngraph [nodesep=.5, ranksep=2.5];\nnode [margin=0.1];\nnode [style=filled, fillcolor=white, fontcolor=black];\nedge [color=white, fontcolor=white];\ngraph [bgcolor=black];\n\"1\" [label=\"node1\", color=grey, penwidth=1];\n}\n";

        let vec = dot_graph.to_vec();
        match String::from_utf8(vec) {
            Ok(mut string) => {
                string = string.replace("\n", "\\n");
                println!("String with literal \\n: {}", string);
            },
            Err(e) => println!("Error: {}", e),
        }
        assert_eq!(dot_graph.to_vec(), expected);
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
        assert!(path.to_str().unwrap().contains(&frame_history.guid));
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

    // #[test]
    // fn test_frame_history_update() {
    //     let mut frame_history = FrameHistory::new(1000);
    //     frame_history.mark_position();
    //     nuclei::block_on(frame_history.update(true));
    //     assert_eq!(frame_history.history_buffer.len(), frame_history.buffer_bytes_count);
    // }

    // #[test]
    // fn test_frame_history_truncate_file() {
    //     let data = BytesMut::from("test data");
    //     let path = PathBuf::from("test_truncate_file.dat");
    //     nuclei::block_on(FrameHistory::truncate_file(path.clone(), data.clone()));
    //     let result = std::fs::read_to_string(path).expect("Failed to read truncated file");
    //     assert_eq!(result, "test data");
    // }

    #[test]
    fn test_frame_history_all_to_file_async() {
        let data = BytesMut::from("test data");
        let path = PathBuf::from("test_all_to_file.dat");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&path)
            .expect("Failed to open file");
        let handle = nuclei::Handle::new(file).expect("Failed to create handle");
        let _ = nuclei::drive(FrameHistory::all_to_file_async(handle, data.clone()));
        let result = std::fs::read_to_string(path).expect("Failed to read written file");
        assert_eq!(result, "test data");
    }

    // #[test]
    // fn test_frame_history_append_to_file() {
    //     let data = BytesMut::from("test data");
    //     let path = PathBuf::from("test_append_file.dat");
    //     nuclei::drive(FrameHistory::append_to_file(path.clone(), data.clone(), true));
    //     let result = std::fs::read_to_string(path).expect("Failed to read appended file");
    //     assert_eq!(result, "test data");
    // }
}
