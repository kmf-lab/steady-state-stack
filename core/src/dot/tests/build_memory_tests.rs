//! DOT memory label/tooltip tests; split from build_tests.rs for the 1,200-line budget.

// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use super::test_dot_frames;
// ss[impl telemetry.dot-export]
use crate::dot_unify::ChannelEdgeRole;
// ss[related telemetry.dot-export]
use crate::monitor::{ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData};
// ss[impl telemetry.dot-export]
use crate::telemetry::metrics_server::async_write_all;
// ss[impl telemetry.dot-export]
use bytes::BytesMut;
// ss[related telemetry.dot-export]
use std::fs::remove_file;
// ss[impl telemetry.dot-export]
use std::path::PathBuf;
// ss[impl telemetry.dot-export]
use std::sync::Arc;
// ss[related telemetry.dot-export]
use std::time::Instant;


/// Builds a minimal two-node `DotState` wrapping the given edges for memory display tests.
// ss[related telemetry.dot-export]
fn memory_test_state(edges: Vec<Edge>, bundle_floor_size: usize) -> DotState {
    let from = ActorName::new("from", None);
    let to = ActorName::new("to", None);
    let mk_node = |name: ActorName| Node {
        id: Some(name),
        color: "grey",
        pen_width: NODE_PEN_WIDTH,
        stats_computer: ActorStatsComputer::default(),
        display_label: name.name.to_string(),
        dot_subtitle: None,
        tooltip: String::new(),
        metric_text: String::new(),
        remote_details: None,
        thread_info_cache: None,
        total_count_restarts: 0,
        bool_stalled: false,
        last_bool_stop: false,
        work_info: None,
    };
    DotState {
        nodes: vec![mk_node(from), mk_node(to)],
        edges,
        seq: 0,
        telemetry_colors: None,
        refresh_rate_ms: 40,
        bundle_floor_size,
    }
}

/// Builds one edge with memory display enabled: capacity 100 × 8-byte items = 800B ring.
// ss[related telemetry.dot-export]
fn memory_edge(id: usize, partner: Option<&'static str>, bundle_index: Option<usize>) -> Edge {
    memory_edge_with_ring_dyn(id, 100 * 8, 0, partner, bundle_index)
}

/// Builds one edge with explicit ring/dyn footprints for partner/bundle rollup tests.
// ss[related telemetry.dot-export]
fn memory_edge_with_ring_dyn(
    id: usize,
    ring_footprint: usize,
    dynamic_footprint: usize,
    partner: Option<&'static str>,
    bundle_index: Option<usize>,
) -> Edge {
    let mut stats = ChannelStatsComputer::default();
    stats.ring_memory_footprint = ring_footprint;
    stats.dynamic_memory_footprint = dynamic_footprint;
    stats.show_memory = true;
    Edge {
        id,
        from: Some(ActorName::new("from", None)),
        to: Some(ActorName::new("to", None)),
        color: "green",
        sidecar: false,
        pen_width: "1".to_string(),
        saturation_score: 0.0,
        ctl_labels: vec![],
        stats_computer: stats,
        display_label: String::new(),
        metric_text: String::new(),
        partner,
        bundle_index,
        ..Default::default()
    }
}



/// Single plain channel with `show_memory` must show its footprint on the edge label.
#[test]
// ss[verify telemetry.dot-export]
// ss[verify channel.memory-usage-telemetry]
fn test_single_edge_shows_memory_on_label() {
    let state = memory_test_state(vec![memory_edge(0, None, None)], 4);
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        dot.contains("Memory: 800B ring"),
        "single edge label must show ring memory footprint, got:\n{dot}"
    );
}

/// Single plain channel without `show_memory` must NOT show memory.


/// Single plain channel without `show_memory` must NOT show memory.
#[test]
// ss[verify telemetry.dot-export]
// ss[verify channel.memory-usage-telemetry]
fn test_single_edge_omits_memory_when_disabled() {
    let mut edge = memory_edge(0, None, None);
    edge.stats_computer.show_memory = false;
    let state = memory_test_state(vec![edge], 4);
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        !dot.contains("Memory:"),
        "memory must be hidden when show_memory is false, got:\n{dot}"
    );
}

