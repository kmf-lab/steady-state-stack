// ss[related graph.for-testing]
use super::super::{
    effective_block_until_stopped_timeout, ActorIdentity, Graph, GraphBuilder, GraphLiveliness,
    GraphLivelinessState, ShutdownVote, VoterStatus,
};
use crate::core_exec;
use crate::{ScheduleAs, SteadyActor};
use futures::lock::Mutex as FutMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn start_with_timeout_empty_graph_reaches_running() {
    let mut graph = GraphBuilder::for_testing().build(());
    assert!(graph.start_with_timeout(Duration::from_secs(1)));
    assert!(graph
        .runtime_state
        .read()
        .is_in_state(&[GraphLivelinessState::Running]));
}


#[test]
// ss[verify graph.for-testing]
fn graph_builder_for_testing_sets_test_flags() {
    let b = GraphBuilder::for_testing();
    assert!(b.is_for_testing);
    assert!(b.block_fail_fast);
}

#[test]
// ss[verify graph.for-testing]
fn graph_builder_chain_sets_optional_fields() {
    let mut names = std::collections::HashSet::new();
    names.insert("PIPE");
    let b = GraphBuilder::for_testing()
        .with_iouring_queue_length(64)
        .with_default_actor_stack_size(1_048_576)
        .with_block_fail_fast()
        .with_test_pipeline_internal_behavior_names(names);
    assert_eq!(b.iouring_queue_length, 64);
    assert!(b.block_fail_fast);
    assert!(b.test_pipeline_internal_names.contains("PIPE"));
    let g = Graph::internal_new((), b);
    assert_eq!(g.default_stack_size, Some(1_048_576));
    assert!(g.test_pipeline_internal_names.contains("PIPE"));
}

#[test]
// ss[verify graph.for-testing]
fn graph_telemetry_rate_below_minimum_clamped_in_test_mode() {
    let b = GraphBuilder::for_testing().with_telemtry_production_rate_ms(25);
    assert_eq!(b.telemtry_production_rate_ms, super::super::MIN_MS_RATE);
}

#[test]
// ss[verify graph.for-testing]
fn graph_builder_extended_chain_sets_graph_fields() {
    let b = GraphBuilder::for_testing()
        .with_bundle_floor_size(77)
        .with_aggregation_threshold(88)
        .with_telemetry_metric_features(true)
        .with_telemtry_production_rate_ms(200)
        .with_telemetry_colors("#111111", "#222222")
        .with_shutdown_barrier(3);
    let g = Graph::internal_new((), b);
    assert_eq!(g.bundle_floor_size, 88); // with_aggregation_threshold last wins for same field
    assert_eq!(g.telemetry_production_rate_ms, 200);
    assert_eq!(
        g.telemetry_colors.as_ref().map(|c| (c.0.as_str(), c.1.as_str())),
        Some(("#111111", "#222222"))
    );
    assert!(g.shutdown_barrier.is_some());
}


#[test]
// ss[verify graph.for-testing]
fn graph_telemetry_rate_respects_minimum_when_telemetry_on() {
    let b = GraphBuilder::for_testing()
        .with_telemetry_metric_features(true)
        .with_telemtry_production_rate_ms(50);
    let g = Graph::internal_new((), b);
    assert_eq!(g.telemetry_production_rate_ms, super::super::MIN_MS_RATE);
}


#[test]
// ss[verify graph.for-testing]
fn graph_args_roundtrip() {
    let g = Graph::internal_new(42_u64, GraphBuilder::for_testing());
    assert_eq!(*g.args::<u64>().expect("typed args"), 42);
}

// ss[verify distributed.media-driver-testing]

#[test]
fn graph_aeron_init_timeouts_depend_on_test_mode() {
    let (t_wait, t_retry) = Graph::aeron_init_timeouts(true);
    let (p_wait, p_retry) = Graph::aeron_init_timeouts(false);
    assert!(t_wait < p_wait);
    assert!(t_retry > p_retry);
}


#[test]
// ss[verify graph.for-testing]
fn graph_stage_manager_guard_derefs_to_initialized_hub() {
    let g = Graph::internal_new((), GraphBuilder::for_testing());
    let guard = g.stage_manager();
    let dbg = format!("{:?}", *guard);
    assert!(dbg.contains("SideChannelHub") || dbg.contains("node"));
}

#[test]
// ss[verify graph.for-testing]
fn graph_builder_build_returns_startable_graph() {
    let mut graph = GraphBuilder::for_testing().build(());
    assert!(graph.start_with_timeout(Duration::from_secs(1)));
    graph.request_shutdown();
    graph
        .block_until_stopped(Duration::from_secs(2))
        .expect("shutdown after build");
}

#[test]
// ss[verify graph.for-testing]
fn graph_builder_telemetry_features_off_keeps_io_driver_disabled() {
    let b = GraphBuilder::for_testing().with_telemetry_metric_features(false);
    assert!(!b.telemetry_metric_features);
    assert!(!b.enable_io_driver);
}

#[test]
// ss[verify graph.for-testing]
fn graph_builder_empty_pipeline_names_clears_allowlist() {
    let names = std::collections::HashSet::new();
    let b = GraphBuilder::for_testing().with_test_pipeline_internal_behavior_names(names);
    assert!(b.test_pipeline_internal_names.is_empty());
    let g = Graph::internal_new((), b);
    assert!(g.test_pipeline_internal_names.is_empty());
}

// ss[verify graph.shutdown.veto]
// ss[verify graph.panic-restart]
// ss[verify philosophy.cooperative-liveliness]
// ss[verify actor.is-running-loop]
// ss[verify actor.shutdown-veto]
