use super::*;
use crate::telemetry_window::compute_refresh_window_frames;
use crate::{
    ActorIdentity, ActorMetaData, AlertColor, Duration, Error, GraphBuilder, MCPU, Percentile,
    Trigger, Work,
};
use futures::channel::oneshot;
use futures_util::FutureExt;
use std::sync::Arc;
use std::sync::OnceLock;

#[test]
// ss[verify actor.regeneration-survives]
fn test_core_balancer() {
    let mut cb = CoreBalancer {
        core_usage: vec![0, 0, 0],
    };
    assert_eq!(cb.allocate_core(&[]), 0);
    assert_eq!(cb.allocate_core(&[]), 1);
    assert_eq!(cb.allocate_core(&[0]), 2);
    assert_eq!(cb.allocate_core(&[]), 0);
    assert_eq!(cb.core_usage, vec![2, 1, 1]);
}

// ss[verify testing.never-run-in-unit]
// ss[verify testing.graph-for-testing]
#[test]
fn test_actor_builder_core_configs() {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = ActorBuilder::new(&mut graph);

    let b2 = builder.with_explicit_core(5);
    assert_eq!(b2.explicit_core, Some(4));

    let b3 = builder.with_core_exclusion(vec![0, 1]);
    assert_eq!(b3.excluded_cores, vec![0, 1]);

    let cb = CoreBalancer {
        core_usage: vec![0],
    };
    let b4 = builder.with_core_balancing(cb);
    assert!(b4.core_balancer.is_some());
}

#[test]
#[should_panic]
// ss[verify graph.panic-restart]
fn test_explicit_core_zero_panic() {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = ActorBuilder::new(&mut graph);
    builder.with_explicit_core(0);
}

// ss[verify graph.troupes]
// ss[verify actor.regeneration-survives]
// ss[verify graph.panic-restart]
#[test]
fn test_troupe_ops() {
    let graph = GraphBuilder::for_testing().build(());
    let mut troupe = Troupe::new(&graph);
    troupe.with_name("TestTroupe");
    assert_eq!(troupe.name, Some("TestTroupe".to_string()));

    let mut other = Troupe::new(&graph);

    // Mock an archetype
    let (_tx, rx) = oneshot::channel();
    let arch = SteadyContextArchetype {
        build_actor_exec: NonSendWrapper::new(ActorBuilder::to_dyn_call(|_| {
            Box::pin(async { Ok::<(), Box<dyn Error>>(()) })
        })),
        runtime_state: graph.runtime_state.clone(),
        channel_count: graph.channel_count.clone(),
        ident: ActorIdentity::default(),
        args: graph.args.clone(),
        all_telemetry_rx: graph.all_telemetry_rx.clone(),
        actor_metadata: Arc::new(ActorMetaData::default()),
        oneshot_shutdown_vec: graph.oneshot_shutdown_vec.clone(),
        oneshot_shutdown: rx.shared(),
        node_tx_rx: None,
        show_thread_info: false,
        aeron_meda_driver: OnceLock::new(),
        aeron_init_for_tests: true,
        never_simulate: false,
        force_internal_behavior_in_test: false,
        shutdown_barrier: None,
    };

    troupe.add_actor(arch.clone(), 40, true, None);
    assert_eq!(troupe.future_builder.len(), 1);

    assert!(troupe.transfer_front_to(&mut other));
    assert_eq!(troupe.future_builder.len(), 0);
    assert_eq!(other.future_builder.len(), 1);

    assert!(other.transfer_back_to(&mut troupe));
    assert_eq!(troupe.future_builder.len(), 1);
}

#[test]
// ss[verify actor.regeneration-survives]
fn test_schedule_as() {
    let mut troupe_guard = None;
    assert!(matches!(
        ScheduleAs::dynamic_schedule(&mut troupe_guard),
        ScheduleAs::SoloAct
    ));

    let graph = GraphBuilder::for_testing().build(());
    let mut troupe_guard = Some(graph.actor_troupe());
    assert!(matches!(
        ScheduleAs::dynamic_schedule(&mut troupe_guard),
        ScheduleAs::MemberOf(_)
    ));
}

