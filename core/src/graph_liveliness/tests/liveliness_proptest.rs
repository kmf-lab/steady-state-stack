// ss[related graph.for-testing]
use crate::ss_proptest;
use proptest::prelude::*;
use super::super::{
    effective_block_until_stopped_timeout, ActorIdentity, GraphLiveliness, GraphLivelinessState,
    VoterStatus,
};
use crate::core_exec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn new_liveliness(
    actors: usize,
) -> (
    Arc<parking_lot::RwLock<GraphLiveliness>>,
    Arc<AtomicUsize>,
    Arc<parking_lot::RwLock<Vec<ActorIdentity>>>,
) {
    let oss = Arc::new(futures::lock::Mutex::new(Vec::new()));
    let actors_count = Arc::new(AtomicUsize::new(actors));
    let catalog = Arc::new(parking_lot::RwLock::new(Vec::new()));
    let gl = GraphLiveliness::new(oss, actors_count.clone(), catalog.clone());
    (Arc::new(parking_lot::RwLock::new(gl)), actors_count, catalog)
}

fn setup_stop_requested(
    l: &Arc<parking_lot::RwLock<GraphLiveliness>>,
    voters: usize,
    yes_votes: usize,
) -> Vec<ActorIdentity> {
    let mut idents = Vec::with_capacity(voters);
    {
        let mut w = l.write();
        for i in 0..voters {
            let ident = ActorIdentity::new(i, "v", None);
            w.register_voter(ident);
            idents.push(ident);
        }
        w.building_to_running();
    }
    core_exec::block_on(GraphLiveliness::internal_request_shutdown(l.clone()));
    for (i, ident) in idents.iter().enumerate() {
        if i < yes_votes {
            let _ = l.read().is_running(*ident, || true);
        }
    }
    idents
}

fn ballots_in_favor(l: &GraphLiveliness) -> usize {
    l.votes
        .iter()
        .filter(|v| v.try_lock().map(|g| g.in_favor).unwrap_or(false))
        .count()
}

