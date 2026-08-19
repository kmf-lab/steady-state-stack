//! Unified edge slots for telemetry [`DotState`](crate::dot::DotState), keyed by
//! [`ChannelMetaData::id`](crate::monitor::ChannelMetaData).
//!
//! # Invariant
//!
//! For each channel telemetry id there must be **at most one** actor setting [`Edge::to`](crate::dot_edge::Edge)
//! ([`ChannelEdgeRole::SetsEdgeTo`], from payload **receive** / `channels_in`) and **at most one** setting
//! [`Edge::from`](crate::dot_edge::Edge) ([`ChannelEdgeRole::SetsEdgeFrom`], from **send** / `channels_out`).
//!
//! If you see `steady_state::telemetry::dot` warnings about endpoint conflicts:
//! - Ensure every channel is allocated from [`Graph::channel_builder()`](crate::Graph::channel_builder) on the **same**
//!   [`Graph`](crate::Graph) (one shared `channel_count` namespace).
//! - In [`SteadyActorShadow::into_spotlight`](crate::steady_actor_shadow::SteadyActorShadow::into_spotlight), list each wire on
//!   the **receiver** side in rx metadata and on the **sender** side in tx metadata only—never both on the same side
//!   for two different actors sharing one id.

// ss[related telemetry.dot-export]
use std::sync::Arc;
#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicUsize, Ordering};

// ss[related telemetry.dot-export]
use log::{trace, warn};
#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
use log::debug;

// ss[related telemetry.dot-export]
use crate::ActorName;
// ss[related telemetry.dot-export]
use crate::channel_stats::ChannelStatsComputer;
// ss[related philosophy.structural-hierarchy]
use crate::dot::DotState;
// ss[related philosophy.structural-hierarchy]
use crate::dot::EDGE_PEN_WIDTH;
// ss[related telemetry.dot-export]
use crate::dot_edge::Edge;
// ss[related philosophy.structural-hierarchy]
use crate::monitor::ChannelMetaData;
// ss[related philosophy.structural-hierarchy]
use crate::graph_liveliness::ActorIdentity;

/// Declares which endpoint of a unified edge this actor is registering (see module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// ss[related telemetry.dot-export]
pub(crate) enum ChannelEdgeRole {
    /// `channels_in`: this actor receives on the channel; sets `DotState.edges[id].to`.
    SetsEdgeTo,
    /// `channels_out`: this actor sends on the channel; sets `DotState.edges[id].from`.
    SetsEdgeFrom,
}

// ss[related telemetry.dot-export]
impl ChannelEdgeRole {
    #[inline]
    // ss[related philosophy.structural-hierarchy]
    fn as_endpoint_str(self) -> &'static str {
        match self {
            ChannelEdgeRole::SetsEdgeTo => "to",
            ChannelEdgeRole::SetsEdgeFrom => "from",
        }
    }
}

/// Describes a second actor claiming the same directional endpoint for one channel id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
// ss[related telemetry.dot-export]
pub(crate) struct EdgeEndpointConflict {
    /// [`ChannelMetaData::id`](crate::monitor::ChannelMetaData) telemetry key.
    pub channel_id: usize,
    /// `"to"` for receive-side registration, `"from"` for send-side.
    pub endpoint: &'static str,
    /// Actor that first claimed this endpoint.
    pub existing: ActorName,
    pub existing_actor_numeric_id: Option<usize>,
    pub existing_claim_meta_arc: Option<usize>,
    pub new_claimant: ActorName,
    pub new_claimant_actor_numeric_id: usize,
    /// `Arc::as_ptr(meta)` for the conflicting (second) claimant’s metadata snapshot.
    pub new_claim_arc_ptr: usize,
}

#[inline]
// ss[related telemetry.dot-export]
fn placeholder_edge_slot() -> Edge {
    Edge {
        id: usize::MAX,
        from: None,
        to: None,
        sidecar: false,
        stats_computer: ChannelStatsComputer::default(),
        ctl_labels: Vec::new(),
        color: "grey",
        pen_width: EDGE_PEN_WIDTH.to_string(),
        saturation_score: 0.0,
        display_label: String::new(),
        metric_text: String::new(),
        partner: None,
        bundle_index: None,
        ..Default::default()
    }
}

