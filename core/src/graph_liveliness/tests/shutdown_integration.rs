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