/// Small partner group: header shows combined footprint; tooltip shows per-lane memory.


/// Small partner group: header shows combined footprint; tooltip shows per-lane memory.
#[test]
// ss[verify telemetry.dot-export]
// ss[verify channel.memory-usage-telemetry]
fn test_partner_group_memory_combined_and_per_lane() {
    // Two lanes sharing partner+bundle_index merge into one partner edge.
    // Each lane: 100 × 8B = 800B → combined 1.6K (1600B).
    let edges = vec![
        memory_edge(0, Some("stream"), Some(0)),
        memory_edge(1, Some("stream"), Some(0)),
    ];
    let state = memory_test_state(edges, 8); // floor above 2 → rendered as a single partner edge
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        dot.contains("stream [0] (1KB ring)"),
        "partner header must show combined ring memory (1600B compresses to 1KB ring), got:\n{dot}"
    );
    // Per-lane breakdown appears in the tooltip (800B ring per lane).
    let lane_memory_count = dot.matches("Ring: 800B ring").count();
    assert!(
        lane_memory_count >= 2,
        "tooltip must show per-lane memory for both lanes (found {lane_memory_count}):\n{dot}"
    );
}

/// Bundle of partnered edges ≥ floor: header and tooltip show summed memory.


/// Bundle of partnered edges ≥ floor: header and tooltip show summed memory.
#[test]
// ss[verify telemetry.dot-export]
// ss[verify channel.memory-usage-telemetry]
fn test_bundle_memory_header_and_tooltip() {
    // 4 partner groups (bundle_index 0..3), each 2 lanes → 8 edges, 8 × 800B = 6.4K.
    let mut edges = Vec::new();
    let mut id = 0;
    for bi in 0..4 {
        for _lane in 0..2 {
            edges.push(memory_edge(id, Some("P"), Some(bi)));
            id += 1;
        }
    }
    let state = memory_test_state(edges, 2); // 4 groups ≥ floor 2 → bundle render
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        dot.contains("P: 4x (6KB ring)"),
        "bundle header must show summed ring memory (6400B compresses to 6KB ring), got:\n{dot}"
    );
    assert!(
        dot.contains("Memory: 6KB ring"),
        "bundle tooltip must show summed ring memory, got:\n{dot}"
    );
}

/// T12 JoinWire-scale partner rollup: small ring + large dyn ceiling, not TB².


/// T12 JoinWire-scale partner rollup: small ring + large dyn ceiling, not TB².
#[test]
// ss[verify telemetry.dot-export]
// ss[verify channel.memory-usage-telemetry]
fn test_partner_join_wire_scale_memory_not_depth_squared() {
    const N_OUT: usize = 12;
    const DEPTH: usize = 16384;
    const PER_SLOT: usize = 256024;
    const MSG_SIZE: usize = 24;
    let ring_per_lane = DEPTH * MSG_SIZE;
    let dyn_per_lane = DEPTH * PER_SLOT;
    let partner_dyn_total = N_OUT * dyn_per_lane;

    let edges: Vec<Edge> = (0..N_OUT)
        .map(|id| {
            memory_edge_with_ring_dyn(id, ring_per_lane, dyn_per_lane, Some("JoinWire"), Some(0))
        })
        .collect();
    let state = memory_test_state(edges, 16);
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        dot.contains("JoinWire [0] (4MB ring + 50GB dyn)"),
        "partner header must show small ring + ~50GB dyn ceiling, got:\n{dot}"
    );
    assert!(
        !dot.contains("825TB"),
        "DEPTH² bug would show TB-scale (825TB), got:\n{dot}"
    );
    assert!(
        !dot.contains("BB"),
        "memory labels must use GB not BB, got:\n{dot}"
    );

    let buggy_total = N_OUT * DEPTH * (DEPTH * PER_SLOT);
    assert!(
        buggy_total > 10_000_000_000_000,
        "sanity: DEPTH² formula must be TB-scale"
    );
    assert!(
        partner_dyn_total < 100_000_000_000,
        "correct T12 dyn ceiling must be under 100GB"
    );
}
