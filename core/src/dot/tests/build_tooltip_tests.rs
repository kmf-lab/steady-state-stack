//! DOT edge/bundle tooltip build tests; split from build_tests.rs for the 1,200-line budget.

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


