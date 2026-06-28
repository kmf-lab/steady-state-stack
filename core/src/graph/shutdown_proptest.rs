//! Property tests for graph shutdown helpers and clean-stop invariants.

use super::{effective_block_until_stopped_timeout, watch_shutdown};
use crate::core_exec;
use crate::graph::state::GraphLivelinessState;
use crate::graph::GraphLiveliness;
use crate::ss_proptest;
use crate::{
    ActorIdentity, GraphBuilder, ScheduleAs, ShutdownVote, SteadyActor, SteadyActorShadow,
    VoterStatus,
};
use futures::lock::Mutex as FutMutex;
use proptest::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn new_liveliness(actors: usize) -> Arc<parking_lot::RwLock<GraphLiveliness>> {
    let oss = Arc::new(FutMutex::new(Vec::new()));
    let actors_count = Arc::new(AtomicUsize::new(actors));
    let catalog = Arc::new(parking_lot::RwLock::new(Vec::new()));
    Arc::new(parking_lot::RwLock::new(GraphLiveliness::new(
        oss,
        actors_count,
        catalog,
    )))
}

fn setup_stop_requested(
    l: &Arc<parking_lot::RwLock<GraphLiveliness>>,
    voters: usize,
    yes_votes: usize,
) {
    {
        let mut w = l.write();
        for i in 0..voters {
            w.register_voter(ActorIdentity::new(i, "v", None));
        }
        w.building_to_running();
    }
    core_exec::block_on(GraphLiveliness::internal_request_shutdown(l.clone()));
    for i in 0..yes_votes {
        let ident = ActorIdentity::new(i, "v", None);
        let _ = l.read().is_running(ident, || true);
    }
}

ss_proptest! {

    /// Property: effective block timeout is at least clean timeout and telemetry floor.
    #[test]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_effective_block_until_stopped_timeout_monotonic(
        clean_ms in 0u64..60_000,
        telemetry_ms in 1u64..10_000,
    ) {
        let clean = Duration::from_millis(clean_ms);
        let got = effective_block_until_stopped_timeout(clean, telemetry_ms);
        let floor = Duration::from_millis(3 * telemetry_ms);
        prop_assert!(got >= clean);
        prop_assert!(got >= floor);
    }

    /// Property: empty testing graph shuts down cleanly after start.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_empty_graph_clean_shutdown(timeout_ms in 50u64..2_000) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let result = graph.block_until_stopped(Duration::from_millis(timeout_ms));
        prop_assert!(result.is_ok());
    }



    /// Property: `never_simulate(true)` edge actors use internal behavior and exit without staging.
    #[test]
    #[ignore] //too slow
    // ss[verify testing.never-run-in-unit]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_never_simulate_edge_exits_without_stage_manager(
        timeout_ms in 200u64..2_000,
    ) {
        use std::thread::sleep;
        let mut graph = GraphBuilder::for_testing().build(());
        graph
            .actor_builder()
            .with_name("NEVER_SIM")
            .never_simulate(true)
            .build(
                |ctx: SteadyActorShadow| async move {
                    let mut actor = ctx.into_spotlight([], []);
                    while actor.is_running(|| true) {
                        actor.wait_periodic(Duration::from_millis(5)).await;
                    }
                    Ok(())
                },
                ScheduleAs::SoloAct,
            );
        graph.start();
        sleep(Duration::from_millis(40));
        graph.request_shutdown();
        let result = graph.block_until_stopped(Duration::from_millis(timeout_ms));
        prop_assert!(result.is_ok());
    }

    /// Property: `watch_shutdown` fails if shutdown was never requested before timeout.
    #[test]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_watch_shutdown_times_out_without_stop_requested(
        voters in 1usize..4,
    ) {
        let rs = new_liveliness(voters);
        {
            let mut w = rs.write();
            for i in 0..voters {
                w.register_voter(ActorIdentity::new(i, "v", None));
            }
            w.building_to_running();
        }
        let started = Instant::now() - Duration::from_secs(5);
        let err = watch_shutdown(
            Duration::from_millis(1),
            started,
            rs.clone(),
            Duration::from_millis(1),
        )
        .expect_err("shutdown never requested");
        prop_assert!(err.to_string().contains("uncleanly"));
        prop_assert_eq!(rs.read().state.clone(), GraphLivelinessState::StoppedUncleanly);
    }

    /// Property: `watch_shutdown` succeeds when every registered voter accepts.
    #[test]
    // ss[verify graph.shutdown.accept]
    // ss[verify verify.process.proptest]
    fn proptest_watch_shutdown_all_voters_accept(
        voters in 1usize..8,
    ) {
        let rs = new_liveliness(voters);
        setup_stop_requested(&rs, voters, voters);
        watch_shutdown(
            Duration::from_secs(5),
            Instant::now(),
            rs.clone(),
            Duration::from_millis(1),
        )
        .expect("clean shutdown");
        prop_assert_eq!(rs.read().state.clone(), GraphLivelinessState::Stopped);
    }

    /// Property: `watch_shutdown` fails when any voter vetoes past the timeout.
    #[test]
    // ss[verify graph.shutdown.veto]
    // ss[verify verify.process.proptest]
    fn proptest_watch_shutdown_veto_becomes_unclean(
        voters in 2usize..6,
        veto_slot in 0usize..6,
    ) {
        let veto_index = veto_slot % voters;
        let rs = new_liveliness(voters);
        setup_stop_requested(&rs, voters, voters - 1);
        {
            let ident = ActorIdentity::new(veto_index, "v", None);
            let _ = rs.read().is_running(ident, || false);
        }
        let started = Instant::now() - Duration::from_secs(2);
        let err = watch_shutdown(
            Duration::from_millis(1),
            started,
            rs.clone(),
            Duration::from_millis(1),
        )
        .expect_err("unclean shutdown");
        prop_assert!(err.to_string().contains("uncleanly"));
        prop_assert_eq!(rs.read().state.clone(), GraphLivelinessState::StoppedUncleanly);
    }

    /// Property: pre-seeded unanimous ballots reach Stopped without waiting on actors.
    #[test]
    // ss[verify graph.shutdown.accept]
    // ss[verify verify.process.proptest]
    fn proptest_watch_shutdown_preloaded_votes(
        voters in 1usize..12,
    ) {
        let rs = new_liveliness(0);
        {
            let mut w = rs.write();
            w.state = GraphLivelinessState::StopRequested;
            w.votes = Arc::new(
                (0..voters)
                    .map(|i| {
                        FutMutex::new(ShutdownVote {
                            id: i,
                            in_favor: true,
                            voter_status: VoterStatus::Registered(ActorIdentity::new(i, "v", None)),
                            ..Default::default()
                        })
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            w.vote_in_favor_total.store(voters, Ordering::SeqCst);
        }
        watch_shutdown(
            Duration::from_secs(10),
            Instant::now(),
            rs.clone(),
            Duration::from_millis(1),
        )
        .expect("clean shutdown");
        prop_assert_eq!(rs.read().state.clone(), GraphLivelinessState::Stopped);
    }
}
