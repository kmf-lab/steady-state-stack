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
fn test_unclean_shutdown_veto() {
    let mut graph = GraphBuilder::for_testing().build(());
    
    graph.actor_builder()
        .with_name("VetoActor")
        .build(|mut actor| {
            Box::pin(async move {
                // Veto shutdown by returning false in the accept_fn
                while actor.is_running(|| false) {
                    actor.wait(Duration::from_millis(10)).await;
                }
                Ok(())
            })
        }, ScheduleAs::SoloAct);

    graph.start();
    graph.request_shutdown();
    
    // This should return an error because the actor vetoed
    let result = graph.block_until_stopped(Duration::from_millis(100));
    assert!(result.is_err());
}

// ss[verify graph.shutdown.accept]
// ss[verify philosophy.cooperative-liveliness]

#[test]
fn test_clean_shutdown_actor_accepts_stop() {
    let mut graph = GraphBuilder::for_testing().build(());

    graph
        .actor_builder()
        .with_name("CleanActor")
        .build(
            |mut actor| {
                Box::pin(async move {
                    while actor.is_running(|| true) {
                        actor.wait(Duration::from_millis(2)).await;
                    }
                    Ok(())
                })
            },
            ScheduleAs::SoloAct,
        );

    graph.start();
    graph.request_shutdown();
    let result = graph.block_until_stopped(Duration::from_secs(2));
    assert!(result.is_ok());
}

/// Regression: `block_until_stopped` must wait indefinitely for shutdown to be
/// requested; `clean_shutdown_timeout` bounds only the voting phase AFTER
/// `StopRequested`. A delayed shutdown request must not trip the timeout.
// ss[verify graph.block-until-stopped]
#[test]
fn test_block_until_stopped_waits_for_delayed_shutdown_request() {
    let mut graph = GraphBuilder::for_testing().build(());

    graph
        .actor_builder()
        .with_name("DelayedShutdown")
        .build(
            |mut actor| {
                Box::pin(async move {
                    // Request shutdown well after the clean-shutdown timeout has passed.
                    actor.wait(Duration::from_millis(1500)).await;
                    actor.request_shutdown().await;
                    while actor.is_running(|| true) {
                        actor.wait(Duration::from_millis(10)).await;
                    }
                    Ok(())
                })
            },
            ScheduleAs::SoloAct,
        );

    graph.start();
    let started = Instant::now();
    let result = graph.block_until_stopped(Duration::from_millis(100));
    assert!(result.is_ok());
    assert!(started.elapsed() >= Duration::from_millis(1500));
}

