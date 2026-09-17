// ss[related graph.for-testing]
use super::super::{
    effective_block_until_stopped_timeout, elapsed_exceeds, ActorIdentity, GraphLiveliness,
    GraphLivelinessState, ShutdownVote, VoterStatus,
};
// ss[related graph.liveliness-voters]
use crate::core_exec;
// ss[related philosophy.structural-hierarchy]
use futures::lock::Mutex as FutMutex;
// ss[related graph.liveliness-voters]
use std::sync::atomic::{AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::Duration;

// ss[related graph.for-testing]
fn new_liveliness(
    actors: usize,
) -> (
    Arc<parking_lot::RwLock<GraphLiveliness>>,
    Arc<AtomicUsize>,
    Arc<parking_lot::RwLock<Vec<ActorIdentity>>>,
) {
    let oss = Arc::new(FutMutex::new(Vec::new()));
    let actors_count = Arc::new(AtomicUsize::new(actors));
    let catalog = Arc::new(parking_lot::RwLock::new(Vec::new()));
    let gl = GraphLiveliness::new(oss, actors_count.clone(), catalog.clone());
    (Arc::new(parking_lot::RwLock::new(gl)), actors_count, catalog)
}

// ss[verify graph.actor-identity]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn actor_by_id_finds_catalog_entries() {
    let (l, _, cat) = new_liveliness(0);
    {
        let a = ActorIdentity::new(3, "alpha", None);
        cat.write().push(a);
        assert_eq!(l.read().actor_by_id(3), Some(a));
        assert_eq!(l.read().actor_by_id(99), None);
    }
}

// ss[verify graph.liveliness-voters]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn remove_voter_marks_dead_when_registered() {
    let (l, _, _) = new_liveliness(0);
    let ident = ActorIdentity::new(0, "v", None);
    {
        let mut w = l.write();
        w.register_voter(ident);
        assert!(matches!(w.registered_voters[0], VoterStatus::Registered(_)));
        w.remove_voter(ident);
        assert!(matches!(w.registered_voters[0], VoterStatus::Dead(_)));
    }
}

