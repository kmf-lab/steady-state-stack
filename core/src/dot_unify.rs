//! Unified edge slots for telemetry [`DotState`](crate::dot::DotState), keyed by
//! [`ChannelMetaData::id`](crate::monitor::ChannelMetaData).
//!
//! # Invariant
//!
//! For each channel telemetry id there must be **at most one** actor setting [`Edge::to`](crate::dot_edge::Edge)
//! ([`ChannelEdgeRole::SetsEdgeTo`], from payload **receive** / `channels_in`) and **at most one** setting
//! [`Edge::from`](crate::dot_edge::Edge) ([`ChannelEdgeRole::SetsEdgeFrom`], from **send** / `channels_out`).
//!
//! In **non-test** builds, merging spotlight metadata that violates this invariant **panics** with a detailed message
//! (see `fatal_endpoint_conflict`). In **test** builds the same situation only logs and returns
//! [`EdgeEndpointConflict`] so unit tests can assert behavior.
//!
//! How to fix mis-wired metadata before that happens:
//! - Ensure every channel is allocated from [`Graph::channel_builder()`](crate::Graph::channel_builder) on the **same**
//!   [`Graph`](crate::Graph) (one shared `channel_count` namespace).
//! - In [`SteadyActorShadow::into_spotlight`](crate::steady_actor_shadow::SteadyActorShadow::into_spotlight), list each wire on
//!   the **receiver** side in rx metadata and on the **sender** side in tx metadata only—never both on the same side
//!   for two different actors sharing one id.
//! - **Shared `SteadyTx` clones** (e.g. multiple actors sending to one DLQ) are valid at runtime, but **only one** actor
//!   should list that transmit end in `tx_meta_data!` / spotlight tx—otherwise two actors both claim the same `from`
//!   endpoint for one telemetry id.

use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use log::{debug, trace};
#[cfg(not(test))]
use log::error;

use crate::ActorName;
use crate::channel_stats::ChannelStatsComputer;
use crate::dot::DotState;
use crate::dot::EDGE_PEN_WIDTH;
use crate::dot_edge::Edge;
use crate::monitor::ChannelMetaData;
use crate::graph_liveliness::ActorIdentity;

/// Declares which endpoint of a unified edge this actor is registering (see module docs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ChannelEdgeRole {
    /// `channels_in`: this actor receives on the channel; sets `DotState.edges[id].to`.
    SetsEdgeTo,
    /// `channels_out`: this actor sends on the channel; sets `DotState.edges[id].from`.
    SetsEdgeFrom,
}

impl ChannelEdgeRole {
    #[inline]
    fn as_endpoint_str(self) -> &'static str {
        match self {
            ChannelEdgeRole::SetsEdgeTo => "to",
            ChannelEdgeRole::SetsEdgeFrom => "from",
        }
    }
}

/// Describes a second actor claiming the same directional endpoint for one channel id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
pub(crate) static EDGE_CONFLICT_DIAG_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
static EDGE_DIAG_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Test-only diagnostic log when two actors collide on one endpoint (non-test builds [`fatal_endpoint_conflict`] instead).
#[cfg(test)]
#[inline]
fn log_endpoint_conflict(details: &EdgeEndpointConflict, meta: &Arc<ChannelMetaData>) {
    EDGE_CONFLICT_DIAG_COUNT.fetch_add(1, Ordering::Relaxed);
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

/// Human-readable explanation for process exit on DOT metadata collision (non-test builds).
#[cfg(not(test))]
fn endpoint_conflict_fatal_message(details: &EdgeEndpointConflict, meta: &ChannelMetaData) -> String {
    let endpoint_expl: &'static str = match details.endpoint {
        "from" => {
            "Endpoint `from`: two different actors registered the same telemetry ChannelMetaData::id as SENDER.\n\
This comes from spotlight transmit metadata (`channels_out`, tx side of `into_spotlight`). \
The unified DOT graph allows only one `from` node per channel id.\n\n"
        }
        "to" => {
            "Endpoint `to`: two different actors registered the same telemetry ChannelMetaData::id as RECEIVER.\n\
This comes from spotlight receive metadata (`channels_in`, rx side of `into_spotlight`). \
The unified DOT graph allows only one `to` node per channel id.\n\n"
        }
        _ => "Internal error: unknown endpoint name for conflict.\n\n",
    };

    let labels_block = if meta.labels.is_empty() {
        "Channel labels: (none on this metadata snapshot). Use ChannelMetaData::id above and locate the channel in \
your `Graph` wiring and in each actor's `tx_meta_data!` / `rx_meta_data!` lists.\n\n"
            .to_string()
    } else {
        format!("Channel labels: {}\n\n", meta.labels.join(", "))
    };

    let show_ty = meta.show_type.unwrap_or("(not set)");

    format!(
        "FATAL: DOT telemetry edge endpoint conflict — unified graph cannot be built.\n\n\
Duplicate telemetry key: ChannelMetaData::id == {}\n\
{}{}\
Second-registration snapshot: partner={:?} bundle_index={:?} connects_sidecar={} show_type={} capacity={}\n\n\
Conflicting actors:\n\
  first claimant (already owned this endpoint): {:?}  actor_id={:?}\n\
  second claimant (this registration):          {:?}  actor_id={}\n\n\
Diagnostics (Arc metadata pointers): existing_claim_meta_arc={:?} new_claim_arc_ptr={:#x}\n\n\
What to check:\n\
- Allocate every Steady channel from ONE `Graph::channel_builder()` chain on ONE `Graph` (single channel id namespace).\n\
- In `SteadyActorShadow::into_spotlight`: each wire's receiver only in rx metadata, sender only in tx metadata — \
never two different actors listing the same side for the same id.\n\
- Shared `SteadyTx` clones (e.g. multiple actors to one DLQ) are valid at runtime; only ONE actor should list that \
transmit end in `tx_meta_data!` / spotlight tx. Omit it from other actors, or route through one dedicated sender actor \
for telemetry.\n",
        details.channel_id,
        labels_block,
        endpoint_expl,
        meta.partner,
        meta.bundle_index,
        meta.connects_sidecar,
        show_ty,
        meta.capacity,
        details.existing,
        details.existing_actor_numeric_id,
        details.new_claimant,
        details.new_claimant_actor_numeric_id,
        details.existing_claim_meta_arc,
        details.new_claim_arc_ptr,
    )
}

