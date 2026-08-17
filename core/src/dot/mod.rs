// This module provides the metrics for both local Graphviz DOT telemetry and Prometheus telemetry
//! based on the settings for the actor builder in the SteadyState project. It includes functions for
//! computing and refreshing metrics, building DOT and Prometheus outputs, and managing historical data.

mod build;
mod colors;
mod escape;
mod format;
mod frames;
mod history;
mod keys;
mod metric;
mod partnered;
mod register;
mod render;

pub(crate) use build::build_dot;
pub(crate) use colors::actor_fillcolor_hex_into;
pub(crate) use frames::DotGraphFrames;
pub(crate) use history::FrameHistory;
pub(crate) use metric::build_metric;
pub(crate) use register::apply_node_def;

#[cfg(test)]
pub(crate) use colors::{color_to_rgb, rgb_to_hex_into};
#[cfg(test)]
pub(crate) use escape::{escape_dot_quotes, escape_node_tooltip_text};
#[cfg(test)]
pub(crate) use format::{
    append_channel_fill_tooltip, format_avg_fill_rollup_line_into,
    mean_avg_fill_from_edge_slice, mean_avg_fill_percent,
};
#[cfg(test)]
pub(crate) use register::define_unified_edges;

use crate::actor_stats::ActorStatsComputer;
use crate::channel_stats::ChannelStatsComputer;
use crate::dot_edge::Edge;
use crate::dot_node::Node;
use crate::*;
use std::fs::OpenOptions;

/// Represents the state of metrics for the graph, including nodes and edges.
#[derive(Default)]
// ss[related telemetry.dot-export]
pub struct DotState {
    pub(crate) nodes: Vec<Node>, // Position matches the node ID
    pub(crate) edges: Vec<Edge>, // Position matches the channel ID
    pub seq: u64,
    pub(crate) telemetry_colors: Option<(String, String)>,
    pub(crate) refresh_rate_ms: u64,
    pub(crate) bundle_floor_size: usize,
}

#[derive(Default, Clone, Debug)]
// ss[related telemetry.dot-export]
pub struct RemoteDetails {
    pub(crate) ips: String,
    pub(crate) match_on: String,
    pub(crate) tech: &'static str,
    pub(crate) direction: &'static str, //  in OR out
}

/// The default pen width for nodes in the DOT graph.
// ss[related telemetry.dot-export]
pub(crate) const NODE_PEN_WIDTH: &str = "3";
/// The default pen width for edges in the DOT graph.
pub(crate) const EDGE_PEN_WIDTH: &str = "1";
/// The pen width for bundles of single kind of channels.
// ss[related telemetry.dot-export]
pub(crate) const BUNDLE_PEN_WIDTH: &str = "4";
/// The pen width for bundles of partnered channels.
pub(crate) const PARTNER_BUNDLE_PEN_WIDTH: &str = "2";

/// Graphviz `dot` spacing (`rankdir=LR`). Smaller = tighter. This is not neato/fdp gravity.
///
/// `ranksep` is the column gap (the main pull-in). `nodesep` is the same-rank gap
/// (keeps sidecar pairs from stacking). Old roomy values were `nodesep=.5`, `ranksep=2.5`
/// so edge labels had space. If labels collide, raise `DOT_RANKSEP` first.
// ss[related telemetry.dot-export]
pub(crate) const DOT_NODESEP: &str = ".35";
/// Column gap for Graphviz `dot` (`rankdir=LR`). See `DOT_NODESEP`.
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
mod dot_tests;
