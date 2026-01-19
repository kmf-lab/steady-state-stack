//! This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

use log::*;
use num_traits::Zero;
use std::fmt::Write;
use std::fs::{create_dir_all, OpenOptions};
use std::path::PathBuf;
use std::collections::{HashMap, BTreeMap};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use bytes::{BufMut, BytesMut};
use time::macros::format_description;
use time::OffsetDateTime;

use crate::actor_stats::ActorStatsComputer;
use crate::ActorName;
use crate::*;
use crate::channel_stats::ChannelStatsComputer;
use crate::dot_edge::Edge;
use crate::dot_node::Node;
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::telemetry::metrics_server;

/// Represents the state of metrics for the graph, including nodes and edges.
#[derive(Default)]
pub struct DotState {
    pub(crate) nodes: Vec<Node>, // Position matches the node ID
    pub(crate) edges: Vec<Edge>, // Position matches the channel ID
    pub seq: u64,
    pub(crate) telemetry_colors: Option<(String, String)>,
    pub(crate) refresh_rate_ms: u64,
    pub(crate) bundle_floor_size: usize,
}

#[derive(Default,Clone,Debug)]
pub struct RemoteDetails {
   pub(crate) ips: String,
   pub(crate) match_on: String,
   pub(crate) tech:  &'static str,
   pub(crate) direction: &'static str, //  in OR out
}

/// The default pen width for nodes in the DOT graph.
pub(crate) const NODE_PEN_WIDTH: &str = "3"; 
/// The default pen width for edges in the DOT graph.
pub(crate) const EDGE_PEN_WIDTH: &str = "1"; 
/// The pen width for bundles of single kind of channels.
pub(crate) const BUNDLE_PEN_WIDTH: &str = "4";
/// The pen width for bundles of partnered channels.
pub(crate) const PARTNER_BUNDLE_PEN_WIDTH: &str = "2";

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
        .filter(|e| e.id != usize::MAX)
        .for_each(|edge| {
            txt_metric.put_slice(edge.metric_text.as_bytes());
        });
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, PartialOrd, Ord)]
struct PrimaryGroupKey {
    from_name: Option<&'static str>,
    from_suffix: Option<usize>,
    to_name: Option<&'static str>,
    to_suffix: Option<usize>,
    sub_capacities: Vec<usize>,
    type_name: String,
    color: String,
    sidecar: bool,
    partner: Option<&'static str>,
}

