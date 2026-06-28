// ss[related telemetry.dot-export]
use std::sync::Arc;

use crate::actor_stats::ActorStatsComputer;
use crate::dot_node::Node;
use crate::dot_unify::ChannelEdgeRole;
use crate::graph_liveliness::ActorIdentity;
use crate::monitor::{ActorMetaData, ChannelMetaData};

use super::{DotState, NODE_PEN_WIDTH};

/// Applies the node definition to the local state.
///
/// Each [`ChannelMetaData`](crate::monitor::ChannelMetaData) telemetry id maps to **one**
/// unified edge slot: **`channels_in` sets [`Edge::to`](crate::dot_edge::Edge), `channels_out` sets [`Edge::from`](crate::dot_edge::Edge)**.
/// A second actor claiming the same id on the **same side** emits a structured warning (`steady_state::telemetry::dot`)
/// and indicates inconsistent metadata—often mixed [`Graph`] `channel_builder` namespaces or swapped rx/tx registration in `into_spotlight`.
///
/// * `channels_in` / `channels_out` must follow the same contract as [`crate::dot_unify`]: one tx and one rx
/// claimant per [`ChannelMetaData`](crate::monitor::ChannelMetaData) telemetry id in a single metrics [`DotState`](DotState).
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
        actor.ident,
        channels_in,
        ChannelEdgeRole::SetsEdgeTo,
        frame_rate_ms,
    );
    define_unified_edges(
        local_state,
        actor.ident,
        channels_out,
        ChannelEdgeRole::SetsEdgeFrom,
        frame_rate_ms,
    );
}

/// Defines unified edges in the local state.
///
/// # Arguments
///
/// * `local_state` - THE local metric state.
/// * `actor_ident` - THE identity of the node (numeric id + label).
/// * `mdvec` - THE metadata of the channels.
/// * `role` - Incoming vs outgoing registration for [`DotState.edges`].
/// * `frame_rate_ms` - THE frame rate in milliseconds.
pub(crate) fn define_unified_edges(
    local_state: &mut DotState,
    actor_ident: ActorIdentity,
    mdvec: &[Arc<ChannelMetaData>],
    role: ChannelEdgeRole,
    frame_rate_ms: u64,
) {
    mdvec.iter().for_each(|meta| {
        let _ = crate::dot_unify::apply_channel_to_unified_edges(
            local_state, actor_ident, meta, role, frame_rate_ms,
        );
    });
}
