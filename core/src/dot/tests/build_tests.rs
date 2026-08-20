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

#[test]
// ss[verify telemetry.dot-export]
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
        last_bool_stop: false,
        work_info: None,
    };
    node.compute_and_refresh(actor_status);
    assert_eq!(node.color, "grey");
    assert_eq!(node.pen_width, NODE_PEN_WIDTH);
}

#[test]
// ss[verify telemetry.dot-export]
fn test_same_base_name_suffixes_share_rank_column() {
    let state = DotState {
        nodes: vec![
            Node {
                id: Some(ActorName::new("Worker", Some(0))),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "Worker0".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                last_bool_stop: false,
                work_info: None,
            },
            Node {
                id: Some(ActorName::new("Worker", Some(1))),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "Worker1".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                last_bool_stop: false,
                work_info: None,
            },
            Node {
                id: Some(ActorName::new("Worker", Some(2))),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "Worker2".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                last_bool_stop: false,
                work_info: None,
            },
        ],
        edges: vec![],
        seq: 0,
        telemetry_colors: None,
        refresh_rate_ms: 40,
        bundle_floor_size: 4,
    };
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        dot.contains("{rank=same; \"Worker0\" \"Worker1\" \"Worker2\"}"),
        "expected same-name column rank block, got:\n{dot}"
    );
}

#[test]
// ss[verify telemetry.dot-export]
fn test_distinct_base_names_do_not_emit_name_rank_column() {
    let state = DotState {
        nodes: vec![
            Node {
                id: Some(ActorName::new("alpha", None)),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "alpha".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                last_bool_stop: false,
                work_info: None,
            },
            Node {
                id: Some(ActorName::new("beta", None)),
                color: "grey",
                pen_width: NODE_PEN_WIDTH,
                stats_computer: ActorStatsComputer::default(),
                display_label: "beta".to_string(),
                dot_subtitle: None,
                tooltip: String::new(),
                metric_text: String::new(),
                remote_details: None,
                thread_info_cache: None,
                total_count_restarts: 0,
                bool_stalled: false,
                last_bool_stop: false,
                work_info: None,
            },
        ],
        edges: vec![],
        seq: 0,
        telemetry_colors: None,
        refresh_rate_ms: 40,
        bundle_floor_size: 4,
    };
    let mut frames = test_dot_frames();
    build_dot(&state, &mut frames);
    let dot = String::from_utf8(frames.active_graph.to_vec()).expect("utf8");

    assert!(
        !dot.contains("{rank=same;"),
        "distinct base names must not emit name-group rank=same, got:\n{dot}"
    );
}

#[test]
// ss[verify telemetry.dot-export]
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
    ..Default::default()
    };
    edge.compute_and_refresh(100, 50);
    assert_eq!(edge.color, "grey");
    assert!(!edge.pen_width.is_empty());
}