#[cfg(not(test))]
#[cold]
#[inline(never)]
fn fatal_endpoint_conflict(details: &EdgeEndpointConflict, meta: &Arc<ChannelMetaData>) -> ! {
    let msg = endpoint_conflict_fatal_message(details, meta.as_ref());
    error!(target: "steady_state::telemetry::dot", "{}", msg);
    panic!("{}", msg);
}

/// Applies one channel's metadata into [`DotState::edges`] (grow slot, assign `from`/`to`, merge labels, maybe init stats).
///
/// Returns [`Some`] when a **different** actor already claimed the same endpoint for this channel id. In **test**
/// builds this only logs diagnostics; in **non-test** builds this **panics** with a detailed message (no return).
///
/// Behaviour after a handled test-only conflict matches the legacy `define_unified_edges` per-item loop (label merge and
/// stats refresh continue). Non-test conflicts do not continue.
#[must_use]
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
                #[cfg(test)]
                {
                    log_endpoint_conflict(&details, meta);
                    conflict = Some(details);
                }
                #[cfg(not(test))]
                fatal_endpoint_conflict(&details, meta);
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
                #[cfg(test)]
                {
                    log_endpoint_conflict(&details, meta);
                    conflict = Some(details);
                }
                #[cfg(not(test))]
                fatal_endpoint_conflict(&details, meta);
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
mod unify_edge_tests {
    use super::*;
    use crate::graph_liveliness::ActorIdentity;

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

    #[test]
    fn duplicate_to_second_actor_returns_conflict() {
        let _lock = EDGE_DIAG_MUTEX.lock().expect("edge diag mutex poisoned");
        EDGE_CONFLICT_DIAG_COUNT.store(0, Ordering::Relaxed);
        let mut st = DotState::default();
        let m = meta_with_id_labels(10, vec![]);
        let a = ActorIdentity::new(101, "A", None);
        let b = ActorIdentity::new(102, "B", None);
        assert!(
            apply_channel_to_unified_edges(&mut st, a, &m, ChannelEdgeRole::SetsEdgeTo, 1000).is_none()
        );
        let c = apply_channel_to_unified_edges(&mut st, b, &m, ChannelEdgeRole::SetsEdgeTo, 1000);
        assert!(c.is_some());
        assert_eq!(EDGE_CONFLICT_DIAG_COUNT.load(Ordering::Relaxed), 1);
        assert_eq!(
            c.unwrap(),
            EdgeEndpointConflict {
                channel_id: 10,
                endpoint: "to",
                existing: a.label,
                existing_actor_numeric_id: Some(a.id),
                existing_claim_meta_arc: Some(Arc::as_ptr(&m) as usize),
                new_claimant: b.label,
                new_claimant_actor_numeric_id: b.id,
                new_claim_arc_ptr: Arc::as_ptr(&m) as usize,
            }
        );
    }

    #[test]
    fn duplicate_from_second_actor_returns_conflict() {
        let _lock = EDGE_DIAG_MUTEX.lock().expect("edge diag mutex poisoned");
        EDGE_CONFLICT_DIAG_COUNT.store(0, Ordering::Relaxed);
        let mut st = DotState::default();
        let m = meta_with_id_labels(11, vec![]);
        let a = ActorIdentity::new(201, "send_a", None);
        let b = ActorIdentity::new(202, "send_b", None);
        assert!(
            apply_channel_to_unified_edges(&mut st, a, &m, ChannelEdgeRole::SetsEdgeFrom, 1000).is_none()
        );
        let c = apply_channel_to_unified_edges(&mut st, b, &m, ChannelEdgeRole::SetsEdgeFrom, 1000);
        assert!(c.is_some());
        assert_eq!(EDGE_CONFLICT_DIAG_COUNT.load(Ordering::Relaxed), 1);
        let d = c.unwrap();
        assert_eq!(d.endpoint, "from");
        assert_eq!(d.existing, a.label);
        assert_eq!(d.new_claimant, b.label);
        assert_eq!(d.existing_actor_numeric_id, Some(a.id));
        assert_eq!(d.new_claimant_actor_numeric_id, b.id);
    }