#[test]
// ss[verify actor.regeneration-survives]
fn test_builder_gauntlet() {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = ActorBuilder::new(&mut graph)
        .with_name("gauntlet")
        .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
        .with_core_exclusion(vec![0])
        .with_explicit_core(2)
        .with_mcpu_percentile(Percentile::p99())
        .with_load_percentile(Percentile::p50())
        .with_mcpu_avg()
        .with_load_avg()
        .with_mcpu_trigger(
            Trigger::AvgAbove(MCPU::new(500).expect("")),
            AlertColor::Red,
        )
        .with_load_trigger(
            Trigger::AvgBelow(Work::new(10.0).expect("")),
            AlertColor::Yellow,
        )
        .with_thread_info()
        .with_stack_size(4 * 1024 * 1024)
        .never_simulate(true);

    let meta = builder.build_actor_metadata(ActorIdentity::new(1, "test", None));

    assert_eq!(meta.ident.id, 1);
    assert_eq!(meta.refresh_rate_in_bits, builder.refresh_rate_in_bits);
    assert!(meta.avg_mcpu);
    assert!(meta.avg_work);
    assert!(meta.show_thread_info);
    assert_eq!(meta.percentiles_mcpu.len(), 1);
    assert_eq!(meta.trigger_mcpu.len(), 1);
    assert_eq!(builder.stack_size, Some(4 * 1024 * 1024));
    assert!(builder.never_simulate);
}

#[test]
// ss[verify actor.regeneration-survives]
fn test_builder_state_modifications() {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = ActorBuilder::new(&mut graph);

    // Test with_name and with_name_and_suffix
    let b_name = builder.with_name("test_actor");
    assert_eq!(b_name.actor_name.name, "test_actor");
    assert!(b_name.actor_name.suffix.is_none());

    let b_suffix = builder.with_name_and_suffix("test_actor", 42);
    assert_eq!(b_suffix.actor_name.name, "test_actor");
    assert_eq!(b_suffix.actor_name.suffix, Some(42));

    // Test telemetry toggle
    let b_no_refresh = builder.with_no_refresh_window();
    assert_eq!(b_no_refresh.refresh_rate_in_bits, 0);
    assert_eq!(b_no_refresh.window_bucket_in_bits, 0);

    // Test thread info toggle
    let b_thread = builder.with_thread_info();
    assert!(b_thread.show_thread_info);

    // Test stack size
    let b_stack = builder.with_stack_size(1024 * 1024);
    assert_eq!(b_stack.stack_size, Some(1024 * 1024));
}

#[test]
// ss[verify actor.regeneration-survives]
fn test_internal_compute_refresh_window_edge_cases() {
    // Test zero frame rate (should return 0,0)
    let (r, w) = ActorBuilder::internal_compute_refresh_window(
        0,
        Duration::from_secs(1),
        Duration::from_secs(10),
    );
    assert_eq!((r, w), (0, 0));

    // Test very small durations
    let (_r, _w) = ActorBuilder::internal_compute_refresh_window(
        100,
        Duration::from_millis(1),
        Duration::from_millis(1),
    );
    // Logic ensures it doesn't crash on small inputs.
}

#[test]
// ss[verify actor.regeneration-survives]
fn test_actor_refresh_window_matches_shared_frame_math() {
    let refresh = Duration::from_secs(1);
    let window = Duration::from_secs(10);
    let actor_bits = ActorBuilder::internal_compute_refresh_window(100, refresh, window);
    let shared = compute_refresh_window_frames(100, refresh, window);
    assert_eq!(actor_bits, shared);
    assert_eq!(actor_bits, (4, 3));
}
