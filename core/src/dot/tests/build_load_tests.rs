//! DOT graph-share Avg load tests (0.3.1); split from build_tests.rs for the 1,200-line budget.

// ss[related telemetry.dot-export]
use super::super::*;
// ss[related telemetry.dot-export]
use crate::monitor::ActorStatus;


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