/// Test-only: incremented whenever [`log_endpoint_conflict`] runs—so cargo-mutants cannot replace
/// that helper with `()` without breaking tests (guarded by [`EDGE_DIAG_MUTEX`] in assertions).
#[cfg(test)]
// ss[related telemetry.dot-export]
pub(crate) static EDGE_CONFLICT_DIAG_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
// ss[related telemetry.dot-export]
static EDGE_DIAG_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[inline]
// ss[related telemetry.dot-export]
fn log_endpoint_conflict(details: &EdgeEndpointConflict, meta: &Arc<ChannelMetaData>) {
    #[cfg(test)]
    EDGE_CONFLICT_DIAG_COUNT.fetch_add(1, Ordering::Relaxed);
    #[cfg(not(test))]
    warn!(
        target: "steady_state::telemetry::dot",
        concat!(
            "dot edge endpoint conflict ",
            "channel_id={} endpoint={} existing_actor={:?} existing_actor_id={:?} existing_first_arc_ptr={:?} ",
            "new_actor={:?} new_actor_id={} new_claim_arc_ptr={:#x} ",
            "partner={:?} bundle_index={:?} ",
            "label_len={} first_label={:?} connects_sidecar={}",
        ),
        details.channel_id,
        details.endpoint,
        details.existing,
        details.existing_actor_numeric_id,
        details.existing_claim_meta_arc,
        details.new_claimant,
        details.new_claimant_actor_numeric_id,
        details.new_claim_arc_ptr,
        meta.partner,
        meta.bundle_index,
        meta.labels.len(),
        meta.labels.first().copied(),
        meta.connects_sidecar,
    );
    #[cfg(test)]
    debug!(
        target: "steady_state::telemetry::dot",
        concat!(
            "dot edge endpoint conflict ",
            "channel_id={} endpoint={} existing_actor={:?} existing_actor_id={:?} existing_first_arc_ptr={:?} ",
            "new_actor={:?} new_actor_id={} new_claim_arc_ptr={:#x} ",
            "partner={:?} bundle_index={:?} ",
            "label_len={} first_label={:?} connects_sidecar={}",
        ),
        details.channel_id,
        details.endpoint,
        details.existing,
        details.existing_actor_numeric_id,
        details.existing_claim_meta_arc,
        details.new_claimant,
        details.new_claimant_actor_numeric_id,
        details.new_claim_arc_ptr,
        meta.partner,
        meta.bundle_index,
        meta.labels.len(),
        meta.labels.first().copied(),
        meta.connects_sidecar,
    );
    trace!(
        target: "steady_state::telemetry::dot",
        "dot edge endpoint conflict meta.labels={:?}",
        meta.labels,
    );
}

/// Applies one channel's metadata into [`DotState::edges`] (grow slot, assign `from`/`to`, merge labels, maybe init stats).
///
/// Returns [`Some`] when a **different** actor already claimed the same endpoint for this channel id (a warning is emitted).
///
/// Behaviour matches the legacy `define_unified_edges` per-item loop—including continuing with label merge and stats refresh
/// after a conflict warning.
#[must_use]
// ss[impl telemetry.dot-export]
pub(crate) fn apply_channel_to_unified_edges(
    local_state: &mut DotState,
    actor_ident: ActorIdentity,
    meta: &Arc<ChannelMetaData>,
    role: ChannelEdgeRole,
    frame_rate_ms: u64,
) -> Option<EdgeEndpointConflict> {
    let idx = meta.id;

    if idx.ge(&local_state.edges.len()) {
        local_state.edges.resize_with(idx + 1, placeholder_edge_slot);
    }

    let edge = &mut local_state.edges[idx];
    assert!(edge.id == idx || edge.id == usize::MAX);
    edge.id = idx;

    let mut conflict = None;

    match role {
        ChannelEdgeRole::SetsEdgeTo => {
            if edge.to.is_none() {
                edge.to = Some(actor_ident.label);
                edge.diag_to_claim_actor_id = Some(actor_ident.id);
                edge.diag_to_claim_meta_arc = Some(Arc::as_ptr(meta) as usize);
            } else if !Some(actor_ident.label).eq(&edge.to) {
                let existing = edge.to.expect("internal error edge.to invariant");
                let details = EdgeEndpointConflict {
                    channel_id: idx,
                    endpoint: role.as_endpoint_str(),
                    existing,
                    existing_actor_numeric_id: edge.diag_to_claim_actor_id,
                    existing_claim_meta_arc: edge.diag_to_claim_meta_arc,
                    new_claimant: actor_ident.label,
                    new_claimant_actor_numeric_id: actor_ident.id,
                    new_claim_arc_ptr: Arc::as_ptr(meta) as usize,
                };
                log_endpoint_conflict(&details, meta);
                conflict = Some(details);
            }
        }
        ChannelEdgeRole::SetsEdgeFrom => {
            if edge.from.is_none() {
                edge.from = Some(actor_ident.label);
                edge.diag_from_claim_actor_id = Some(actor_ident.id);
                edge.diag_from_claim_meta_arc = Some(Arc::as_ptr(meta) as usize);
            } else if !Some(actor_ident.label).eq(&edge.from) {
                let existing = edge.from.expect("internal error edge.from invariant");
                let details = EdgeEndpointConflict {
                    channel_id: idx,
                    endpoint: role.as_endpoint_str(),
                    existing,
                    existing_actor_numeric_id: edge.diag_from_claim_actor_id,
                    existing_claim_meta_arc: edge.diag_from_claim_meta_arc,
                    new_claimant: actor_ident.label,
                    new_claimant_actor_numeric_id: actor_ident.id,
                    new_claim_arc_ptr: Arc::as_ptr(meta) as usize,
                };
                log_endpoint_conflict(&details, meta);
                conflict = Some(details);
            }
        }
    }

    let labels_to_add: Vec<&'static str> = meta
        .labels
        .iter()
        .copied()
        .filter(|f| !edge.ctl_labels.contains(f))
        .collect();
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

    conflict
}