// ss[verify graph.liveliness-voters]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn wait_for_registrations_waits_until_actor_count_matches() {
    let (l, count, _) = new_liveliness(1);
    let ident = ActorIdentity::new(0, "w", None);
    {
        let mut w = l.write();
        w.register_voter(ident);
        w.wait_for_registrations(Duration::from_secs(2));
        assert_eq!(w.state, GraphLivelinessState::Running);
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

// ss[verify graph.liveliness-voters]
#[test]
#[should_panic(expected = "timeout on startup, not all actors registered")]
// ss[verify verify.process.mutants]
fn wait_for_registrations_panics_in_tests_when_nobody_registers() {
    let (l, _, _) = new_liveliness(1);
    l.write().wait_for_registrations(Duration::from_millis(80));
}

// ss[verify graph.liveliness-voters]
#[test]
// ss[verify verify.process.mutants]
fn wait_for_registrations_empty_graph_goes_running() {
    let (l, _, _) = new_liveliness(0);
    l.write().wait_for_registrations(Duration::from_millis(80));
    assert_eq!(l.read().state, GraphLivelinessState::Running);
}

// ss[verify graph.liveliness-voters]
#[test]
// ss[verify verify.process.mutants]
fn wait_for_registrations_timeout_does_not_fire_at_t0() {
    let (l, _, _) = new_liveliness(1);
    let budget = Duration::from_millis(200); // multiple of the 40ms poll inside wait_for_registrations
    let start = std::time::Instant::now();
    let handle = std::thread::spawn(move || {
        l.write().wait_for_registrations(budget);
    });
    std::thread::sleep(Duration::from_millis(80));
    assert!(
        !handle.is_finished(),
        "elapsed > timeout must wait the budget; firing at t=0 is the < mutant"
    );
    let join = handle.join();
    assert!(join.is_err(), "unregistered actors must still trip the timeout panic");
    // `>` fires after the first poll past 200ms (~240ms). `>=` can fire at exactly 200ms.
    assert!(
        start.elapsed() > budget,
        "strict elapsed > timeout; >= would trip on the 200ms poll"
    );
}

// ss[verify graph.block-until-stopped]
#[test]
// ss[verify verify.process.mutants]
fn check_is_stopped_none_while_votes_outstanding_within_timeout() {
    let (l, _, _) = new_liveliness(0);
    {
        let mut w = l.write();
        w.register_voter(ActorIdentity::new(0, "a", None));
        w.register_voter(ActorIdentity::new(1, "b", None));
        w.building_to_running();
    }
    core_exec::block_on(GraphLiveliness::internal_request_shutdown(l.clone()));
    let got = l
        .read()
        .check_is_stopped(std::time::Instant::now(), Duration::from_secs(30));
    assert_eq!(got, None);
}

// ss[verify graph.block-until-stopped]
#[test]
// ss[verify verify.process.mutants]
fn check_is_stopped_unclean_when_timeout_expires_with_outstanding_votes() {
    let (l, _, _) = new_liveliness(0);
    {
        let mut w = l.write();
        w.register_voter(ActorIdentity::new(0, "a", None));
        w.register_voter(ActorIdentity::new(1, "b", None));
        w.building_to_running();
    }
    core_exec::block_on(GraphLiveliness::internal_request_shutdown(l.clone()));
    let started = std::time::Instant::now() - Duration::from_secs(5);
    let got = l.read().check_is_stopped(started, Duration::from_millis(1));
    assert_eq!(got, Some(GraphLivelinessState::StoppedUncleanly));
}

// ss[verify graph.block-until-stopped]
#[test]
// ss[verify verify.process.mutants]
fn check_is_stopped_clean_when_no_voters() {
    let (l, _, _) = new_liveliness(0);
    l.write().state = GraphLivelinessState::StopRequested;
    let got = l
        .read()
        .check_is_stopped(std::time::Instant::now(), Duration::from_secs(1));
    assert_eq!(got, Some(GraphLivelinessState::Stopped));
}

// ss[verify actor.is-running-loop]
#[test]
// ss[verify verify.process.mutants]
fn is_running_matches_graph_state() {
    let (l, _, _) = new_liveliness(0);
    let ident = ActorIdentity::new(0, "r", None);
    {
        let mut w = l.write();
        w.state = GraphLivelinessState::Building;
        assert_eq!(w.is_running(ident, || true), None);
        w.state = GraphLivelinessState::Running;
        assert_eq!(w.is_running(ident, || true), Some(true));
        w.state = GraphLivelinessState::Stopped;
        assert_eq!(w.is_running(ident, || true), Some(false));
        w.state = GraphLivelinessState::StoppedUncleanly;
        assert_eq!(w.is_running(ident, || true), Some(false));
    }
}

// ss[verify graph.liveliness-voters]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn vote_for_the_dead_casts_dead_actor_ballots() {
    let (rs, _, _) = new_liveliness(0);
    let ident = ActorIdentity::new(0, "dead", None);
    {
        let mut w = rs.write();
        w.registered_voters = vec![VoterStatus::Dead(ident)];
        w.votes = Arc::new(
            vec![FutMutex::new(ShutdownVote {
                id: 0,
                signature: None,
                in_favor: false,
                voter_status: VoterStatus::Dead(ident),
                veto_backtrace: None,
                veto_reason: None,
            })]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(0, Ordering::SeqCst);
    }
    GraphLiveliness::vote_for_the_dead(rs.clone());
    assert_eq!(
        rs.read().vote_in_favor_total.load(Ordering::SeqCst),
        1
    );
    let rl = rs.read();
    let v = rl.votes[0].try_lock().expect("vote lock");
    assert!(v.in_favor);
}

// ss[verify telemetry.shutdown-complete]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn is_shutdown_telemetry_complete_counts_non_telemetry_voters() {
    let (l, _, _) = new_liveliness(0);
    {
        let mut w = l.write();
        w.votes = Arc::new(
            vec![
                FutMutex::new(ShutdownVote::default()),
                FutMutex::new(ShutdownVote::default()),
                FutMutex::new(ShutdownVote::default()),
                FutMutex::new(ShutdownVote::default()),
                FutMutex::new(ShutdownVote::default()),
            ]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(3, Ordering::Relaxed);
    }
        assert!(l.read().is_shutdown_telemetry_complete(2));
    l.write()
        .vote_in_favor_total
        .store(4, Ordering::Relaxed);
    assert!(l.read().is_shutdown_telemetry_complete(2));
    l.write()
        .vote_in_favor_total
        .store(2, Ordering::Relaxed);
    assert!(!l.read().is_shutdown_telemetry_complete(2));
}

// ss[verify graph.shutdown.accept]
#[test]
// ss[verify verify.process.mutants]
fn apply_shutdown_vote_matches_graph_state() {
    let (l, _, _) = new_liveliness(0);
    let ident = ActorIdentity::new(0, "v", None);
    {
        let mut w = l.write();
        w.votes = Arc::new(
            vec![FutMutex::new(ShutdownVote {
                id: 0,
                ..Default::default()
            })]
            .into_boxed_slice(),
        );
        w.state = GraphLivelinessState::Building;
        assert_eq!(w.apply_shutdown_vote(ident, true), None);
        w.state = GraphLivelinessState::Running;
        assert_eq!(w.apply_shutdown_vote(ident, true), Some(true));
        w.state = GraphLivelinessState::Stopped;
        assert_eq!(w.apply_shutdown_vote(ident, true), Some(false));
        w.state = GraphLivelinessState::StopRequested;
        w.vote_in_favor_total.store(0, Ordering::SeqCst);
        assert_eq!(w.apply_shutdown_vote(ident, true), Some(false));
        assert_eq!(w.vote_in_favor_total.load(Ordering::SeqCst), 1);
        assert_eq!(w.apply_shutdown_vote(ident, false), Some(true));
        assert_eq!(w.vote_in_favor_total.load(Ordering::SeqCst), 1);
    }
}

// ss[verify graph.shutdown.accept]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn is_running_accept_shutdown_transitions_vote() {
    let (l, _, _) = new_liveliness(0);
    let ident = ActorIdentity::new(0, "r", None);
    {
        let mut w = l.write();
        w.state = GraphLivelinessState::StopRequested;
        w.votes = Arc::new(
            vec![FutMutex::new(ShutdownVote {
                id: 0,
                ..Default::default()
            })]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(0, Ordering::SeqCst);
    }
    let r = l.read();
    assert_eq!(r.is_running(ident, || true), Some(false));
    drop(r);
    assert_eq!(l.read().vote_in_favor_total.load(Ordering::SeqCst), 1);
    let r2 = l.read();
    assert_eq!(r2.is_running(ident, || false), Some(true));
    drop(r2);
    assert_eq!(l.read().vote_in_favor_total.load(Ordering::SeqCst), 1);
}

// ss[verify graph.request-shutdown]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn internal_request_shutdown_from_running_sets_stop_requested() {
    let (rs, _, _) = new_liveliness(0);
    {
        let mut w = rs.write();
        w.register_voter(ActorIdentity::new(0, "s", None));
        w.building_to_running();
    }
    core_exec::block_on(GraphLiveliness::internal_request_shutdown(rs.clone()));
    let r = rs.read();
    assert!(r.is_in_state(&[GraphLivelinessState::StopRequested]));
    assert_eq!(r.votes.len(), 1);
}

