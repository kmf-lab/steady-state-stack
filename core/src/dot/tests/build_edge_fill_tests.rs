//! DOT edge avg-fill build tests; split from build_tests.rs for the 1,200-line budget.

// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use super::test_dot_frames;


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
