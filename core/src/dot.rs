// This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

use log::*;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::fs::{OpenOptions, create_dir_all};
use std::path::PathBuf;

use bytes::{BufMut, BytesMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::ActorName;
use crate::actor_stats::ActorStatsComputer;
use crate::channel_stats::{ChannelStatsComputer, FilledVisualMode};
use crate::dot_edge::Edge;
use crate::dot_node::Node;
use crate::serialize::byte_buffer_packer::PackedVecWriter;
use crate::serialize::fast_protocol_packed::write_long_unsigned;
use crate::telemetry::metrics_server;
use crate::*;

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

#[derive(Default, Clone, Debug)]
pub struct RemoteDetails {
    pub(crate) ips: String,
    pub(crate) match_on: String,
    pub(crate) tech: &'static str,
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

/// Percent of border RGB mixed into actor node `fillcolor` (remainder is white). Tweak for a stronger or weaker tint.
const ACTOR_FILL_TINT_PERCENT: u32 = 12;

/// Max number of per-channel `Avg fill` values to print comma-separated; above this, labels use
/// a single `mean, N ch` line (aligns with large-bundle tooltips and Stage 2 bundle headers).
const MAX_INLINE_AVG_FILL_LANES: usize = 20;

/// Maps a color name to its RGB components.
fn color_to_rgb(color: &str) -> (u32, u32, u32) {
    match color {
        "red" => (255, 0, 0),
        "green" => (0, 169, 0),
        "blue" => (0, 0, 255),
        "grey" | "gray" => (128, 128, 128),
        "yellow" => (255, 255, 0),
        "purple" => (128, 0, 128),
        "white" => (255, 255, 255),
        _ => (0, 0, 0), // black/default
    }
}

/// Writes `#RRGGBB` into `out` (reused across DOT builds to avoid per-edge `String` churn).
fn rgb_to_hex_into(out: &mut String, r: u32, g: u32, b: u32) {
    out.clear();
    let _ = write!(
        out,
        "#{:02X}{:02X}{:02X}",
        r.min(255),
        g.min(255),
        b.min(255)
    );
}

/// Actor node interior: white blended with `border_color` so the fill reads solid on black backgrounds.
pub(crate) fn actor_fillcolor_hex_into(out: &mut String, border_color: &str) {
    if border_color.is_empty() {
        rgb_to_hex_into(out, 255, 255, 255);
        return;
    }
    let k = ACTOR_FILL_TINT_PERCENT.clamp(1, 99);
    let (r, g, b) = color_to_rgb(border_color);
    let blend = |c: u32| (255u32 * (100 - k) + c * k) / 100;
    rgb_to_hex_into(out, blend(r), blend(g), blend(b));
}

/// Single hex color: arithmetic mean of lane RGBs (DOT multi-lane / bundle rollup).
fn hex_color_average_into(out: &mut String, lane_rgbs: &[(u32, u32, u32)]) {
    if lane_rgbs.is_empty() {
        rgb_to_hex_into(out, 128, 128, 128);
        return;
    }
    let n = lane_rgbs.len() as u32;
    let r = lane_rgbs.iter().map(|(r, _, _)| *r).sum::<u32>() / n;
    let g = lane_rgbs.iter().map(|(_, g, _)| *g).sum::<u32>() / n;
    let b = lane_rgbs.iter().map(|(_, _, b)| *b).sum::<u32>() / n;
    rgb_to_hex_into(out, r, g, b);
}

/// Per-resolved-edge color name counts for tooltips (e.g. `Lane colors: 3 red, 120 grey`).
/// Reuses `counts` across calls to avoid allocating a new `BTreeMap` per line.
fn format_lane_color_histogram_into(
    counts: &mut BTreeMap<&'static str, usize>,
    out: &mut String,
    lane_colors: &[&'static str],
) {
    counts.clear();
    for c in lane_colors {
        *counts.entry(*c).or_insert(0) += 1;
    }
    out.clear();
    out.push_str("Lane colors: ");
    let mut first = true;
    for (name, n) in counts.iter() {
        if !first {
            out.push_str(", ");
        }
        first = false;
        let _ = write!(out, "{} {}", n, name);
    }
}

/// Mean whole-percent avg fill from channel edges; ignores lanes with no `Some` sample.
fn mean_avg_fill_from_edge_slice(edges: &[&Edge]) -> Option<u8> {
    let mut sum = 0u32;
    let mut count = 0u32;
    for e in edges {
        if let Some(n) = e.stats_computer.avg_filled_whole_percent() {
            sum += u32::from(n);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count) as u8)
}

/// Multi-lane `Avg fill` for DOT: comma list when `edges.len() <=` [`MAX_INLINE_AVG_FILL_LANES`], else
/// a single `mean, N ch` line (see module constant). Omits the line entirely when no lane has a sample (`None`).
fn format_avg_fill_rollup_line_into(out: &mut String, edges: &[&Edge]) {
    out.clear();
    if edges.is_empty() {
        return;
    }
    if edges.len() > MAX_INLINE_AVG_FILL_LANES {
        let n = edges.len();
        if let Some(m) = mean_avg_fill_from_edge_slice(edges) {
            let _ = write!(out, "Avg fill: {}% (mean, {} ch)\n", m, n);
        }
    } else {
        let mut started = false;
        for e in edges {
            if let Some(n) = e.stats_computer.avg_filled_whole_percent() {
                if !started {
                    out.push_str("Avg fill: ");
                    started = true;
                } else {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}%", n);
            }
        }
        if started {
            out.push('\n');
        }
    }
}

/// Integer mean of `Some` percent values; `None` if there are no samples.
fn mean_avg_fill_percent<'a, I: Iterator<Item = &'a Option<u8>>>(iter: I) -> Option<u8> {
    let mut sum = 0u32;
    let mut count = 0u32;
    for o in iter {
        if let Some(n) = o {
            sum += u32::from(*n);
            count += 1;
        }
    }
    (count > 0).then(|| (sum / count) as u8)
}

/// Per-channel hover line: rolling-window avg fill when enabled, else snapshot inflight/capacity.
fn append_channel_fill_tooltip(
    tooltip: &mut String,
    stats: &crate::channel_stats::ChannelStatsComputer,
    saturation_score: f64,
) {
    if stats.show_avg_filled {
        if let Some(n) = stats.avg_filled_whole_percent() {
            tooltip.push_str("\n ");
            let _ = write!(tooltip, "Avg fill: {}%\n", n);
        }
    } else {
        let _ = write!(
            tooltip,
            "\n Instant fill: {}%\n",
            (saturation_score * 100.0) as usize
        );
    }
}

#[inline]
fn escape_dot_quotes(out: &mut String, src: &str) {
    out.clear();
    out.reserve(src.len());
    for ch in src.chars() {
        if ch == '"' {
            out.push('\'');
        } else {
            out.push(ch);
        }
    }
}

#[inline]
fn escape_node_tooltip_text(out: &mut String, src: &str) {
    out.clear();
    out.reserve(src.len().saturating_mul(2));
    for ch in src.chars() {
        match ch {
            '"' => out.push('\''),
            '\n' => {
                out.push('\\');
                out.push('n');
            }
            _ => out.push(ch),
        }
    }
}

/// Builds the Prometheus metrics from the current state.
///
/// # Arguments
///
/// * `state` - THE current metric state.
/// * `txt_metric` - THE buffer to store the metrics text.
pub(crate) fn build_metric(state: &DotState, txt_metric: &mut BytesMut) {
    txt_metric.clear(); // Clear the buffer for reuse

    state
        .nodes
        .iter()
        .filter(|n| n.id.is_some())
        .for_each(|node| {
            txt_metric.put_slice(node.metric_text.as_bytes());
        });

    state
        .edges
        .iter()
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
    sidecar: bool,
    partner: Option<&'static str>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct PartnerKey {
    from: Option<(&'static str, Option<usize>)>,
    to: Option<(&'static str, Option<usize>)>,
    partner: Option<&'static str>,
    bundle_index: Option<usize>,
    /// Only used if partner is None to keep edges separate.
    edge_id: Option<usize>,
}

/// Working buffers for DOT + Prometheus + config JSON (reused each telemetry frame).
pub struct DotGraphFrames {
    pub(crate) active_metric: BytesMut,
    pub(crate) active_graph: BytesMut,
    /// Small JSON payload built without per-frame `String` allocation on the hot path.
    pub(crate) config_line: String,
    /// DOT escapes, rollups, histogram text (used sequentially; not nested).
    pub(crate) dot_scratch: String,
    /// `#RRGGBB` for the current edge render (separate from [`DotGraphFrames::dot_scratch`]).
    pub(crate) hex_line: String,
    pub(crate) lane_color_counts: BTreeMap<&'static str, usize>,
    pub(crate) last_generated_graph: Instant,
}

/// Builds the DOT graph from the current state.
///
/// # Arguments
///
/// * `state` - THE current metric state.
/// * `frames` - Working buffers including the DOT output (`active_graph`).
pub(crate) fn build_dot(state: &DotState, frames: &mut DotGraphFrames) {
    frames.active_graph.clear(); // Clear the buffer for reuse
    let dot_graph = &mut frames.active_graph;

    dot_graph.put_slice(b"digraph G {\nrankdir=");
    dot_graph.put_slice("LR".as_bytes());
    dot_graph.put_slice(b";\n");

    // Keep sidecars near with nodesep and ranksep spreads the rest out for label room.
    dot_graph.put_slice(b"graph [nodesep=.5, ranksep=2.5];\n");
    dot_graph.put_slice(b"node [margin=0.1];\n"); // Gap around text inside the circle

    dot_graph.put_slice(b"node [style=filled, fillcolor=white, fontcolor=black];\n");
    dot_graph.put_slice(b"edge [color=white, fontcolor=white];\n");
    dot_graph.put_slice(b"graph [bgcolor=black];\n");

    state
        .nodes
        .iter()
        .filter(|n| {
            // Only fully defined nodes, some may be in the process of being defined
            n.id.is_some()
        })
        .for_each(|node| {
            dot_graph.put_slice(b"\"");

            if let Some(f) = node.id {
                dot_graph.put_slice(f.name.as_bytes());
                if let Some(s) = f.suffix {
                    dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
                }
            } else {
                dot_graph.put_slice(b"No Name");
            }

            dot_graph.put_slice(b"\" [label=\"");
            dot_graph.put_slice(node.display_label.as_bytes());
            if !node.tooltip.is_empty() {
                dot_graph.put_slice(b"\", tooltip=\"");
                escape_node_tooltip_text(&mut frames.dot_scratch, &node.tooltip);
                dot_graph.put_slice(frames.dot_scratch.as_bytes());
            }
            if !node.color.is_empty() {
                dot_graph.put_slice(b"\", color=\"");
                dot_graph.put_slice(node.color.as_bytes());
            }
            actor_fillcolor_hex_into(
                &mut frames.hex_line,
                if node.color.is_empty() {
                    ""
                } else {
                    node.color
                },
            );
            dot_graph.put_slice(b"\", fillcolor=\"");
            dot_graph.put_slice(frames.hex_line.as_bytes());
            dot_graph.put_slice(b"\", penwidth=");
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
    let mut partner_groups: BTreeMap<PartnerKey, Vec<&Edge>> = BTreeMap::new();
    for edge in state
        .edges
        .iter()
        .filter(|e| e.id != usize::MAX && e.from.is_some() && e.to.is_some())
    {
        let key = PartnerKey {
            from: edge.from.map(|n| (n.name, n.suffix)),
            to: edge.to.map(|n| (n.name, n.suffix)),
            partner: edge.partner,
            bundle_index: edge.bundle_index,
            edge_id: if edge.partner.is_none() {
                Some(edge.id)
            } else {
                None
            },
        };
        partner_groups.entry(key).or_default().push(edge);
    }

    struct PartneredEdge {
        from: Option<ActorName>,
        to: Option<ActorName>,
        lane_rgbs: Vec<(u32, u32, u32)>,
        /// Resolved edge colors (after triggers), one per lane — for tooltip histograms.
        lane_colors: Vec<&'static str>,
        /// Whole-percent avg fill per lane (`None` if disabled or no window sample).
        avg_fill_per_lane: Vec<Option<u8>>,
        show_avg_filled_any: bool,
        summary_label: String,
        combined_type: String,
        partner_name: Option<&'static str>,
        sub_capacities: Vec<usize>,
        sidecar: bool,
        saturation_score: f64,
        tooltip: String,
        sub_totals: Vec<u128>,
        sum_total_consumed: u128,
        ids: Vec<usize>,
        ctl_labels: Vec<&'static str>,
        pen_width: String,
        memory_footprint: usize,
        show_memory: bool,
        show_total: bool,
    }

    let mut partnered_edges = Vec::new();
    for (_, mut edges) in partner_groups {
        // S-Tier: Sort edges by type name to ensure index-aligned vectors are stable across the bundle
        edges.sort_by_key(|e| e.stats_computer.show_type.unwrap_or(""));

        let first = edges[0];
        let is_partnered = first.partner.is_some();
        let mut lane_rgbs = Vec::new();
        let mut type_list = Vec::new();
        let mut ctl_labels = Vec::new();
        let mut ids = Vec::new();
        let mut tooltip = String::new();

        // Add Window info to the top of the tooltip if active
        if !first.stats_computer.time_label.is_empty() {
            let _ = write!(tooltip, "Window: {}\n", first.stats_computer.time_label);
        }

        let mut sum_saturation = 0.0;
        let mut sub_capacities = Vec::with_capacity(edges.len());
        let mut sub_totals = Vec::with_capacity(edges.len());
        let mut sum_total_consumed = 0u128;
        let mut memory_footprint = 0;
        let mut show_memory = false;
        let mut lane_colors = Vec::with_capacity(edges.len());
        let mut avg_fill_per_lane = Vec::with_capacity(edges.len());

        let is_large_bundle = edges.len() > MAX_INLINE_AVG_FILL_LANES;
        let show_avg_filled_any = edges.iter().any(|e| e.stats_computer.show_avg_filled);

        for e in edges.iter() {
            lane_rgbs.push(color_to_rgb(e.color));
            lane_colors.push(e.color);
            avg_fill_per_lane.push(e.stats_computer.avg_filled_whole_percent());

            let short_type = e.stats_computer.show_type.unwrap_or("");
            if !short_type.is_empty() {
                type_list.push(short_type);
            }
            sub_capacities.push(e.stats_computer.capacity);
            sub_totals.push(e.stats_computer.total_consumed);
            memory_footprint += e.stats_computer.memory_footprint;
            show_memory |= e.stats_computer.show_memory;

            if !is_large_bundle {
                let _ = write!(
                    tooltip,
                    "CH#{}: {}\n",
                    e.id,
                    if short_type.is_empty() {
                        "Data"
                    } else {
                        short_type
                    }
                );
                tooltip.push_str(" Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    e.stats_computer.capacity as u128,
                    &mut tooltip,
                );
                // FIX: Show Total (cumulative) on tooltip to match edge label
                tooltip.push_str("\n Total: ");
                crate::channel_stats_labels::format_compressed_u128(
                    e.stats_computer.total_consumed,
                    &mut tooltip,
                );
                append_channel_fill_tooltip(&mut tooltip, &e.stats_computer, e.saturation_score);
            }

            ids.push(e.id);
            sum_saturation += e.saturation_score;
            sum_total_consumed += e.stats_computer.total_consumed;

            for &l in &e.ctl_labels {
                if !ctl_labels.contains(&l) {
                    ctl_labels.push(l);
                }
            }
        }

        if !is_large_bundle && !lane_colors.is_empty() {
            format_lane_color_histogram_into(
                &mut frames.lane_color_counts,
                &mut frames.dot_scratch,
                &lane_colors,
            );
            let _ = write!(tooltip, "{}\n", frames.dot_scratch);
        }

        if is_large_bundle {
            let _ = write!(tooltip, "Summary: {} channels\n", edges.len());

            // Total volume across all channels
            let _ = write!(tooltip, "Total: ");
            crate::channel_stats_labels::format_compressed_u128(
                sum_total_consumed,
                &mut tooltip,
            );
            tooltip.push('\n');

            // Capacity range
            let cap_min = sub_capacities.iter().copied().min().unwrap_or(0);
            let cap_max = sub_capacities.iter().copied().max().unwrap_or(0);
            if cap_min == cap_max {
                let _ = write!(tooltip, "Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_min as u128,
                    &mut tooltip,
                );
                tooltip.push('\n');
            } else {
                let _ = write!(tooltip, "Capacity: ");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_min as u128,
                    &mut tooltip,
                );
                tooltip.push_str("–");
                crate::channel_stats_labels::format_compressed_u128(
                    cap_max as u128,
                    &mut tooltip,
                );
                tooltip.push('\n');
            }

            // Mean avg fill
            if let Some(m) = mean_avg_fill_percent(avg_fill_per_lane.iter()) {
                let _ = write!(tooltip, "Avg fill: {}% (mean)\n", m);
            }

            // Mean saturation
            let mean_sat = (sum_saturation / edges.len() as f64 * 100.0) as usize;
            let _ = write!(tooltip, "Saturation: {}% (mean)\n", mean_sat);

            // Memory footprint
            if show_memory {
                tooltip.push_str("Memory: ");
                crate::channel_stats_labels::format_compressed_u128(
                    memory_footprint as u128,
                    &mut tooltip,
                );
                tooltip.push_str("B\n");
            }

            // Lane color histogram
            if !lane_colors.is_empty() {
                format_lane_color_histogram_into(
                    &mut frames.lane_color_counts,
                    &mut frames.dot_scratch,
                    &lane_colors,
                );
                let _ = write!(tooltip, "{}\n", frames.dot_scratch);
            }
        }

        let combined_type = type_list.join("/");

        // Multi-lane avg fill: rebuild first lane's label without "Avg filled" line, then append comma rollup.
        let mut summary_label = if edges.len() > 1 && show_avg_filled_any {
            let mut body = String::new();
            let mut dummy_metric = String::new();
            first.stats_computer.append_visual_metric_lines(
                &mut body,
                &mut dummy_metric,
                FilledVisualMode::SuppressAvgOnly,
            );
            format_avg_fill_rollup_line_into(&mut frames.dot_scratch, &edges);
            body.push_str(&frames.dot_scratch);
            body
        } else {
            first.display_label.clone()
        };

        // CRITICAL: For partnered edges, we must preserve the display_label which contains the
        // rate/filled metrics computed by ChannelStatsComputer. The partner info is prepended
        // to the existing label rather than replacing it entirely.
        if is_partnered {
            // Prepend partner identifier to the existing label, don't replace it
            let partner_header = format!(
                "{} [{}]",
                first.partner.unwrap(),
                first.bundle_index.unwrap_or(0)
            );

            // Build partner info line with memory if needed
            let mut partner_info = partner_header;
            if show_memory {
                partner_info.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(
                    memory_footprint as u128,
                    &mut partner_info,
                );
                partner_info.push_str("B)");
            }

            // Combine: Partner info first, then original label (which contains rate/fill from ChannelStatsComputer)
            // This ensures avg_rate and other dynamic metrics are preserved on the label
            summary_label = format!("{}\n{}", partner_info, summary_label);
        }

        // FIX: Always show the total(s) on the edge label itself, not just in the tooltip.
        // For bundled edges that will be rendered individually (len < bundle_floor_size),
        // show all totals separated by commas. For single edges, show the total directly.
        // For large bundles (len >= bundle_floor_size), totals are handled in the bundle rendering section below.
        if first.stats_computer.show_total {
            let mut total_label = String::new();
            if edges.len() == 1 {
                // Single edge: show total directly
                total_label.push_str("Total: ");
                crate::channel_stats_labels::format_compressed_u128(
                    first.stats_computer.total_consumed,
                    &mut total_label,
                );
            } else if edges.len() < state.bundle_floor_size {
                // Bundled edges (but not large enough to be rendered as bundle): show all totals separated by commas
                total_label.push_str("Totals: ");
                for (i, total) in sub_totals.iter().enumerate() {
                    if i > 0 {
                        total_label.push_str(", ");
                    }
                    crate::channel_stats_labels::format_compressed_u128(*total, &mut total_label);
                }
            }
            // For large bundles (len >= bundle_floor_size), the totals are handled in the bundle rendering section below
            if !total_label.is_empty() {
                summary_label = format!("{}{}\n", summary_label, total_label);
            }
        }

        partnered_edges.push(PartneredEdge {
            from: first.from,
            to: first.to,
            lane_rgbs,
            lane_colors,
            avg_fill_per_lane,
            show_avg_filled_any,
            summary_label,
            combined_type,
            partner_name: first.partner,
            sub_capacities,
            sidecar: first.sidecar,
            saturation_score: sum_saturation / edges.len() as f64,
            tooltip,
            sub_totals,
            sum_total_consumed,
            ids,
            ctl_labels,
            pen_width: if is_partnered {
                PARTNER_BUNDLE_PEN_WIDTH.to_string()
            } else {
                first.pen_width.clone()
            },
            memory_footprint,
            show_memory,
            show_total: first.stats_computer.show_total,
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
                hex_color_average_into(&mut frames.hex_line, &pe.lane_rgbs);

                render_edge_internal(
                    dot_graph,
                    p_key.from_name.unwrap_or("unknown"),
                    p_key.from_suffix,
                    p_key.to_name.unwrap_or("unknown"),
                    p_key.to_suffix,
                    &pe.summary_label,
                    &frames.hex_line,
                    &pe.pen_width,
                    "",
                    p_key.sidecar,
                    "",
                    "",
                    &pe.tooltip,
                    &mut frames.dot_scratch,
                );
            }
        } else {
            // Render as Bundle
            let n = edges.len();
            let total_channels: usize = edges.iter().map(|e| e.ids.len()).sum();
            let sum_traffic: f64 = edges
                .iter()
                .map(|e| e.saturation_score * e.sub_capacities.iter().sum::<usize>() as f64)
                .sum();
            let bundle_capacity = (n as f64) * p_key.sub_capacities.iter().sum::<usize>() as f64;
            let _bundle_util = if bundle_capacity > 0.0 {
                (sum_traffic / bundle_capacity).clamp(0.0, 1.0)
            } else {
                0.0
            };

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

            let mut all_labels: Vec<&'static str> = edges
                .iter()
                .flat_map(|e| e.ctl_labels.iter().cloned())
                .collect();
            all_labels.sort();
            all_labels.dedup();

            let label_prefix = p_key.partner.unwrap_or("Bundle");
            let mut header = format!("{}: {}x", label_prefix, n);

            if show_mem {
                header.push_str(" (");
                crate::channel_stats_labels::format_compressed_u128(
                    total_memory as u128,
                    &mut header,
                );
                header.push_str("B)");
            }
            // FIX: Show comma-separated totals for each partner type in the bundle, not a single aggregated total
            if edges[0].show_total {
                header.push_str("\nTotals: ");
                for (i, total) in bundle_totals.iter().enumerate() {
                    if i > 0 {
                        header.push_str(", ");
                    }
                    crate::channel_stats_labels::format_compressed_u128(*total, &mut header);
                }
            }
            if edges.iter().any(|pe| pe.show_avg_filled_any) {
                frames.dot_scratch.clear();
                if total_channels > MAX_INLINE_AVG_FILL_LANES {
                    if let Some(m) = mean_avg_fill_percent(
                        edges
                            .iter()
                            .flat_map(|pe| pe.avg_fill_per_lane.iter()),
                    ) {
                        let _ = write!(
                            frames.dot_scratch,
                            "{}% (mean, {} ch)",
                            m, total_channels
                        );
                    }
                } else {
                    let mut started = false;
                    for o in edges.iter().flat_map(|pe| pe.avg_fill_per_lane.iter()) {
                        if let Some(n) = o {
                            if !started {
                                started = true;
                            } else {
                                frames.dot_scratch.push_str(", ");
                            }
                            let _ = write!(frames.dot_scratch, "{}%", n);
                        }
                    }
                }
                if !frames.dot_scratch.is_empty() {
                    header.push_str("\nAvg fill: ");
                    header.push_str(&frames.dot_scratch);
                }
            }
            if !p_key.type_name.is_empty() {
                header.push_str("\n");
                header.push_str(&p_key.type_name);
            }
            all_labels.iter().for_each(|l| {
                header.push(' ');
                header.push_str(l);
            });

            let mut bundle_tooltip = format!("Bundle ({} chans in {} groups):", total_channels, n);
            // Add Window info to the top of the bundle tooltip
            if !edges[0].tooltip.is_empty() && edges[0].tooltip.starts_with("Window:") {
                if let Some(first_line) = edges[0].tooltip.split("\\n").next() {
                    bundle_tooltip.push_str("\\n");
                    bundle_tooltip.push_str(first_line);
                }
            }

            if total_channels > MAX_INLINE_AVG_FILL_LANES {
                // Large bundle tooltip - show summary, but no total volume or avg saturation
                let _ = write!(bundle_tooltip, "\\nSummary: {} channels", total_channels);
            } else {
                if p_key.sub_capacities.len() > 1 {
                    bundle_tooltip.push_str("\\n Capacities: (");
                    for (i, cap) in p_key.sub_capacities.iter().enumerate() {
                        if i > 0 {
                            bundle_tooltip.push_str(", ");
                        }
                        crate::channel_stats_labels::format_compressed_u128(
                            *cap as u128,
                            &mut bundle_tooltip,
                        );
                    }
                    bundle_tooltip.push_str(")");
                }
                for e in edges.iter() {
                    // Skip the Window line if it was already added to the bundle header
                    let entry_tooltip = if e.tooltip.starts_with("Window:") {
                        e.tooltip.splitn(2, "\\n").nth(1).unwrap_or(&e.tooltip)
                    } else {
                        &e.tooltip
                    };
                    bundle_tooltip.push_str("\\n");
                    bundle_tooltip.push_str(entry_tooltip);
                }
            }

            let flat_lane_colors: Vec<&'static str> = edges
                .iter()
                .flat_map(|pe| pe.lane_colors.iter().copied())
                .collect();
            if !flat_lane_colors.is_empty() {
                format_lane_color_histogram_into(
                    &mut frames.lane_color_counts,
                    &mut frames.dot_scratch,
                    &flat_lane_colors,
                );
                let _ = write!(bundle_tooltip, "\\n{}", frames.dot_scratch);
            }

            let is_partnered = p_key.partner.is_some();
            let pen_width = if is_partnered {
                PARTNER_BUNDLE_PEN_WIDTH
            } else {
                BUNDLE_PEN_WIDTH
            };

            let all_rgbs: Vec<(u32, u32, u32)> = edges
                .iter()
                .flat_map(|pe| pe.lane_rgbs.iter().cloned())
                .collect();
            hex_color_average_into(&mut frames.hex_line, &all_rgbs);

            render_edge_internal(
                dot_graph,
                p_key.from_name.unwrap_or("unknown"),
                p_key.from_suffix,
                p_key.to_name.unwrap_or("unknown"),
                p_key.to_suffix,
                &header,
                &frames.hex_line,
                pen_width,
                ", style=\"bold,dashed\"",
                p_key.sidecar,
                "",
                "",
                &bundle_tooltip,
                &mut frames.dot_scratch,
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
    escape_buf: &mut String,
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
    escape_dot_quotes(escape_buf, label);
    dot_graph.put_slice(escape_buf.as_bytes());

    if !headlabel.is_empty() {
        dot_graph.put_slice(b"\", headlabel=\"");
        escape_dot_quotes(escape_buf, headlabel);
        dot_graph.put_slice(escape_buf.as_bytes());
    }
    if !taillabel.is_empty() {
        dot_graph.put_slice(b"\", taillabel=\"");
        escape_dot_quotes(escape_buf, taillabel);
        dot_graph.put_slice(escape_buf.as_bytes());
    }
    if !tooltip.is_empty() {
        dot_graph.put_slice(b"\", tooltip=\"");
        escape_dot_quotes(escape_buf, tooltip);
        dot_graph.put_slice(escape_buf.as_bytes());

        // NOTE: We intentionally do NOT add labeltooltip here. The tooltip attribute is sufficient
        // for hover information. Adding labeltooltip was causing flickering and black lines in the
        // graph visualization, likely due to parsing issues when the tooltip contains newlines.
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

/// Applies the node definition to the local state.
///
/// # Arguments
///
/// * `local_state` - THE local metric state.
/// * `actor` - THE metadata of the actor.
/// * `channels_in` - THE input channels.
/// * `channels_out` - THE output channels.
/// * `frame_rate_ms` - THE frame rate in milliseconds.
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
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                work_info: None,
            }
        });
    }
    local_state.nodes[id].id = Some(actor.ident.label);
    local_state.nodes[id].remote_details = actor.remote_details.clone();
    local_state.nodes[id].display_label = if let Some(suf) = actor.ident.label.suffix {
        format!("{}{}\n", actor.ident.label.name, suf)
    } else {
        format!("{}\n", actor.ident.label.name)
    };
    local_state.nodes[id].tooltip = String::new();
    local_state.nodes[id]
        .stats_computer
        .init(actor.clone(), frame_rate_ms);

    // Edges are defined by both the sender and the receiver
    // We need to record both monitors in this edge as to and from
    // actor.ident.id.label
    define_unified_edges(
        local_state,
        actor.ident.label,
        channels_in,
        true,
        frame_rate_ms,
    );
    define_unified_edges(
        local_state,
        actor.ident.label,
        channels_out,
        false,
        frame_rate_ms,
    );
}

/// Defines unified edges in the local state.
///
/// # Arguments
///
/// * `local_state` - THE local metric state.
/// * `node_name` - THE name of the node.
/// * `mdvec` - THE metadata of the channels.
/// * `set_to` - A boolean indicating if the edges are set to the node.
/// * `frame_rate_ms` - THE frame rate in milliseconds.
fn define_unified_edges(
    local_state: &mut DotState,
    node_name: ActorName,
    mdvec: &[Arc<ChannelMetaData>],
    set_to: bool,
    frame_rate_ms: u64,
) {
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
                if !Some(node_name).eq(&edge.to) {
                    warn!(
                        "internal error: edge.to was {:?} but now {:?}",
                        edge.to.expect("to"),
                        node_name
                    );
                }
            }
        } else if edge.from.is_none() {
            edge.from = Some(node_name);
        } else {
            if !Some(node_name).eq(&edge.from) {
                warn!(
                    "internal error: edge.from was {:?} but now {:?}",
                    edge.from.expect("from"),
                    node_name
                );
            }
        }

        // Collect the labels that need to be added
        // This is redundant but provides safety if two different label lists are in play
        let labels_to_add: Vec<_> = meta
            .labels
            .iter()
            .filter(|f| !edge.ctl_labels.contains(f))
            .cloned() // Clone the items to be added
            .collect();
        // Now, append them to `ctl_labels`
        for label in labels_to_add {
            edge.ctl_labels.push(label);
        }

        if let Some(node_from) = edge.from {
            if let Some(node_to) = edge.to {
                if edge.stats_computer.capacity == 0 {
                    edge.stats_computer
                        .init(meta, node_from, node_to, frame_rate_ms);
                }
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
        let mut buf = BytesMut::with_capacity(HISTORY_WRITE_BLOCK_SIZE * 2);

        //set history file header with key information about our run
        //
        //what time did we start
        let now: u64 = OffsetDateTime::now_utc().unix_timestamp() as u64;
        write_long_unsigned(now, &mut buf);
        //
        //set history file header with key information about our frame rate
        write_long_unsigned(ms_rate, &mut buf); //time between frames

        let mut runtime_config = 0;

        #[cfg(test)]
        {
            runtime_config |= 1;
        } // ones bit is ether test(1) or release(0)

        #[cfg(feature = "prometheus_metrics")]
        {
            runtime_config |= 2;
        } // twos bit is ether prometheus(1) or none(0)

        #[cfg(feature = "proactor_tokio")]
        {
            runtime_config |= 4;
        } // fours bit is ether tokio(1) or nuclei(0)

        #[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
        {
            runtime_config |= 8;
        } // eights bit is ether telemetry(1) or none(0)

        #[cfg(feature = "telemetry_server_cdn")]
        {
            runtime_config |= 16;
        } // sixteenth bit is ether cdn(1) or builtin(0)

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
    /// * `name` - THE name of the node.
    /// * `id` - THE ID of the node.
    /// * `chin` - THE input channels.
    /// * `chout` - THE output channels.
    pub fn apply_node(
        &mut self,
        name: &'static str,
        id: usize,
        chin: &[Arc<ChannelMetaData>],
        chout: &[Arc<ChannelMetaData>],
    ) {
        write_long_unsigned(REC_NODE, &mut self.history_buffer); // Message type
        write_long_unsigned(id as u64, &mut self.history_buffer); // Message type

        write_long_unsigned(name.len() as u64, &mut self.history_buffer);
        self.history_buffer
            .write_str(name)
            .expect("internal error writing to ByteMut");

        write_long_unsigned(chin.len() as u64, &mut self.history_buffer);
        chin.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer
                    .write_str(s)
                    .expect("internal error writing to ByteMut");
            });
        });

        write_long_unsigned(chout.len() as u64, &mut self.history_buffer);
        chout.iter().for_each(|meta| {
            write_long_unsigned(meta.id as u64, &mut self.history_buffer);
            write_long_unsigned(meta.labels.len() as u64, &mut self.history_buffer);
            meta.labels.iter().for_each(|s| {
                write_long_unsigned(s.len() as u64, &mut self.history_buffer);
                self.history_buffer
                    .write_str(s)
                    .expect("internal error writing to ByteMut");
            });
        });
    }

    /// Applies an edge definition to the history buffer.
    ///
    /// # Arguments
    ///
    /// * `total_take_send` - THE total take and send values.
    /// * `frame_rate_ms` - THE frame rate in milliseconds.
    pub fn apply_edge(&mut self, total_take_send: &[(i64, i64)], frame_rate_ms: u64) {
        write_long_unsigned(REC_EDGE, &mut self.history_buffer); // Message type

        let total_take: Vec<i64> = total_take_send.iter().map(|(t, _)| *t).collect();
        let total_send: Vec<i64> = total_take_send.iter().map(|(_, s)| *s).collect();

        if (self.packed_sent_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000)
        {
            self.packed_sent_writer
                .add_vec(&mut self.history_buffer, &total_send);
        } else {
            self.packed_sent_writer.sync_data();
            self.packed_sent_writer
                .add_vec(&mut self.history_buffer, &total_send);
        };

        if (self.packed_take_writer.delta_write_count() * frame_rate_ms as usize) < (10 * 60 * 1000)
        {
            self.packed_take_writer
                .add_vec(&mut self.history_buffer, &total_take);
        } else {
            self.packed_take_writer.sync_data();
            self.packed_take_writer
                .add_vec(&mut self.history_buffer, &total_take);
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

        if (flush_all || self.will_span_into_next_block())
            && (cur_bytes_written != self.local_thread_bytes_cache || 0 == cur_bytes_written)
        {
            // Store this and do not run again until this has changed
            self.local_thread_bytes_cache = cur_bytes_written;

            let buf_bytes_count = self.buffer_bytes_count;
            let continued_buffer: BytesMut = self.history_buffer.split_off(buf_bytes_count);
            // trace!("attempt to write history");
            let to_be_written =
                std::mem::replace(&mut self.history_buffer, continued_buffer).to_owned();
            if !to_be_written.is_empty() {
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
        let file_to_append_onto = format!(
            "{}_{}_log.dat",
            log_time
                .format(&format)
                .unwrap_or_else(|_| "0000_00_00".to_string()),
            self.guid
        );

        // If we are starting a new file reset our counter to zero
        if !self.last_file_to_append_onto.eq(&file_to_append_onto) {
            self.file_bytes_written.store(0, Ordering::SeqCst);
            self.last_file_to_append_onto
                .clone_from(&file_to_append_onto);
        }
        self.output_log_path.join(&file_to_append_onto)
    }

    /// Checks if the next block will span into the next file write block.
    ///
    /// # Returns
    ///
    /// `true` if the next block will span into the next file write block, `false` otherwise.
    fn will_span_into_next_block(&self) -> bool {
        let old_blocks = (self.file_bytes_written.load(Ordering::SeqCst) + self.buffer_bytes_count)
            / HISTORY_WRITE_BLOCK_SIZE;
        let new_blocks = (self.file_bytes_written.load(Ordering::SeqCst)
            + self.history_buffer.len())
            / HISTORY_WRITE_BLOCK_SIZE;
        new_blocks > old_blocks
    }

    /// Truncates the file at the given path and writes the provided data to it.
    ///
    /// # Arguments
    ///
    /// * `path` - THE file path.
    /// * `data` - THE data to write.
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
    async fn append_to_file(
        path: PathBuf,
        data: BytesMut,
        flush: bool,
    ) -> Result<(), std::io::Error> {
        let file = OpenOptions::new().append(true).create(true).open(&path)?;
        metrics_server::async_write_all(data, flush, file).await
    }
}

#[cfg(test)]
mod dot_tests {
    use super::*;
    use crate::monitor::{ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData};
    use crate::telemetry::metrics_server::async_write_all;
    use bytes::BytesMut;
    use std::fs::remove_file;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Instant;

    fn test_dot_frames() -> DotGraphFrames {
        DotGraphFrames {
            active_metric: BytesMut::new(),
            active_graph: BytesMut::new(),
            config_line: String::new(),
            dot_scratch: String::new(),
            hex_line: String::new(),
            lane_color_counts: std::collections::BTreeMap::new(),
            last_generated_graph: Instant::now(),
        }
    }

    /// `bool::then_some(x)` evaluates `x` eagerly; mean must use `then` so empty iterators never divide.
    #[test]
    fn test_mean_avg_fill_percent_all_none_returns_none_without_panic() {
        let all_none = [None::<u8>, None];
        assert_eq!(mean_avg_fill_percent(all_none.iter()), None);
        let empty: [Option<u8>; 0] = [];
        assert_eq!(mean_avg_fill_percent(empty.iter()), None);
    }

    #[test]
    fn test_node_compute_and_refresh() {
        let actor_status = ActorStatus {
            ident: Default::default(),
            await_total_ns: 100,
            unit_total_ns: 200,
            total_count_restarts: 1,
            iteration_start: 0,
            iteration_sum: 0,
            bool_stop: false,
            is_quiet: false,
            calls: [0; 6],
            thread_info: None,
            bool_blocking: false,
        };
        let mut node = Node {
            id: Some(ActorName::new("1", None)),
            color: "grey",
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(), // Defined when the content arrives
            dot_subtitle: None,
            tooltip: String::new(),
            metric_text: String::new(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            bool_stalled: false,
            work_info: None,
        };
        node.compute_and_refresh(actor_status);
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
            display_label: String::new(), // Defined when the content arrives
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
            nodes: vec![Node {
                id: Some(ActorName::new("1", None)),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: String::new(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: "node_metric".to_string(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                work_info: None,
            }],
            edges: vec![Edge {
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
            }],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };
        let mut txt_metric = BytesMut::new();
        build_metric(&state, &mut txt_metric);
    }

    #[test]
    fn test_build_dot() {
        let state = DotState {
            nodes: vec![Node {
                id: Some(ActorName::new("1", None)),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "node1".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                work_info: None,
            }],
            edges: vec![Edge {
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
            }],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };
        let mut frames = test_dot_frames();

        build_dot(&state, &mut frames);
        let vec = frames.active_graph.to_vec();
        let result = String::from_utf8(vec).expect("Invalid UTF-8");

        // Updated: Check for the actual output format - node ID is "1" not "1 "
        assert!(
            result.contains("\"1\" [label=\"node1\""),
            "found: {}",
            result
        );
        assert!(result.contains("color=\"grey\""), "found: {}", result);
        assert!(
            result.contains("fillcolor=\"#EFEFEF\""),
            "expected tinted white fill for grey border, found: {}",
            result
        );
    }

    #[test]
    fn test_actor_fillcolor_hex_into() {
        let mut s = String::new();
        actor_fillcolor_hex_into(&mut s, "");
        assert_eq!(s, "#FFFFFF");

        actor_fillcolor_hex_into(&mut s, "grey");
        assert_eq!(s, "#EFEFEF");

        actor_fillcolor_hex_into(&mut s, "green");
        assert_eq!(s, "#E0F4E0");

        actor_fillcolor_hex_into(&mut s, "red");
        assert_eq!(s, "#FFE0E0");
    }

    // ============================================================================
    // ROLLUP VERIFICATION TESTS - These tests verify that the total_consumed
    // is correctly displayed on both edge labels AND tooltips.
    // ============================================================================

    /// Multi-lane partner group: comma-separated whole-percent avg fill and lane color histogram.
    #[test]
    fn test_multi_lane_avg_fill_rollup_and_lane_color_histogram() {
        use crate::actor_stats::ChannelBlock;

        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut lane0 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        lane0.current_filled = Some(ChannelBlock {
            histogram: None,
            runner: 10_000,
            sum_of_squares: 0,
        });

        let mut lane1 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        lane1.current_filled = Some(ChannelBlock {
            histogram: None,
            runner: 40_000,
            sum_of_squares: 0,
        });

        let edges = vec![
            Edge {
                id: 0,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.1,
                ctl_labels: vec![],
                stats_computer: lane0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
            Edge {
                id: 1,
                from: Some(from),
                to: Some(to),
                color: "red",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.4,
                ctl_labels: vec![],
                stats_computer: lane1,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
        ];

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            result.contains("Avg fill: 10%, 40%"),
            "expected rollup line: {}",
            result
        );
        assert!(
            result.contains("Lane colors: 1 green, 1 red"),
            "expected histogram: {}",
            result
        );
    }

    /// Merged bundle edge (`n` groups ≥ `bundle_floor_size`, `total_channels` > `MAX_INLINE_AVG_FILL_LANES`) must not list
    /// one percent per channel on the label; use a single mean line instead.
    #[test]
    fn test_large_bundle_avg_fill_uses_mean_summary() {
        use crate::actor_stats::ChannelBlock;

        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut edges: Vec<Edge> = Vec::new();
        let mut id: usize = 0;
        for bi in 0..8 {
            for type_s in ["A", "B", "C"] {
                let mut stats = ChannelStatsComputer {
                    capacity: 100,
                    show_avg_filled: true,
                    show_type: Some(type_s),
                    refresh_rate_in_bits: 0,
                    window_bucket_in_bits: 0,
                    ..Default::default()
                };
                stats.current_filled = Some(ChannelBlock {
                    histogram: None,
                    runner: 5_000,
                    sum_of_squares: 0,
                });

                edges.push(Edge {
                    id,
                    from: Some(from),
                    to: Some(to),
                    color: "green",
                    sidecar: false,
                    pen_width: "1".to_string(),
                    saturation_score: 0.0,
                    ctl_labels: vec![],
                    stats_computer: stats,
                    display_label: String::new(),
                    metric_text: String::new(),
                    partner: Some("P"),
                    bundle_index: Some(bi),
                });
                id += 1;
            }
        }

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 2,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            result.contains("Avg fill: 5% (mean, 24 ch)"),
            "expected mean summary on bundle label, got output starting: {}",
            &result[..result.len().min(500)]
        );
        assert!(
            !result.contains("0%, 0%, 0%, 0%, 0%, 0%"),
            "label should not contain a long comma-separated fill list"
        );
    }

    /// One partner group with 21 parallel lanes: Stage 1 `Avg fill` must use the mean line (not 21 commas).
    #[test]
    fn test_stage1_avg_fill_mean_when_lanes_exceed_inline_cap() {
        use crate::actor_stats::ChannelBlock;

        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut edges: Vec<Edge> = Vec::new();
        for i in 0..21 {
            let mut stats = ChannelStatsComputer {
                capacity: 100,
                show_avg_filled: true,
                show_type: Some("T"),
                refresh_rate_in_bits: 0,
                window_bucket_in_bits: 0,
                ..Default::default()
            };
            stats.current_filled = Some(ChannelBlock {
                histogram: None,
                runner: 5_000,
                sum_of_squares: 0,
            });

            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.0,
                ctl_labels: vec![],
                stats_computer: stats,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("Q"),
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
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            result.contains("Avg fill: 5% (mean, 21 ch)"),
            "expected Stage1 mean summary: {}",
            &result[..result.len().min(800)]
        );
    }

    /// Test: Edge tooltip uses total_consumed (cumulative), not last_total (inflight)
    /// This verifies the fix - tooltip should match edge label
    #[test]
    fn test_edge_tooltip_uses_total_consumed() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        // Create edge with known total_consumed and last_total
        let mut stats = ChannelStatsComputer::default();
        stats.capacity = 100;
        stats.show_total = true;
        stats.total_consumed = 1000; // Cumulative total - what user wants to see
        stats.last_total = 50; // Current inflight - NOT what user wants to see
        stats.saturation_score = 0.5;

        let edge = Edge {
            id: 0,
            from: Some(from),
            to: Some(to),
            color: "green",
            sidecar: false,
            pen_width: "1".to_string(),
            saturation_score: 0.5,
            ctl_labels: vec![],
            stats_computer: stats,
            display_label: "test".to_string(),
            metric_text: String::new(),
            partner: None,
            bundle_index: None,
        };

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges: vec![edge],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        // Edge label should show total_consumed (1000)
        assert!(
            result.contains("Total: 1000"),
            "Edge label should show total_consumed: {}",
            result
        );

        // Tooltip should also show total_consumed (1000), NOT last_total (50)
        assert!(
            result.contains("Total: 1000"),
            "Tooltip should show total_consumed, not last_total: {}",
            result
        );

        println!("✓ Edge tooltip correctly uses total_consumed (cumulative): 1000");
    }

    /// When `avg_filled` is enabled, tooltip shows rolling-window **Avg fill**, not snapshot Instant fill
    /// (which is often 0% when inflight is drained between samples).
    #[test]
    fn test_edge_tooltip_prefers_avg_fill_when_enabled() {
        use crate::actor_stats::ChannelBlock;

        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut stats = ChannelStatsComputer::default();
        stats.capacity = 100;
        stats.show_total = true;
        stats.show_avg_filled = true;
        stats.refresh_rate_in_bits = 0;
        stats.window_bucket_in_bits = 0;
        stats.total_consumed = 0;
        stats.saturation_score = 0.0;
        stats.current_filled = Some(ChannelBlock {
            histogram: None,
            runner: 50_000,
            sum_of_squares: 0,
        });

        let edge = Edge {
            id: 0,
            from: Some(from),
            to: Some(to),
            color: "green",
            sidecar: false,
            pen_width: "1".to_string(),
            saturation_score: 0.0,
            ctl_labels: vec![],
            stats_computer: stats,
            display_label: "edge".to_string(),
            metric_text: String::new(),
            partner: None,
            bundle_index: None,
        };

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges: vec![edge],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            result.contains("Avg fill: 50%"),
            "expected rolling avg fill in tooltip: {}",
            result
        );
        assert!(
            !result.contains("Instant fill:"),
            "should not show Instant fill when avg fill is enabled: {}",
            result
        );
    }

    /// No rolling-window sample: omit `Avg fill` entirely (no `-` placeholder).
    #[test]
    fn test_edge_tooltip_omits_avg_fill_when_no_window_sample() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut stats = ChannelStatsComputer::default();
        stats.capacity = 100;
        stats.show_total = true;
        stats.show_avg_filled = true;
        stats.refresh_rate_in_bits = 0;
        stats.window_bucket_in_bits = 0;
        stats.total_consumed = 0;
        stats.saturation_score = 0.0;

        let edge = Edge {
            id: 0,
            from: Some(from),
            to: Some(to),
            color: "green",
            sidecar: false,
            pen_width: "1".to_string(),
            saturation_score: 0.0,
            ctl_labels: vec![],
            stats_computer: stats,
            display_label: "edge".to_string(),
            metric_text: String::new(),
            partner: None,
            bundle_index: None,
        };

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges: vec![edge],
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            !result.contains("Avg fill:"),
            "must not print Avg fill placeholder when no sample: {}",
            result
        );
    }

    /// Partner rollup: all lanes lack `current_filled` → no `Avg fill` line on the edge label.
    #[test]
    fn test_partner_rollup_no_avg_fill_when_all_samples_missing() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let lane0 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        let lane1 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };

        let edges = vec![
            Edge {
                id: 0,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.1,
                ctl_labels: vec![],
                stats_computer: lane0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
            Edge {
                id: 1,
                from: Some(from),
                to: Some(to),
                color: "red",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.4,
                ctl_labels: vec![],
                stats_computer: lane1,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
        ];

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            !result.contains("Avg fill:"),
            "rollup must omit Avg fill when every lane lacks a sample: {}",
            result
        );
    }

    /// Partner rollup: one lane has a sample, one does not → single percent, no comma placeholder.
    #[test]
    fn test_partner_rollup_avg_fill_skips_none_lane() {
        use crate::actor_stats::ChannelBlock;

        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        let mut lane0 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        lane0.current_filled = Some(ChannelBlock {
            histogram: None,
            runner: 10_000,
            sum_of_squares: 0,
        });

        let lane1 = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            show_type: Some("T"),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };

        let edges = vec![
            Edge {
                id: 0,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.1,
                ctl_labels: vec![],
                stats_computer: lane0,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
            Edge {
                id: 1,
                from: Some(from),
                to: Some(to),
                color: "red",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.4,
                ctl_labels: vec![],
                stats_computer: lane1,
                display_label: String::new(),
                metric_text: String::new(),
                partner: Some("L"),
                bundle_index: Some(0),
            },
        ];

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "from".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(
            result.contains("Avg fill: 10%"),
            "expected single sampled lane only: {}",
            result
        );
        assert!(
            !result.contains("Avg fill: 10%,"),
            "must not emit trailing comma for missing second lane: {}",
            result
        );
    }

    /// Test: Bundle tooltip uses sum of total_consumed, not sum of last_total
    #[test]
    fn test_bundle_tooltip_uses_total_consumed() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        // Create 3 edges with different total_consumed and last_total
        let mut edges = Vec::new();
        for i in 0..3 {
            let mut stats = ChannelStatsComputer::default();
            stats.capacity = 100;
            stats.show_total = true;
            stats.total_consumed = (i as u128 + 1) * 100; // 100, 200, 300 = 600 total
            stats.last_total = (i as i64 + 1) * 10; // 10, 20, 30 = 60 total (inflight)
            stats.saturation_score = 0.3;

            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.3,
                ctl_labels: vec![],
                stats_computer: stats,
                display_label: format!("CH{}", i),
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
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        // Each edge shows its own Total in the label (format: "CH0Total: 100")
        assert!(
            result.contains("Total: 100"),
            "Edge 0 should show Total: 100: {}",
            result
        );
        assert!(
            result.contains("Total: 200"),
            "Edge 1 should show Total: 200: {}",
            result
        );
        assert!(
            result.contains("Total: 300"),
            "Edge 2 should show Total: 300: {}",
            result
        );

        // Tooltip should also show these totals
        assert!(
            result.contains("Total: 100"),
            "Tooltip channel 0 should show 100: {}",
            result
        );
        assert!(
            result.contains("Total: 200"),
            "Tooltip channel 1 should show 200: {}",
            result
        );
        assert!(
            result.contains("Total: 300"),
            "Tooltip channel 2 should show 300: {}",
            result
        );

        println!("✓ Bundle tooltip correctly uses total_consumed (cumulative)");
    }

    /// Test: Large bundle (more than MAX_INLINE channels) shows summary without total volume or avg saturation
    #[test]
    fn test_large_bundle_tooltip_no_total_volume() {
        let from = ActorName::new("from", None);
        let to = ActorName::new("to", None);

        // Create 25 edges (large bundle)
        let mut edges = Vec::new();
        for i in 0..25 {
            let mut stats = ChannelStatsComputer::default();
            stats.capacity = 100;
            stats.show_total = true;
            stats.total_consumed = (i as u128 + 1) * 100;
            stats.last_total = (i as i64 + 1) * 10;
            stats.saturation_score = 0.3;

            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.3,
                ctl_labels: vec![],
                stats_computer: stats,
                display_label: format!("CH{}", i),
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
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        // Large bundle should show summary
        assert!(
            result.contains("Summary: 25 channels"),
            "Large bundle should show Summary: {}",
            result
        );

        // Should NOT contain Total Volume or Avg Saturation
        assert!(
            !result.contains("Total Volume:"),
            "Large bundle should NOT show Total Volume: {}",
            result
        );
        assert!(
            !result.contains("Avg Saturation:"),
            "Large bundle should NOT show Avg Saturation: {}",
            result
        );
    }

    /// Test: Partner channels show correct rollup
    #[test]
    fn test_partner_tooltip_uses_total_consumed() {
        let from = ActorName::new("partner", None);
        let to = ActorName::new("to", None);

        // Create 3 partner lanes
        let mut edges = Vec::new();
        let mut expected_total = 0u128;
        for i in 0..3 {
            let mut stats = ChannelStatsComputer::default();
            stats.capacity = 100;
            stats.show_total = true;
            let tc = (i as u128 + 1) * 1000; // 1000, 2000, 3000
            stats.total_consumed = tc;
            stats.last_total = (i as i64 + 1) * 100; // 100, 200, 300
            stats.saturation_score = 0.4;
            expected_total += tc;

            edges.push(Edge {
                id: i,
                from: Some(from),
                to: Some(to),
                color: "green",
                sidecar: false,
                pen_width: "1".to_string(),
                saturation_score: 0.4,
                ctl_labels: vec![],
                stats_computer: stats,
                display_label: format!("CH{}", i),
                metric_text: String::new(),
                partner: Some("partner_lane"),
                bundle_index: Some(i),
            });
        }

        let state = DotState {
            nodes: vec![
                Node {
                    id: Some(from),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "partner".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 40,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        // Each partner lane shows its Total in the label
        assert!(
            result.contains("Total: 1000"),
            "Partner lane 0 should show Total: 1000: {}",
            result
        );
        assert!(
            result.contains("Total: 2000"),
            "Partner lane 1 should show Total: 2000: {}",
            result
        );
        assert!(
            result.contains("Total: 3000"),
            "Partner lane 2 should show Total: 3000: {}",
            result
        );

        // Tooltip should also show these totals
        assert!(
            result.contains("Total: 1000"),
            "Tooltip should show 1000: {}",
            result
        );

        println!(
            "✓ Partner tooltip correctly uses total_consumed: expected = {}",
            expected_total
        );
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
        assert!(
            path.to_str()
                .expect("iternal error")
                .contains(&frame_history.guid)
        );
    }

    #[test]
    fn test_frame_history_mark_position() {
        let mut frame_history = FrameHistory::new(1000);
        frame_history.mark_position();
        assert_eq!(
            frame_history.buffer_bytes_count,
            frame_history.history_buffer.len()
        );
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

        let result = std::fs::read_to_string(&path).expect("Failed to read written file");
        assert_eq!(result, "test data");

        // Clean up
        let _ = remove_file(&path);
    }

    #[test]
    fn test_node_compute_refresh_with_load_calculation() {
        // Test THE load calculation branch (lines 66-69)
        let actor_status = ActorStatus {
            ident: Default::default(),
            await_total_ns: 100,
            unit_total_ns: 500,
            total_count_restarts: 1,
            iteration_start: 10, // Non-zero to trigger load calculation
            iteration_sum: 0,
            bool_stop: false,
            is_quiet: false,
            calls: [0; 6],
            thread_info: None,
            bool_blocking: false,
        };
        let mut node = Node {
            id: Some(ActorName::new("test_node", None)),
            color: "grey",
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            dot_subtitle: None,
            tooltip: String::new(),
            metric_text: String::new(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            bool_stalled: false,
            work_info: None,
        };
        node.compute_and_refresh(actor_status);
        // Local load: 100 * (500-100)/500 = 80; mCPU: (400*1024)/500 = 819 (integer busy fraction).
        assert_eq!(node.work_info, Some((819, 80)));
    }

    #[test]
    fn test_node_compute_refresh_full_busy_when_await_zero() {
        // No instrumented/profile time in window → treat as fully busy (not 0 mCPU).
        let actor_status = ActorStatus {
            ident: Default::default(),
            await_total_ns: 0,
            unit_total_ns: 500,
            total_count_restarts: 0,
            iteration_start: 1,
            iteration_sum: 1,
            bool_stop: false,
            is_quiet: false,
            calls: [0; 6],
            thread_info: None,
            bool_blocking: false,
        };
        let mut node = Node {
            id: Some(ActorName::new("full_busy", None)),
            color: "grey",
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            dot_subtitle: None,
            tooltip: String::new(),
            metric_text: String::new(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            bool_stalled: false,
            work_info: None,
        };
        node.compute_and_refresh(actor_status);
        assert_eq!(node.work_info, Some((1024, 100)));
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
        assert_eq!(
            local_state.nodes[0].id.expect("internal error").name,
            "test_actor"
        );
        assert!(local_state.nodes[0].remote_details.is_some());
        assert_eq!(local_state.edges.len(), 2); // One for input, one for output
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
                    total_consumed: (i as u128 + 1) * 100,
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
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
                Node {
                    id: Some(to),
                    color: "grey",
                    pen_width: NODE_PEN_WIDTH,
                    stats_computer: ActorStatsComputer::default(),
                    display_label: "to".to_string(),
                    dot_subtitle: None,
                    tooltip: String::new(),
                    metric_text: String::new(),
                    remote_details: None,
                    thread_info_cache: None,
                    total_count_restarts: 0,
                    bool_stalled: false,
                    work_info: None,
                },
            ],
            edges,
            seq: 0,
            telemetry_colors: None,
            refresh_rate_ms: 0,
            bundle_floor_size: 4,
        };

        let mut frames = test_dot_frames();
        build_dot(&state, &mut frames);
        let result = String::from_utf8(frames.active_graph.to_vec()).expect("internal error");

        assert!(result.contains("Bundle: 5x"));
        assert!(result.contains("penwidth=4"));
        assert!(result.contains("style=\"bold,dashed\""));
        // Check for bundle tooltip - look for the header (without trailing newline)
        assert!(result.contains("Bundle (5 chans in 5 groups):"));
        assert!(result.contains("CH#0: TestType"));
        assert!(result.contains("Instant fill: 10%"));
    }
}