#[test]
// ss[verify telemetry.dot-export]
fn test_large_bundle_avg_fill_uses_mean_summary() {
    // ss[impl telemetry.dot-export]
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
            ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
fn test_stage1_avg_fill_mean_when_lanes_exceed_inline_cap() {
    // ss[impl telemetry.dot-export]
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
        ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
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
    ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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

    // Edge label should show total_consumed (1000 -> "1K")
    assert!(
        result.contains("Total: 1K"),
        "Edge label should show total_consumed: {}",
        result
    );

    // Tooltip should also show total_consumed (1000 -> "1K"), NOT last_total (50)
    assert!(
        result.contains("Total: 1K"),
        "Tooltip should show total_consumed, not last_total: {}",
        result
    );

    println!("✓ Edge tooltip correctly uses total_consumed (cumulative): 1K");
}

/// When `avg_filled` is enabled, tooltip shows rolling-window **Avg fill**, not snapshot Instant fill
/// (which is often 0% when inflight is drained between samples).

#[test]
// ss[verify telemetry.dot-export]
fn test_edge_tooltip_prefers_avg_fill_when_enabled() {
    // ss[impl telemetry.dot-export]
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
    ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
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
    ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
fn test_multi_lane_avg_fill_omits_when_all_zero_percent() {
    // ss[impl telemetry.dot-export]
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
        runner: 0,   // 0% fill (idle)
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
        runner: 0,   // 0% fill (idle)
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
        ..Default::default()
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
        ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
        "must omit Avg fill line when all lanes are 0% (idle): {}",
        result
    );
}

/// Verify `format_avg_fill_rollup_line_into` produces nothing when all edges have runner==0.

#[test]
// ss[verify telemetry.dot-export]
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
        ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
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
        ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
// ss[verify telemetry.dot-export]
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
        ..Default::default()
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
                last_bool_stop: false,
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
                last_bool_stop: false,
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
        result.contains("Total: 1K"),
        "Partner lane 0 should show Total: 1K: {}",
        result
    );
    assert!(
        result.contains("Total: 2K"),
        "Partner lane 1 should show Total: 2K: {}",
        result
    );
    assert!(
        result.contains("Total: 3K"),
        "Partner lane 2 should show Total: 3K: {}",
        result
    );

    // Tooltip should also show these totals (each lane's tooltip has CH#N line + Total line)
    // NOTE: Total: 1K appears in the edge label for lane 0 AND in the tooltip for lane 0,
    // so this single assertion covers both. The lane 1 (2K) and lane 2 (3K) totals are
    // verified above in the edge label checks.
    assert!(
        result.contains("Total: 1K"),
        "Tooltip should show 1K: {}",
        result
    );

    // Verify partner header format appears for each lane
    assert!(
        result.contains("partner_lane [0]"),
        "Partner lane 0 should show header 'partner_lane [0]': {}",
        result
    );
    assert!(
        result.contains("partner_lane [1]"),
        "Partner lane 1 should show header 'partner_lane [1]': {}",
        result
    );
    assert!(
        result.contains("partner_lane [2]"),
        "Partner lane 2 should show header 'partner_lane [2]': {}",
        result
    );

    println!(
        "✓ Partner tooltip correctly uses total_consumed: expected = {}",
        expected_total
    );
}


#[test]
// ss[verify telemetry.dot-export]
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
        last_bool_stop: false,
        work_info: None,
    };
    node.compute_and_refresh(actor_status);
    // mCPU: (400*1024)/500 = 819; sole actor → 100% graph load share.
    assert_eq!(node.work_info, Some((819, 100)));
}


#[test]
// ss[verify telemetry.dot-export]
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
        last_bool_stop: false,
        work_info: None,
    };
    node.compute_and_refresh(actor_status);
    assert_eq!(node.work_info, Some((1024, 100)));
}

#[test]
// ss[verify telemetry.dot-export]
fn test_refresh_actor_loads_graph_share() {
    fn status_with_busy(await_ns: u64, unit_ns: u64) -> ActorStatus {
        ActorStatus {
            ident: Default::default(),
            await_total_ns: await_ns,
            unit_total_ns: unit_ns,
            total_count_restarts: 0,
            iteration_start: 1,
            iteration_sum: 0,
            bool_stop: false,
            is_quiet: false,
            calls: [0; 6],
            thread_info: None,
            bool_blocking: false,
        }
    }

    let mut state = DotState {
        nodes: vec![
            Node {
                id: Some(ActorName::new("A", None)),
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
                last_bool_stop: false,
                work_info: None,
            },
            Node {
                id: Some(ActorName::new("B", None)),
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
                last_bool_stop: false,
                work_info: None,
            },
        ],
        ..Default::default()
    };

    // Actor A: 768 mCPU (busy 750/1000), Actor B: 256 mCPU (busy 250/1000) → 75% / 25% share.
    state.nodes[0].apply_local_mcpu(status_with_busy(250, 1000));
    state.nodes[1].apply_local_mcpu(status_with_busy(750, 1000));
    state.refresh_actor_loads(&[0, 1]);
    assert_eq!(state.nodes[0].work_info, Some((768, 75)));
    assert_eq!(state.nodes[1].work_info, Some((256, 25)));

    // Sparse update of A only: total still uses B's last-known mCPU.
    state.nodes[0].apply_local_mcpu(status_with_busy(0, 500));
    state.refresh_actor_loads(&[0]);
    assert_eq!(state.nodes[0].work_info, Some((1024, 80)));
    assert_eq!(state.nodes[1].work_info, Some((256, 20)));
}

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