proptest! {
    #![proptest_config(crate::proptest_support::default_config())]

    /// Property: effective block timeout is at least clean_shutdown_timeout and telemetry floor.
    #[test]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_effective_timeout_monotonic(
        clean_ms in 0u64..60_000,
        telemetry_ms in 1u64..10_000,
    ) {
        let clean = Duration::from_millis(clean_ms);
        let got = effective_block_until_stopped_timeout(clean, telemetry_ms);
        let floor = Duration::from_millis(3 * telemetry_ms);
        prop_assert!(got >= clean);
        prop_assert!(got >= floor);
    }

    /// Property: GraphLivelinessState equality is reflexive.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_liveliness_state_reflexive(state in prop::sample::select(vec![
        GraphLivelinessState::Building,
        GraphLivelinessState::Running,
        GraphLivelinessState::Stopped,
        GraphLivelinessState::StopRequested,
        GraphLivelinessState::StoppedUncleanly,
    ])) {
        prop_assert_eq!(&state, &state);
    }

    /// Property: `is_in_state` matches membership for any selected state.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_is_in_state_membership(
        state in prop::sample::select(vec![
            GraphLivelinessState::Building,
            GraphLivelinessState::Running,
            GraphLivelinessState::StopRequested,
            GraphLivelinessState::Stopped,
            GraphLivelinessState::StoppedUncleanly,
        ]),
    ) {
        let (l, _, _) = new_liveliness(0);
        l.write().state = state.clone();
        let r = l.read();
        prop_assert!(r.is_in_state(&[state.clone()]));
        let other = match state {
            GraphLivelinessState::Building => GraphLivelinessState::Running,
            GraphLivelinessState::Running => GraphLivelinessState::Building,
            GraphLivelinessState::StopRequested => GraphLivelinessState::Running,
            GraphLivelinessState::Stopped => GraphLivelinessState::StopRequested,
            GraphLivelinessState::StoppedUncleanly => GraphLivelinessState::Stopped,
        };
        prop_assert!(!r.is_in_state(&[other]));
    }

    /// Property: `vote_in_favor_total` always equals the number of ballots marked in favor.
    #[test]
    // ss[verify graph.liveliness-voters]
    // ss[verify verify.process.proptest]
    fn proptest_vote_in_favor_total_conserved(
        voter_count in 1usize..8,
        accepts in prop::collection::vec(any::<bool>(), 1..24),
    ) {
        let (l, _, _) = new_liveliness(0);
        let idents = setup_stop_requested(&l, voter_count, 0);
        let mut prev_total = 0usize;
        for (step, &accept) in accepts.iter().enumerate() {
            let ident = idents[step % voter_count];
            let _ = l.read().is_running(ident, || accept);
            let r = l.read();
            let total = r.vote_in_favor_total.load(Ordering::SeqCst);
            let counted = ballots_in_favor(&r);
            prop_assert_eq!(total, counted);
            prop_assert!(total >= prev_total);
            prop_assert!(total <= voter_count);
            prev_total = total;
        }
    }

    /// Property: accepting shutdown twice for the same actor does not double-count.
    #[test]
    // ss[verify graph.shutdown.accept]
    // ss[verify verify.process.proptest]
    fn proptest_vote_accept_idempotent(voter_count in 1usize..6) {
        let (l, _, _) = new_liveliness(0);
        let idents = setup_stop_requested(&l, voter_count, 0);
        for ident in &idents {
            let _ = l.read().is_running(*ident, || true);
            let _ = l.read().is_running(*ident, || true);
        }
        let r = l.read();
        prop_assert_eq!(r.vote_in_favor_total.load(Ordering::SeqCst), voter_count);
        prop_assert_eq!(ballots_in_favor(&r), voter_count);
    }

    /// Property: dead voters are auto-cast yes on shutdown request.
    #[test]
    // ss[verify graph.liveliness-voters]
    // ss[verify verify.process.proptest]
    fn proptest_dead_voters_auto_yes(
        live_count in 0usize..4,
        dead_count in 1usize..4,
    ) {
        let (l, _, _) = new_liveliness(0);
        let mut idents = Vec::new();
        {
            let mut w = l.write();
            let total = live_count + dead_count;
            for i in 0..total {
                let ident = ActorIdentity::new(i, "v", None);
                w.register_voter(ident);
                idents.push(ident);
            }
            for ident in idents.iter().skip(live_count) {
                w.remove_voter(*ident);
            }
            w.building_to_running();
        }
        core_exec::block_on(GraphLiveliness::internal_request_shutdown(l.clone()));
        let r = l.read();
        prop_assert_eq!(r.vote_in_favor_total.load(Ordering::SeqCst), dead_count);
        for i in live_count..(live_count + dead_count) {
            let vote = r.votes[i].try_lock().expect("vote lock");
            prop_assert!(vote.in_favor);
        }
    }

    /// Property: `check_is_stopped` is monotonic — once `Stopped`, later checks stay `Stopped`.
    #[test]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_check_is_stopped_monotonic(
        voters in 1usize..6,
        yes_steps in 1usize..6,
    ) {
        let yes_steps = yes_steps.min(voters);
        let (l, _, _) = new_liveliness(0);
        let idents = setup_stop_requested(&l, voters, 0);
        let timeout = Duration::from_secs(60);
        let now = Instant::now();

        let mut last: Option<GraphLivelinessState> = None;
        for step in 0..=yes_steps {
            if step > 0 {
                let _ = l.read().is_running(idents[step - 1], || true);
            }
            let got = l.read().check_is_stopped(now, timeout);
            let is_stopped = got == Some(GraphLivelinessState::Stopped);
            if let Some(ref prev) = last {
                if *prev == GraphLivelinessState::Stopped {
                    prop_assert!(is_stopped);
                }
            }
            if is_stopped {
                last = Some(GraphLivelinessState::Stopped);
            }
        }
        if yes_steps == voters {
            prop_assert_eq!(
                l.read().check_is_stopped(now, timeout),
                Some(GraphLivelinessState::Stopped)
            );
        }
    }

    /// Property: all votes in ⇒ clean stopped; running graph ⇒ still in progress.
    #[test]
    // ss[verify graph.block-until-stopped]
    // ss[verify verify.process.proptest]
    fn proptest_check_is_stopped_outcomes(
        voters in 1usize..6,
        all_voted in any::<bool>(),
    ) {
        let (l, _, _) = new_liveliness(0);
        if all_voted {
            setup_stop_requested(&l, voters, voters);
            let got = l.read().check_is_stopped(Instant::now(), Duration::from_secs(60));
            prop_assert_eq!(got, Some(GraphLivelinessState::Stopped));
        } else {
            l.write().state = GraphLivelinessState::Running;
            let got = l.read().check_is_stopped(Instant::now(), Duration::from_secs(1));
            prop_assert_eq!(got, None);
        }
    }
}