// ss[verify graph.actor-identity]

// ss[verify graph.liveliness-voters]
#[test]
// ss[related philosophy.structural-hierarchy]
fn actor_identity_debug_includes_name_and_suffix() {
    let id = ActorIdentity::new(7, "ProbeActor", Some(2));
    let s = format!("{:?}", id);
    assert!(s.contains("ProbeActor"));
    assert!(s.contains("-2"));
}

// ss[verify graph.for-testing]

#[test]
// ss[verify graph.for-testing]
fn effective_block_until_stopped_timeout_uses_telemetry_floor() {
    assert_eq!(
        effective_block_until_stopped_timeout(Duration::from_millis(10), 100),
        Duration::from_millis(300)
    );
    assert_eq!(
        effective_block_until_stopped_timeout(Duration::from_millis(500), 100),
        Duration::from_millis(500)
    );
}

// ss[verify philosophy.explicit-ownership]

#[test]
// ss[verify graph.for-testing]
fn test_graph_liveliness_state_equality() {
    let building = GraphLivelinessState::Building;
    let running = GraphLivelinessState::Running;
    let stop_requested = GraphLivelinessState::StopRequested;
    let stopped = GraphLivelinessState::Stopped;
    let stopped_uncleanly = GraphLivelinessState::StoppedUncleanly;

    assert_eq!(building, GraphLivelinessState::Building);
    assert_ne!(building, running);
    assert_eq!(running, GraphLivelinessState::Running);
    assert_ne!(running, stop_requested);
    assert_eq!(stop_requested, GraphLivelinessState::StopRequested);
    assert_ne!(stop_requested, stopped);
    assert_eq!(stopped, GraphLivelinessState::Stopped);
    assert_ne!(stopped, stopped_uncleanly);
    assert_eq!(stopped_uncleanly, GraphLivelinessState::StoppedUncleanly);
    assert_ne!(stopped_uncleanly, building);
}


#[test]
// ss[verify graph.for-testing]
fn test_graph_liveliness_state_cloning() {
    let building = GraphLivelinessState::Building;
    let building_clone = building.clone();
    assert_eq!(building, building_clone);
}


#[test]
// ss[verify graph.for-testing]
fn test_graph_liveliness_state_debug_output() {
    let building = GraphLivelinessState::Building;
    let debug_str = format!("{:?}", building);
    assert_eq!(debug_str, "Building");
}

#[test]
// ss[verify graph.for-testing]
// ss[verify verify.process.mutants]
fn elapsed_exceeds_is_strict_at_the_budget() {
    let budget = Duration::from_millis(200);
    assert!(!elapsed_exceeds(budget, budget));
    assert!(elapsed_exceeds(budget + Duration::from_millis(1), budget));
    assert!(!elapsed_exceeds(Duration::ZERO, budget));
}
