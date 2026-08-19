// ss[related graph.for-testing]
use super::super::{
    watch_shutdown, ActorIdentity, GraphLiveliness, GraphLivelinessState, ShutdownVote,
    VoterStatus,
};
// ss[related philosophy.structural-hierarchy]
use crate::expression_steady_eye::Eye;
// ss[related philosophy.structural-hierarchy]
use futures::lock::Mutex as FutMutex;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::Ordering;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};

// ss[related philosophy.structural-hierarchy]
fn new_liveliness() -> Arc<parking_lot::RwLock<GraphLiveliness>> {
    let oss = Arc::new(FutMutex::new(Vec::new()));
    let actors_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let catalog = Arc::new(parking_lot::RwLock::new(Vec::new()));
    Arc::new(parking_lot::RwLock::new(GraphLiveliness::new(
        oss,
        actors_count,
        catalog,
    )))
}

#[test]
// ss[verify graph.shutdown.accept]
fn watch_shutdown_returns_ok_when_all_votes_in_favor() {
    let rs = new_liveliness();
    {
        let mut w = rs.write();
        w.state = GraphLivelinessState::StopRequested;
        w.votes = Arc::new(
            vec![FutMutex::new(ShutdownVote {
                id: 0,
                in_favor: true,
                voter_status: VoterStatus::Registered(ActorIdentity::new(0, "a", None)),
                ..Default::default()
            })]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(1, Ordering::SeqCst);
    }

    watch_shutdown(
        Duration::from_secs(60),
        Instant::now(),
        rs.clone(),
        Duration::from_millis(1),
    )
    .expect("clean shutdown");

    assert_eq!(rs.read().state, GraphLivelinessState::Stopped);
}

#[test]
// ss[verify graph.shutdown.veto]
fn watch_shutdown_returns_err_on_unclean_timeout() {
    let rs = new_liveliness();
    let ident = ActorIdentity::new(0, "veto", None);
    {
        let mut w = rs.write();
        w.state = GraphLivelinessState::StopRequested;
        w.votes = Arc::new(
            vec![FutMutex::new(ShutdownVote {
                id: 0,
                signature: Some(ident),
                in_favor: false,
                voter_status: VoterStatus::Registered(ident),
                veto_reason: Some(Eye {
                    expression: "rx.is_closed_and_empty()",
                    file: "shutdown_unit.rs",
                    line: 1,
                }),
                ..Default::default()
            })]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(0, Ordering::SeqCst);
    }

    let started = Instant::now() - Duration::from_secs(5);
    let err = watch_shutdown(
        Duration::from_millis(1),
        started,
        rs.clone(),
        Duration::from_millis(1),
    )
    .expect_err("unclean shutdown");

    assert!(err.to_string().contains("uncleanly"));
    assert_eq!(rs.read().state, GraphLivelinessState::StoppedUncleanly);
}

#[test]
// ss[verify graph.shutdown.veto]
fn watch_shutdown_unclean_reports_multiple_voters_with_backtrace() {
    // ss[related philosophy.structural-hierarchy]
    use std::backtrace::Backtrace;

    let rs = new_liveliness();
    let voter_ok = ActorIdentity::new(0, "clean", None);
    let voter_veto = ActorIdentity::new(1, "veto", None);
    let voter_internal = ActorIdentity::new(2, "metrics_server", None);
    let voter_collector = ActorIdentity::new(3, "metrics_collector", None);
    {
        let mut w = rs.write();
        w.state = GraphLivelinessState::StopRequested;
        w.votes = Arc::new(
            vec![
                FutMutex::new(ShutdownVote {
                    id: 0,
                    signature: Some(voter_ok),
                    in_favor: true,
                    voter_status: VoterStatus::Registered(voter_ok),
                    ..Default::default()
                }),
                FutMutex::new(ShutdownVote {
                    id: 1,
                    signature: Some(voter_veto),
                    in_favor: false,
                    voter_status: VoterStatus::Registered(voter_veto),
                    veto_reason: Some(Eye {
                        expression: "logger_tx.mark_closed()",
                        file: "shutdown_unit.rs",
                        line: 42,
                    }),
                    veto_backtrace: Some(Backtrace::capture()),
                    ..Default::default()
                }),
                FutMutex::new(ShutdownVote {
                    id: 2,
                    signature: Some(voter_internal),
                    in_favor: false,
                    voter_status: VoterStatus::Registered(voter_internal),
                    veto_reason: Some(Eye {
                        expression: "internal",
                        file: "shutdown_unit.rs",
                        line: 1,
                    }),
                    veto_backtrace: Some(Backtrace::capture()),
                    ..Default::default()
                }),
                FutMutex::new(ShutdownVote {
                    id: 3,
                    signature: Some(voter_collector),
                    in_favor: false,
                    voter_status: VoterStatus::Registered(voter_collector),
                    ..Default::default()
                }),
            ]
            .into_boxed_slice(),
        );
        w.vote_in_favor_total.store(1, Ordering::SeqCst);
    }

    let started = Instant::now() - Duration::from_secs(5);
    let err = watch_shutdown(
        Duration::from_millis(1),
        started,
        rs.clone(),
        Duration::from_millis(1),
    )
    .expect_err("unclean with veto backtrace");

    assert!(err.to_string().contains("uncleanly"));
    assert_eq!(rs.read().state, GraphLivelinessState::StoppedUncleanly);
}