    #[test]
    fn same_claimant_twice_no_conflict_for_to() {
        let mut st = DotState::default();
        let m = meta_with_id_labels(12, vec!["L1"]);
        let a = ActorIdentity::new(303, "R", None);
        assert!(
            apply_channel_to_unified_edges(&mut st, a, &m, ChannelEdgeRole::SetsEdgeTo, 1000).is_none()
        );
        assert!(
            apply_channel_to_unified_edges(&mut st, a, &m, ChannelEdgeRole::SetsEdgeTo, 1000).is_none()
        );
        assert_eq!(st.edges[12].to, Some(a.label));
        assert!(st.edges[12].from.is_none());
    }

    #[test]
    fn two_node_wire_initializes_stats_once() {
        let mut st = DotState::default();
        let shared = meta_with_id_labels(20, vec!["lane"]);
        let recv = ActorIdentity::new(401, "consumer", None);
        let send = ActorIdentity::new(402, "producer", None);
        let _ = apply_channel_to_unified_edges(&mut st, recv, &shared, ChannelEdgeRole::SetsEdgeTo, 500);
        let _ = apply_channel_to_unified_edges(&mut st, send, &shared, ChannelEdgeRole::SetsEdgeFrom, 500);
        assert_eq!(st.edges[20].from, Some(send.label));
        assert_eq!(st.edges[20].to, Some(recv.label));
        let cap_before = st.edges[20].stats_computer.capacity;
        assert_eq!(cap_before, 8);
        assert_eq!(st.edges[20].partner, shared.partner);
        assert_eq!(st.edges[20].bundle_index, shared.bundle_index);
        assert_eq!(st.edges[20].diag_to_claim_actor_id, Some(recv.id));
        assert_eq!(st.edges[20].diag_from_claim_actor_id, Some(send.id));
        // Idempotent replay (same registrations)
        assert!(
            apply_channel_to_unified_edges(&mut st, recv, &shared, ChannelEdgeRole::SetsEdgeTo, 500).is_none()
        );
        assert!(
            apply_channel_to_unified_edges(&mut st, send, &shared, ChannelEdgeRole::SetsEdgeFrom, 500).is_none()
        );
        assert_eq!(st.edges[20].stats_computer.capacity, cap_before);
    }

    #[test]
    fn sparse_channel_ids_leave_placeholders_between() {
        let mut st = DotState::default();
        let low = meta_with_id_labels(0, vec![]);
        let _ = apply_channel_to_unified_edges(
            &mut st,
            ActorIdentity::new(501, "X", None),
            &low,
            ChannelEdgeRole::SetsEdgeTo,
            100,
        );
        let high = meta_with_id_labels(1000, vec![]);
        let _ = apply_channel_to_unified_edges(
            &mut st,
            ActorIdentity::new(502, "Y", None),
            &high,
            ChannelEdgeRole::SetsEdgeFrom,
            100,
        );
        assert_eq!(st.edges.len(), 1001);
        assert_eq!(st.edges[500].id, usize::MAX);
        assert!(st.edges[500].from.is_none() && st.edges[500].to.is_none());
        assert_eq!(st.edges[1000].id, 1000);
    }

    #[test]
    fn label_union_from_two_metas_same_id() {
        let mut st = DotState::default();
        let m1 = meta_with_id_labels(30, vec!["a", "b"]);
        let m2 = Arc::new(ChannelMetaData {
            labels: vec!["b", "c"],
            ..(*m1).clone()
        });
        let n = ActorIdentity::new(600, "n", None);
        let _ = apply_channel_to_unified_edges(&mut st, n, &m1, ChannelEdgeRole::SetsEdgeTo, 100);
        let _ = apply_channel_to_unified_edges(&mut st, n, &m2, ChannelEdgeRole::SetsEdgeTo, 100);
        assert_eq!(st.edges[30].ctl_labels, vec!["a", "b", "c"]);
    }

    #[test]
    fn self_loop_same_actor_in_and_out_same_id() {
        let mut st = DotState::default();
        let m = meta_with_id_labels(40, vec![]);
        let echo = ActorIdentity::new(700, "echo", None);
        let _ = apply_channel_to_unified_edges(&mut st, echo, &m, ChannelEdgeRole::SetsEdgeTo, 100);
        let _ = apply_channel_to_unified_edges(&mut st, echo, &m, ChannelEdgeRole::SetsEdgeFrom, 100);
        let e = &st.edges[40];
        assert_eq!(e.from, Some(echo.label));
        assert_eq!(e.to, Some(echo.label));
        assert_eq!(e.stats_computer.capacity, 8);
    }

    #[test]
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
