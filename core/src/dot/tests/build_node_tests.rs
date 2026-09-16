//! DOT node/rank build tests; split from build_tests.rs for the 1,200-line budget.

// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use super::test_dot_frames;
// ss[related telemetry.dot-export]
use crate::monitor::ActorStatus;


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