/// Builds the DOT graph from the current state.
///
/// # Arguments
///
/// * `state` - The current metric state.
/// * `dot_graph` - The buffer to store the DOT graph.
pub(crate) fn build_dot(state: &DotState, dot_graph: &mut BytesMut) {
    dot_graph.clear(); // Clear the buffer for reuse

    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice("LR".as_bytes());
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
        if !node.tooltip.is_empty() {
            dot_graph.put_slice(b"\", tooltip=\"");
            let escaped = node.tooltip.replace('"', "'").replace('\n', "\\n");
            dot_graph.put_slice(escaped.as_bytes());
        }
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

    // Stage 1: Partnering
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct PartnerKey {
        from: Option<(&'static str, Option<usize>)>,
        to: Option<(&'static str, Option<usize>)>,
        partner: Option<&'static str>,
        bundle_index: Option<usize>,
        edge_id: Option<usize>, // Only used if partner is None to keep them separate
    }

    let mut partner_groups: BTreeMap<PartnerKey, Vec<&Edge>> = BTreeMap::new();
    for edge in state.edges.iter().filter(|e| e.id != usize::MAX) {
        let key = PartnerKey {
            from: edge.from.map(|n| (n.name, n.suffix)),
            to: edge.to.map(|n| (n.name, n.suffix)),
            partner: edge.partner,
            bundle_index: edge.bundle_index,
            edge_id: if edge.partner.is_none() { Some(edge.id) } else { None },
        };
        partner_groups.entry(key).or_default().push(edge);
    }

    struct PartneredEdge {
        from: Option<ActorName>,
        to: Option<ActorName>,
        combined_color: String,
        summary_label: String,
        combined_type: String,
        partner_name: Option<&'static str>,
        sub_capacities: Vec<usize>,
        sidecar: bool,
        saturation_score: f64,
        tooltip: String,
        sub_totals: Vec<u128>,
        last_total: i64,
        ids: Vec<usize>,
        ctl_labels: Vec<&'static str>,
        pen_width: String,
        memory_footprint: usize,
        show_memory: bool,
    }

    let mut partnered_edges = Vec::new();
    for (_, mut edges) in partner_groups {
        // S-Tier: Sort edges by type name to ensure index-aligned vectors are stable across the bundle
        edges.sort_by_key(|e| e.stats_computer.show_type.unwrap_or(""));
        
        let first = edges[0];
        let is_partnered = first.partner.is_some();
        let mut combined_color = String::new();
        let mut type_list = Vec::new();
        let mut ctl_labels = Vec::new();
        let mut ids = Vec::new();
        let mut tooltip = String::new();
        let mut sum_saturation = 0.0;
        let mut sub_capacities = Vec::with_capacity(edges.len());
        let mut sub_totals = Vec::with_capacity(edges.len());
        let mut sum_total = 0;
        let mut memory_footprint = 0;
        let mut show_memory = false;

        for (i, e) in edges.iter().enumerate() {
            if i > 0 {
                if is_partnered {
                    combined_color.push_str(":black:");
                } else {
                    combined_color.push(':');
                }
            }
            combined_color.push_str(e.color);
            
            let short_type = e.stats_computer.show_type.unwrap_or("");
            if !short_type.is_empty() {
                type_list.push(short_type);
            }
            sub_capacities.push(e.stats_computer.capacity);
            sub_totals.push(e.stats_computer.total_consumed);
            memory_footprint += e.stats_computer.memory_footprint;
            show_memory |= e.stats_computer.show_memory;
            
            let _ = write!(tooltip, "CH#{}: {} | Cap: ", 
                e.id, 
                if short_type.is_empty() { "Data" } else { short_type }
            );
            crate::channel_stats_labels::format_compressed_u128(e.stats_computer.capacity as u128, &mut tooltip);
            let _ = write!(tooltip, " | Vol: {} (Total: ", e.stats_computer.last_total);
            crate::channel_stats_labels::format_compressed_u128(e.stats_computer.total_consumed, &mut tooltip);
            let _ = write!(tooltip, ") | Color: {}\\n", e.color);

            ids.push(e.id);
            sum_saturation += e.saturation_score;
            sum_total += e.stats_computer.last_total;
            
            for &l in &e.ctl_labels {
                if !ctl_labels.contains(&l) {
                    ctl_labels.push(l);
                }
            }
        }
        
        let combined_type = type_list.join("/");

        let mut summary_label = first.display_label.clone();
        if is_partnered {
            summary_label = format!("{} [{}] (", first.partner.unwrap(), first.bundle_index.unwrap_or(0));
            for (i, cap) in sub_capacities.iter().enumerate() {
                if i > 0 { summary_label.push_str(", "); }
                crate::channel_stats_labels::format_compressed_u128(*cap as u128, &mut summary_label);
            }
            summary_label.push(')');

            if show_memory {
                summary_label.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(memory_footprint as u128, &mut summary_label);
                summary_label.push_str("B)");
            }
            summary_label.push_str("\\nTotal: ");

            for (i, total) in sub_totals.iter().enumerate() {
                if i > 0 { summary_label.push_str(", "); }
                crate::channel_stats_labels::format_compressed_u128(*total, &mut summary_label);
            }
            if !combined_type.is_empty() {
                summary_label.push_str("\\n");
                summary_label.push_str(&combined_type);
            }
        }

        partnered_edges.push(PartneredEdge {
            from: first.from,
            to: first.to,
            combined_color,
            summary_label,
            combined_type,
            partner_name: first.partner,
            sub_capacities,
            sidecar: first.sidecar,
            saturation_score: sum_saturation / edges.len() as f64,
            tooltip,
            sub_totals,
            last_total: sum_total,
            ids,
            ctl_labels,
            pen_width: if is_partnered { PARTNER_BUNDLE_PEN_WIDTH.to_string() } else { first.pen_width.clone() },
            memory_footprint,
            show_memory,
        });
    }

    // Stage 2: Aggregation
    let mut primary_groups: HashMap<PrimaryGroupKey, Vec<PartneredEdge>> = HashMap::new();
    for pe in partnered_edges {
        let key = PrimaryGroupKey {
            from_name: pe.from.map(|f| f.name),
            from_suffix: pe.from.and_then(|f| f.suffix),
            to_name: pe.to.map(|f| f.name),
            to_suffix: pe.to.and_then(|f| f.suffix),
            sub_capacities: pe.sub_capacities.clone(),
            type_name: pe.combined_type.clone(),
            color: pe.combined_color.clone(),
            sidecar: pe.sidecar,
            partner: pe.partner_name,
        };
        primary_groups.entry(key).or_default().push(pe);
    }

    // Sort keys to ensure deterministic DOT output
    let mut sorted_primary: Vec<_> = primary_groups.keys().collect();
    sorted_primary.sort();

    for p_key in sorted_primary {
        let edges = &primary_groups[p_key];
        
        if edges.len() < state.bundle_floor_size {
            for pe in edges {
                render_edge_internal(
                    dot_graph,
                    p_key.from_name.unwrap_or("unknown"),
                    p_key.from_suffix,
                    p_key.to_name.unwrap_or("unknown"),
                    p_key.to_suffix,
                    &pe.summary_label,
                    &pe.combined_color,
                    &pe.pen_width,
                    "",
                    p_key.sidecar,
                    "", "", &pe.tooltip
                );
            }
        } else {
            // Render as Bundle
            let n = edges.len();
            let sum_traffic: f64 = edges.iter().map(|e| e.saturation_score * e.sub_capacities.iter().sum::<usize>() as f64).sum();
            let bundle_capacity = (n as f64) * p_key.sub_capacities.iter().sum::<usize>() as f64;
            let _bundle_util = if bundle_capacity > 0.0 { (sum_traffic / bundle_capacity).clamp(0.0, 1.0) } else { 0.0 };
            
            // S-Tier: Element-wise summation of index-aligned totals
            let mut bundle_totals = vec![0u128; edges[0].sub_totals.len()];
            let mut total_memory = 0usize;
            let mut show_mem = false;
            for pe in edges {
                for (i, val) in pe.sub_totals.iter().enumerate() {
                    bundle_totals[i] += val;
                }
                total_memory += pe.memory_footprint;
                show_mem |= pe.show_memory;
            }

            let mut all_labels: Vec<&'static str> = edges.iter().flat_map(|e| e.ctl_labels.iter().cloned()).collect();
            all_labels.sort();
            all_labels.dedup();

            let label_prefix = p_key.partner.unwrap_or("Bundle");
            let mut header = format!("{}: {}x", label_prefix, n);
            if p_key.sub_capacities.len() > 1 {
                header.push('(');
                for (i, cap) in p_key.sub_capacities.iter().enumerate() {
                    if i > 0 { header.push_str(", "); }
                    crate::channel_stats_labels::format_compressed_u128(*cap as u128, &mut header);
                }
                header.push(')');
            } else {
                crate::channel_stats_labels::format_compressed_u128(p_key.sub_capacities[0] as u128, &mut header);
            }

            if show_mem {
                header.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(total_memory as u128, &mut header);
                header.push_str("B)");
            }
            header.push_str("\\nTotal: ");
            for (i, total) in bundle_totals.iter().enumerate() {
                if i > 0 { header.push_str(", "); }
                crate::channel_stats_labels::format_compressed_u128(*total, &mut header);
            }
            if !p_key.type_name.is_empty() {
                header.push_str("\\n");
                header.push_str(&p_key.type_name);
            }
            all_labels.iter().for_each(|l| {
                header.push(' ');
                header.push_str(l);
            });

            let mut bundle_tooltip = format!("Bundle Details ({} partnered edges):\\n", n);
            if p_key.sub_capacities.len() > 1 {
                bundle_tooltip.push_str("Capacities: (");
                for (i, cap) in p_key.sub_capacities.iter().enumerate() {
                    if i > 0 { bundle_tooltip.push_str(", "); }
                    crate::channel_stats_labels::format_compressed_u128(*cap as u128, &mut bundle_tooltip);
                }
                bundle_tooltip.push_str(")\\n");
            }
            for (i, e) in edges.iter().enumerate() {
                if i >= 10 {
                    let _ = write!(bundle_tooltip, "... and {} more", n - 10);
                    break;
                }
                let _ = write!(bundle_tooltip, "IDs={:?}: Vol={} ({}%)\\n", e.ids, e.last_total, (e.saturation_score * 100.0) as usize);
            }

            let is_partnered = p_key.partner.is_some();
            let pen_width = if is_partnered { PARTNER_BUNDLE_PEN_WIDTH } else { BUNDLE_PEN_WIDTH };

            render_edge_internal(
                dot_graph,
                p_key.from_name.unwrap_or("unknown"),
                p_key.from_suffix,
                p_key.to_name.unwrap_or("unknown"),
                p_key.to_suffix,
                &header,
                &p_key.color,
                pen_width,
                ", style=\"bold,dashed\"",
                p_key.sidecar,
                "", "", &bundle_tooltip
            );
        }
    }
    dot_graph.put_slice(b"}\n");
}

fn render_edge_internal(
    dot_graph: &mut BytesMut,
    from_name: &'static str,
    from_suffix: Option<usize>,
    to_name: &'static str,
    to_suffix: Option<usize>,
    label: &str,
    color: &str,
    pen_width: &str,
    style: &str,
    sidecar: bool,
    headlabel: &str,
    taillabel: &str,
    tooltip: &str,
) {
    dot_graph.put_slice(b"\"");
    dot_graph.put_slice(from_name.as_bytes());
    if let Some(s) = from_suffix {
        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
    }
    dot_graph.put_slice(b"\" -> \"");
    dot_graph.put_slice(to_name.as_bytes());
    if let Some(s) = to_suffix {
        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
    }
    dot_graph.put_slice(b"\" [label=\"");
    let escaped_label = label.replace('"', "'");
    dot_graph.put_slice(escaped_label.as_bytes());
    
    if !headlabel.is_empty() {
        dot_graph.put_slice(b"\", headlabel=\"");
        dot_graph.put_slice(headlabel.replace('"', "'").as_bytes());
    }
    if !taillabel.is_empty() {
        dot_graph.put_slice(b"\", taillabel=\"");
        dot_graph.put_slice(taillabel.replace('"', "'").as_bytes());
    }
    if !tooltip.is_empty() {
        dot_graph.put_slice(b"\", tooltip=\"");
        dot_graph.put_slice(tooltip.replace('"', "'").as_bytes());
    }

    dot_graph.put_slice(b"\", color=\"");
    dot_graph.put_slice(color.as_bytes());
    dot_graph.put_slice(b"\", penwidth=");
    dot_graph.put_slice(pen_width.as_bytes());
    dot_graph.put_slice(style.as_bytes());
    dot_graph.put_slice(b"];\n");

    if sidecar {
        dot_graph.put_slice(b"{rank=same; \"");
        dot_graph.put_slice(to_name.as_bytes());
        if let Some(s) = to_suffix {
            dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
        }
        dot_graph.put_slice(b"\" \"");
        dot_graph.put_slice(from_name.as_bytes());
        if let Some(s) = from_suffix {
            dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
        }
        dot_graph.put_slice(b"\"}\n");
    }
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
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: String::new(), // Defined when the content arrives
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                work_info: None
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
    local_state.nodes[id].tooltip = String::new();
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
/// * `node_name` - The name of the node.
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
                    pen_width: EDGE_PEN_WIDTH.to_string(),
                    saturation_score: 0.0,
                    display_label: String::new(), // Defined when the content arrives
                    metric_text: String::new(),
                    partner: None,
                    bundle_index: None,
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
                    edge.partner = meta.partner;
                    edge.bundle_index = meta.bundle_index;
            }
        }


    });
}

