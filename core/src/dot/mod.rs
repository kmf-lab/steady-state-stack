// This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

// ss[related telemetry.dot-export]
mod build;
// ss[impl telemetry.dot-export]
mod colors;
// ss[impl telemetry.dot-export]
mod escape;
// ss[related telemetry.dot-export]
mod format;
// ss[impl telemetry.dot-export]
mod frames;
// ss[impl telemetry.dot-export]
mod history;
// ss[related telemetry.dot-export]
mod keys;
// ss[impl telemetry.dot-export]
mod metric;
// ss[impl telemetry.dot-export]
mod partnered;
// ss[related telemetry.dot-export]
mod register;
// ss[impl telemetry.dot-export]
mod render;

// ss[related telemetry.dot-export]
pub(crate) use build::build_dot;
// ss[impl telemetry.dot-export]
pub(crate) use colors::actor_fillcolor_hex_into;
// ss[impl telemetry.dot-export]
pub(crate) use frames::DotGraphFrames;
// ss[related telemetry.dot-export]
pub(crate) use history::FrameHistory;
// ss[impl telemetry.dot-export]
pub(crate) use metric::build_metric;
// ss[impl telemetry.dot-export]
pub(crate) use register::apply_node_def;

#[cfg(test)]
// ss[related telemetry.dot-export]
pub(crate) use colors::{color_to_rgb, rgb_to_hex_into};
#[cfg(test)]
// ss[impl telemetry.dot-export]
pub(crate) use escape::{escape_dot_quotes, escape_node_tooltip_text};
#[cfg(test)]
// ss[related telemetry.dot-export]
pub(crate) use format::{
    append_channel_fill_tooltip, format_avg_fill_rollup_line_into,
    mean_avg_fill_from_edge_slice, mean_avg_fill_percent,
};
#[cfg(test)]
// ss[related telemetry.dot-export]
pub(crate) use register::define_unified_edges;

// ss[impl telemetry.dot-export]
use crate::actor_stats::ActorStatsComputer;
// ss[related telemetry.dot-export]
use crate::channel_stats::ChannelStatsComputer;
// ss[impl telemetry.dot-export]
use crate::dot_edge::Edge;
// ss[impl telemetry.dot-export]
use crate::dot_node::Node;
// ss[related telemetry.dot-export]
use crate::*;
// ss[impl telemetry.dot-export]
use std::fs::OpenOptions;

/// Represents the state of metrics for the graph, including nodes and edges.
#[derive(Default)]
// ss[related telemetry.dot-export]
pub struct DotState {
    // ss[impl telemetry.dot-export]
    pub(crate) nodes: Vec<Node>, // Position matches the node ID
    // ss[impl telemetry.dot-export]
    pub(crate) edges: Vec<Edge>, // Position matches the channel ID
    pub seq: u64,
    // ss[impl telemetry.dot-export]
    pub(crate) telemetry_colors: Option<(String, String)>,
    // ss[impl telemetry.dot-export]
    pub(crate) refresh_rate_ms: u64,
    // ss[impl telemetry.dot-export]
    pub(crate) bundle_floor_size: usize,
}

/// Sum of last-known mCPU across all defined actor nodes.
// ss[related telemetry.dot-export]
pub(crate) fn graph_mcpu_total(nodes: &[Node]) -> u128 {
    nodes
        .iter()
        .filter(|n| n.id.is_some())
        .filter_map(|n| n.work_info.map(|(mcpu, _)| mcpu as u128))
        .sum()
}

// ss[related telemetry.dot-export]
impl DotState {
    /// Recomputes graph-share load for every defined node from last-known mCPU totals.
    ///
    /// Only indexes listed in `touched` accumulate new rolling samples; others refresh labels only.
    // ss[related telemetry.dot-export]
    pub(crate) fn refresh_actor_loads(&mut self, touched: &[usize]) {
        let total = graph_mcpu_total(&self.nodes);
        let touched_set: std::collections::HashSet<usize> = touched.iter().copied().collect();
        for (idx, node) in self.nodes.iter_mut().enumerate() {
            if node.id.is_none() || node.work_info.is_none() {
                continue;
            }
            let accumulate = touched_set.contains(&idx);
            node.apply_graph_load_and_emit(total, accumulate);
        }
    }
}

#[derive(Default, Clone, Debug)]
// ss[related telemetry.dot-export]
pub struct RemoteDetails {
    // ss[impl telemetry.dot-export]
    pub(crate) ips: String,
    // ss[impl telemetry.dot-export]
    pub(crate) match_on: String,
    // ss[impl telemetry.dot-export]
    pub(crate) tech: &'static str,
    // ss[impl telemetry.dot-export]
    pub(crate) direction: &'static str, //  in OR out
}

/// The default pen width for nodes in the DOT graph.
// ss[related telemetry.dot-export]
pub(crate) const NODE_PEN_WIDTH: &str = "3";
/// The default pen width for edges in the DOT graph.
// ss[impl telemetry.dot-export]
pub(crate) const EDGE_PEN_WIDTH: &str = "1";
/// The pen width for bundles of single kind of channels.
// ss[related telemetry.dot-export]
pub(crate) const BUNDLE_PEN_WIDTH: &str = "4";
/// The pen width for bundles of partnered channels.
// ss[impl telemetry.dot-export]
pub(crate) const PARTNER_BUNDLE_PEN_WIDTH: &str = "2";

/// Graphviz `dot` spacing (`rankdir=LR`). Smaller = tighter. This is not neato/fdp gravity.
///
/// `ranksep` is the column gap (the main pull-in). `nodesep` is the same-rank gap
/// (keeps sidecar pairs from stacking). Old roomy values were `nodesep=.5`, `ranksep=2.5`
/// so edge labels had space. If labels collide, raise `DOT_RANKSEP` first.
// ss[related telemetry.dot-export]
pub(crate) const DOT_NODESEP: &str = ".35";
/// Column gap for Graphviz `dot` (`rankdir=LR`). See `DOT_NODESEP`.
// ss[impl telemetry.dot-export]
pub(crate) const DOT_RANKSEP: &str = "1.2";

/// Percent of border RGB mixed into actor node `fillcolor` (remainder is white). Tweak for a stronger or weaker tint.
// ss[related telemetry.dot-export]
pub(crate) const ACTOR_FILL_TINT_PERCENT: u32 = 12;

/// Max number of per-channel `Avg fill` values to print comma-separated; above this, labels use
/// a single `mean, N ch` line (aligns with large-bundle tooltips and Stage 2 bundle headers).
// ss[related telemetry.dot-export]
pub(crate) const MAX_INLINE_AVG_FILL_LANES: usize = 20;

#[cfg(test)]
#[path = "tests/mod.rs"]
// ss[related telemetry.dot-export]
mod dot_tests;