#[cfg(test)]
// ss[related telemetry.dot-export]
mod unify_edge_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::ss_proptest;
    // ss[related telemetry.dot-export]
    use crate::graph_liveliness::ActorIdentity;

    // ss[related telemetry.dot-export]
    fn meta_with_id_labels(id: usize, labels: Vec<&'static str>) -> Arc<ChannelMetaData> {
        Arc::new(ChannelMetaData {
            id,
            labels,
            capacity: 8,
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
            partner: Some("partner_x"),
            bundle_index: Some(2),
            type_byte_count: 4,
            show_total: false,
            girth: 1,
            show_memory: false,
        })
    }

    // ss[related telemetry.dot-export]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: at most one SetsEdgeTo and one SetsEdgeFrom claimant per channel id.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_at_most_one_endpoint_per_channel(
            claims in prop::collection::vec(
                (0usize..32, 1usize..500, prop::bool::ANY),
                1..24,
            ),
        ) {
            let _lock = EDGE_DIAG_MUTEX.lock().expect("edge diag mutex poisoned");
            EDGE_CONFLICT_DIAG_COUNT.store(0, Ordering::Relaxed);
            let mut st = DotState::default();
            let mut to_owner: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            let mut from_owner: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

            for (channel_id, actor_id, is_to) in claims {
                let role = if is_to { ChannelEdgeRole::SetsEdgeTo } else { ChannelEdgeRole::SetsEdgeFrom };
                let actor = ActorIdentity::new(actor_id, "actor", Some(actor_id));
                let meta = meta_with_id_labels(channel_id, vec![]);
                let conflict = apply_channel_to_unified_edges(&mut st, actor, &meta, role, 1000);

                match role {
                    ChannelEdgeRole::SetsEdgeTo => {
                        if let Some(&existing) = to_owner.get(&channel_id) {
                            if existing != actor_id {
                                prop_assert!(conflict.is_some());
                            } else {
                                prop_assert!(conflict.is_none());
                            }
                        } else {
                            prop_assert!(conflict.is_none());
                            to_owner.insert(channel_id, actor_id);
                        }
                    }
                    ChannelEdgeRole::SetsEdgeFrom => {
                        if let Some(&existing) = from_owner.get(&channel_id) {
                            if existing != actor_id {
                                prop_assert!(conflict.is_some());
                            } else {
                                prop_assert!(conflict.is_none());
                            }
                        } else {
                            prop_assert!(conflict.is_none());
                            from_owner.insert(channel_id, actor_id);
                        }
                    }
                }
            }

            for (id, edge) in st.edges.iter().enumerate() {
                if edge.id == usize::MAX {
                    continue;
                }
                if edge.to.is_some() && edge.from.is_some() {
                    prop_assert_eq!(edge.id, id);
                }
            }
        }
    }

    // ss[verify telemetry.dot-export]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn default_channel_meta_id_zero_collides_by_design() {
        let _lock = EDGE_DIAG_MUTEX.lock().expect("edge diag mutex poisoned");
        EDGE_CONFLICT_DIAG_COUNT.store(0, Ordering::Relaxed);
        let mut st = DotState::default();
        let m1 = Arc::new(ChannelMetaData::default());
        let m2 = Arc::new(ChannelMetaData::default());
        let alice = ActorIdentity::new(801, "alice", None);
        let bob = ActorIdentity::new(802, "bob", None);
        assert!(
            apply_channel_to_unified_edges(&mut st, alice, &m1, ChannelEdgeRole::SetsEdgeTo, 1).is_none()
        );
        let c = apply_channel_to_unified_edges(&mut st, bob, &m2, ChannelEdgeRole::SetsEdgeTo, 1);
        assert!(c.is_some());
        assert_eq!(EDGE_CONFLICT_DIAG_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(c.unwrap().channel_id, 0);
    }
}