/// Represents the frame history for a graph, including packed data and output paths.
pub struct FrameHistory {
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
                core_exec::spawn_detached(async move {
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
    /// THE history file path.
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
    use crate::monitor::{ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData};
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
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            tooltip: String::new(),
            metric_text: String::new(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            work_info: None
        };
        node.compute_and_refresh(actor_status, total_work_ns);
        assert_eq!(node.color, "grey");
        assert_eq!(node.pen_width, NODE_PEN_WIDTH);
    }

    #[test]
    fn test_edge_compute_and_refresh() {
        let mut edge = Edge {
            id: 1,
            from: None,
            to: None,
            color: "grey",
            sidecar: false,
            pen_width: EDGE_PEN_WIDTH.to_string(),
            saturation_score: 0.0,
            ctl_labels: Vec::new(),
            stats_computer: ChannelStatsComputer::default(),
            display_label: String::new(),
            metric_text: String::new(),
            partner: None,
            bundle_index: None,
        };
        edge.compute_and_refresh(100, 50);
        assert_eq!(edge.color, "grey");
        assert!(!edge.pen_width.is_empty());
    }

    #[test]
    fn test_build_metric() {
        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(ActorName::new("1",None)),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: String::new(),
                    tooltip: String::new(),
                    metric_text: "node_metric".to_string(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: None,
                    to: None,
                    color: "grey",
                    sidecar: false,
                    pen_width: EDGE_PEN_WIDTH.to_string(),
                    saturation_score: 0.0,
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "edge_metric".to_string(),
                    metric_text: "edge_metric".to_string(),
                    partner: None,
                    bundle_index: None,
                }
            ],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "node1".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: None,
                    to: None,
                    color: "grey",
                    sidecar: false,
                    pen_width: EDGE_PEN_WIDTH.to_string(),
                    saturation_score: 0.0,
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "edge1".to_string(),
                    metric_text: String::new(),
                    partner: None,
                    bundle_index: None,
                }
            ],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();


        build_dot(&state, &mut dot_graph);
        let vec = dot_graph.to_vec();
        let result = String::from_utf8(vec).expect("Invalid UTF-8");
        
        assert!(result.contains("label=\"edge1\""),"found: {}",result);
        assert!(result.contains("tooltip=\"CH#1: Data | Cap: 0 | Vol: 0 (Total: 0) | Color: grey\\n\""),"found: {}",result);
        assert!(result.contains("color=\"grey\""),"found: {}",result);
    }

    #[test]
    fn test_frame_history_new() {
        let frame_history = FrameHistory::new(1000);
        assert_eq!(frame_history.packed_sent_writer.delta_write_count(), 0);
        assert_eq!(frame_history.packed_take_writer.delta_write_count(), 0);
        assert!(!frame_history.history_buffer.is_empty());
    }

    #[test]
    fn test_frame_history_apply_node() {
        let mut frame_history = FrameHistory::new(1000);
        let chin = vec![Arc::new(ChannelMetaData::default())];
        let chout = vec![Arc::new(ChannelMetaData::default())];
        frame_history.apply_node("node1", 1, &chin, &chout);
        assert!(!frame_history.history_buffer.is_empty());
    }

    #[test]
    fn test_frame_history_apply_edge() {
        let mut frame_history = FrameHistory::new(1000);
        let total_take_send = vec![(100, 50)];
        frame_history.apply_edge(&total_take_send, 1000);
        assert!(!frame_history.history_buffer.is_empty());
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
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            tooltip: String::new(),
            metric_text: String::new(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            work_info: None

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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: String::new(),
                    tooltip: String::new(),
                    metric_text: "node_metric".to_string(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: Some(ActorName::new("from_node", None)),
                    to: Some(ActorName::new("to_node", None)),
                    color: "grey",
                    sidecar: false,
                    pen_width: EDGE_PEN_WIDTH.to_string(),
                    saturation_score: 0.0,
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "test_edge".to_string(),
                    metric_text: "edge_metric".to_string(),
                    partner: None,
                    bundle_index: None,
                }
            ],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "node_with_suffix".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();


        build_dot(&state, &mut dot_graph);
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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "remote".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: Some(remote_details),
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();


        build_dot(&state, &mut dot_graph);
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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                },
                Node {
                    id: Some(ActorName::new("to_node", Some(5))),
                    color: "yellow",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![
                Edge {
                    id: 1,
                    from: Some(ActorName::new("from_node", None)),
                    to: Some(ActorName::new("to_node", Some(5))),
                    color: "purple",
                    sidecar: true, // Test sidecar functionality
                    pen_width: "4".to_string(),
                    saturation_score: 0.0,
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "test_edge".to_string(),
                    metric_text: String::new(),
                    partner: None,
                    bundle_index: None,
                }
            ],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();


        build_dot(&state, &mut dot_graph);
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
            show_thread_info: false,
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
            min_rate: false,
            max_rate: false,
            min_latency: false,
            max_latency: false,
            connects_sidecar: false,
            partner: None,
            bundle_index: None,
            type_byte_count: 0,
            show_total: false,
            girth: 1,
            show_memory: false,
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
            min_rate: false,
            max_rate: false,
            min_latency: false,
            max_latency: false,
            connects_sidecar: true,
            partner: None,
            bundle_index: None,
            type_byte_count: 0,
            show_total: false,
            girth: 1,
            show_memory: false,
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
            min_rate: false,
            max_rate: false,
            min_latency: false,
            max_latency: false,
            connects_sidecar: false,
            partner: None,
            bundle_index: None,
            type_byte_count: 0,
            show_total: false,
            girth: 1,
            show_memory: false,
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
        assert!(!frame_history.history_buffer.is_empty());
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
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "unknown_node".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None

                }
            ],
            edges: vec![],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();

        // This should not include the node since id is None (filtered out)
        build_dot(&state, &mut dot_graph);
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
                    pen_width: EDGE_PEN_WIDTH.to_string(),
                    saturation_score: 0.0,
                    ctl_labels: Vec::new(),
                    stats_computer: ChannelStatsComputer::default(),
                    display_label: "test_edge".to_string(),
                    metric_text: String::new(),
                    partner: None,
                    bundle_index: None,
                }
            ],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut dot_graph = BytesMut::new();


        // This should process the edge as "unknown" -> "unknown"
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        assert!(result.contains("test_edge"));
    }

    #[test]
    fn test_build_dot_aggregation() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        for i in 0..5 {
            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "4".to_string(),
                ctl_labels: vec!["test_label"],
                stats_computer: ChannelStatsComputer {
                    capacity: 10,
                    show_type: Some("TestType"),
                    type_byte_count: 8,
                    last_send: 100,
                    last_take: 90,
                    last_total: 10,
                    ..Default::default()
                },
                saturation_score: 0.1,
                display_label: format!("CH{} stats", i),
                metric_text: String::new(),
                partner: None,
                bundle_index: None,
            });
        }

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        assert!(result.contains("Bundle: 5x10\\nTotal: 0\\nTestType test_label"));
        assert!(result.contains("penwidth=4"));
        assert!(result.contains("style=\"bold,dashed\""));
        assert!(result.contains("tooltip=\"Bundle Details (5 partnered edges):\\nIDs=[0]: Vol=10 (10%)\\n"));
    }

    #[test]
    fn test_build_dot_complex_bundling() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        // 2 Red, 6 Grey
        for i in 0..8 {
            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: if i < 2 { "red" } else { "grey" },
                sidecar: false,
                pen_width: EDGE_PEN_WIDTH.to_string(),
                ctl_labels: vec!["shared"],
                stats_computer: ChannelStatsComputer {
                    capacity: 64,
                    show_type: Some("AttestationEvent"),
                    last_total: 25, // 8 * 25 = 200 total
                    ..Default::default()
                },
                saturation_score: 0.0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: None,
                bundle_index: None,
            });
        }

        let state = DotState {
            nodes: vec![],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        // Should have two bundles
        // Red group (2) is below threshold (4), so it stays discrete
        assert!(result.contains("color=\"red\"")); 
        // Grey group (6) is above threshold, so it bundles
        assert!(result.contains("Bundle: 6x64\\nTotal: 0\\nAttestationEvent shared"));
        assert!(result.contains("color=\"grey\""));
    }

    #[test]
    fn test_build_dot_no_aggregation_different_labels() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        for i in 0..5 {
            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "4".to_string(),
                ctl_labels: if i < 3 { vec!["A"] } else { vec!["B"] },
                stats_computer: ChannelStatsComputer {
                    capacity: 10,
                    type_byte_count: 4,
                    ..Default::default()
                },
                saturation_score: 0.0,
                display_label: format!("CH{} stats", i),
                metric_text: String::new(),
                partner: None,
                bundle_index: None,
            });
        }

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        // Neither group reaches the threshold of 4
        assert!(!result.contains("BUNDLE"));
    }

    #[test]
    fn test_build_dot_partnered_aggregation() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        for i in 0..5 {
            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: EDGE_PEN_WIDTH.to_string(),
                ctl_labels: vec!["test_label"],
                stats_computer: ChannelStatsComputer {
                    capacity: 10,
                    show_type: Some("TestType"),
                    type_byte_count: 8,
                    last_send: 100,
                    last_take: 90,
                    last_total: 10,
                    ..Default::default()
                },
                saturation_score: 0.1,
                display_label: format!("CH{} stats", i),
                metric_text: String::new(),
                partner: Some("MyPartner"),
                bundle_index: Some(0),
            });
        }

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    work_info: None
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        assert!(result.contains("penwidth=2"),"{}",result); // Partnered bundle width
    }

    #[test]
    fn test_build_dot_partnered_colors() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        // 2 edges with same partner and bundle index
        for i in 0..2 {
            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: if i == 0 { "red" } else { "blue" },
                sidecar: false,
                pen_width: EDGE_PEN_WIDTH.to_string(),
                ctl_labels: vec![],
                stats_computer: ChannelStatsComputer {
                    capacity: 10,
                    show_type: Some("TestType"),
                    ..Default::default()
                },
                saturation_score: 0.0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("MyPartner"),
                bundle_index: Some(0),
            });
        }

        let state = DotState {
            nodes: vec![],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        assert!(result.contains("color=\"red:black:blue\""));
        assert!(result.contains("penwidth=2"));
    }

    #[test]
    fn test_build_dot_partnered_bundle_label() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);
        
        let mut edges = Vec::new();
        // 5 partnerships, each with 2 channels (Cap 10 and Cap 100)
        for i in 0..5 {
            // Control channel
            edges.push(Edge {
                id: i * 2,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: EDGE_PEN_WIDTH.to_string(),
                ctl_labels: vec![],
                stats_computer: ChannelStatsComputer {
                    capacity: 10,
                    show_type: Some("Control"),
                    ..Default::default()
                },
                saturation_score: 0.0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("MyStream"),
                bundle_index: Some(i),
            });
            // Payload channel
            edges.push(Edge {
                id: i * 2 + 1,
                from: Some(from),
                to: Some(to),
                color: "blue",
                sidecar: false,
                pen_width: EDGE_PEN_WIDTH.to_string(),
                ctl_labels: vec![],
                stats_computer: ChannelStatsComputer {
                    capacity: 100,
                    show_type: Some("Payload"),
                    ..Default::default()
                },
                saturation_score: 0.0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("MyStream"),
                bundle_index: Some(i),
            });
        }

        let state = DotState {
            nodes: vec![],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut dot_graph = BytesMut::new();
        build_dot(&state, &mut dot_graph);
        let result = String::from_utf8(dot_graph.to_vec()).expect("internal error");
        
        assert!(result.contains("MyStream: 5x(10, 100)"), "{}", result);
        assert!(result.contains("Capacities: (10, 100)"), "{}", result);
    }
}
