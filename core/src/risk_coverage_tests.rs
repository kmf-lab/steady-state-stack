//! Focused checks for the highest-risk Steady State contracts:
//! peek/take ordering, backpressure, `SteadyState` across panic,
//! shutdown veto while a channel still holds work, and telemetry emit.

// ss[related philosophy.zero-copy-discipline]
use std::collections::VecDeque;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::Instant;

// ss[related philosophy.structural-hierarchy]
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData, RxTel};
// ss[related telemetry.prometheus-metrics]
use crate::telemetry::metrics_collector::{
    CollectorDetail, DiagramData, MetricsCollector, NAME as COLLECTOR_NAME,
};
// ss[related philosophy.structural-hierarchy]
use crate::*;

struct CommitLog {
    committed: Vec<u64>,
    last_generation: u32,
}

/// Peek does not consume. After a take, the next head is a different message,
/// and `is_showstopper(n)` trips after the same number of peeks on every message.
#[test]
// ss[verify philosophy.zero-copy-discipline]
// ss[verify channel.backpressure-never-drop]
fn peek_does_not_consume_and_showstopper_is_stable_across_messages() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    tx.testing_send_all(vec![1, 2, 3], false);

    let steady = rx.clone();
    let mut guard = core_exec::block_on(steady.lock());
    let mut buf = [0u8; 2];
    assert_eq!(guard.try_peek_slice(&mut buf), 2);
    assert_eq!(buf, [1, 2]);
    assert_eq!(
        guard.try_peek(),
        Some(&1),
        "slice peek must not advance the head"
    );

    const PEEKS: usize = 2;
    for _ in 0..PEEKS {
        assert_eq!(guard.try_peek(), Some(&1));
    }
    assert!(guard.is_showstopper(PEEKS));
    assert_eq!(guard.try_take(), Some(1));

    for _ in 0..PEEKS {
        assert_eq!(guard.try_peek(), Some(&2));
    }
    assert!(
        guard.is_showstopper(PEEKS),
        "message 2 after {PEEKS} peeks, repeats={}",
        guard.peek_repeats.load(Ordering::Relaxed)
    );
    drop(guard);

    assert_eq!(rx.testing_take_all(), vec![2, 3]);
}

/// Graph-owned state and the peeked head both survive an actor panic.
/// The panicked generation must not commit, and the restart commits once.
#[test]
// ss[verify state.steady-state-persistence]
// ss[verify state.lock-init-once]
// ss[verify actor.regeneration-survives]
// ss[verify graph.panic-restart]
// ss[verify philosophy.zero-copy-discipline]
fn steady_state_survives_panic_without_losing_peeked_message() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(8).build::<u64>();
    let state = new_state::<CommitLog>();
    let progress = Arc::new(AtomicUsize::new(0));

    let state_actor = state.clone();
    let progress_actor = progress.clone();
    graph
        .actor_builder()
        .with_name("PEEK_COMMIT")
        .never_simulate(true)
        .build(
            move |ctx| {
                let state_actor = state_actor.clone();
                let rx = rx.clone();
                let progress_actor = progress_actor.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], []);
                    let mut state = state_actor
                        .lock(|| CommitLog {
                            committed: Vec::new(),
                            last_generation: 0,
                        })
                        .await;
                    state.last_generation = actor.regeneration();
                    let mut rx = rx.lock().await;
                    while actor.is_running(|| rx.is_closed_and_empty()) {
                        let _clean = await_for_all!(actor.wait_avail(&mut rx, 1));
                        if let Some(&value) = actor.try_peek(&mut rx) {
                            if value == 7 && actor.regeneration() == 0 {
                                panic!("intentional peek-before-commit panic");
                            }
                            let taken = actor
                                .try_take(&mut rx)
                                .expect("peeked message still queued");
                            state.committed.push(taken);
                            progress_actor.store(state.committed.len(), Ordering::SeqCst);
                        }
                    }
                    Ok(())
                }
            },
            SoloAct,
        );

    tx.testing_send_all(vec![7, 8], true);
    assert!(graph.start_with_timeout(Duration::from_secs(5)));

    let deadline = Instant::now() + Duration::from_secs(3);
    while progress.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        progress.load(Ordering::SeqCst),
        2,
        "peeked message was lost or duplicated across restart"
    );

    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(3))?;
    let final_state = state.try_lock_sync().expect("state lock after shutdown");
    assert_eq!(final_state.committed, vec![7, 8]);
    assert!(final_state.last_generation >= 1);
    Ok(())
}

/// Shutdown is vetoed while a queued message is still peeked, then accepted
/// once that message is taken. The queued value is not dropped to force the stop.
#[test]
// ss[verify graph.shutdown.veto]
// ss[verify graph.shutdown.accept]
// ss[verify actor.shutdown-veto]
// ss[verify philosophy.cooperative-liveliness]
// ss[verify channel.backpressure-never-drop]
fn shutdown_veto_holds_until_queued_message_is_taken() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u64>();
    let seen = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let taken = Arc::new(AtomicU64::new(0));

    let seen_actor = seen.clone();
    let release_actor = release.clone();
    let taken_actor = taken.clone();
    graph
        .actor_builder()
        .with_name("VETO_UNTIL_TAKE")
        .never_simulate(true)
        .build(
            move |ctx| {
                let rx = rx.clone();
                let seen_actor = seen_actor.clone();
                let release_actor = release_actor.clone();
                let taken_actor = taken_actor.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], []);
                    let mut rx = rx.lock().await;
                    while actor.is_running(|| rx.is_closed_and_empty()) {
                        let _clean = await_for_all!(
                            actor.wait_periodic(Duration::from_millis(5)),
                            actor.wait_avail(&mut rx, 1)
                        );
                        if let Some(&_value) = actor.try_peek(&mut rx) {
                            seen_actor.store(true, Ordering::SeqCst);
                            if release_actor.load(Ordering::SeqCst) {
                                if let Some(got) = actor.try_take(&mut rx) {
                                    taken_actor.store(got, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                    Ok(())
                }
            },
            SoloAct,
        );

    tx.testing_send_all(vec![42], true);
    assert!(graph.start_with_timeout(Duration::from_secs(5)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !seen.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        seen.load(Ordering::SeqCst),
        "actor never peeked the queued message"
    );

    let release_later = release.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        release_later.store(true, Ordering::SeqCst);
    });
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;
    assert_eq!(taken.load(Ordering::SeqCst), 42);
    Ok(())
}

struct VolumeTel {
    meta: Arc<ActorMetaData>,
    rx_meta: Arc<ChannelMetaData>,
    tx_meta: Arc<ChannelMetaData>,
    status_once: AtomicBool,
    subtitle_once: AtomicBool,
}

impl RxTel for VolumeTel {
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        vec![self.tx_meta.clone()]
    }

    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>> {
        vec![self.rx_meta.clone()]
    }

    fn consume_actor(&self) -> Option<ActorStatus> {
        if self
            .status_once
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(ActorStatus {
                ident: self.meta.ident,
                ..ActorStatus::default()
            })
        } else {
            None
        }
    }

    fn consume_dot_subtitle(&self) -> Option<Option<String>> {
        if self
            .subtitle_once
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Some("lane".to_string()))
        } else {
            None
        }
    }

    fn actor_metadata(&self) -> Arc<ActorMetaData> {
        self.meta.clone()
    }

    fn consume_take_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        _future_take: &mut Vec<i64>,
        _future_send: &mut Vec<i64>,
    ) -> bool {
        let id = self.rx_meta.id;
        if id < take_send_source.len() {
            take_send_source[id].0 = 4;
            true
        } else {
            false
        }
    }

    fn consume_send_into(
        &self,
        take_send_source: &mut Vec<(i64, i64)>,
        _future_send: &mut Vec<i64>,
    ) -> bool {
        let id = self.tx_meta.id;
        if id < take_send_source.len() {
            take_send_source[id].1 = 9;
            true
        } else {
            false
        }
    }

    fn actor_rx(&self, _version: u32) -> Option<Box<SteadyRx<ActorStatus>>> {
        None
    }

    fn is_empty_and_closed(&self) -> bool {
        true
    }

    fn is_empty(&self) -> bool {
        false
    }
}

/// Collector emits node, status, subtitle, and channel volume, and skips its own name.
#[test]
// ss[verify telemetry.prometheus-metrics]
fn collector_emits_volume_status_and_skips_internal_name() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph
        .channel_builder()
        .with_capacity(32)
        .build::<DiagramData>();
    let targets = Arc::new([tx.clone()]);
    let registry = graph.all_telemetry_rx.clone();

    let worker_ident = ActorIdentity::new(7, "worker_cov", None);
    let worker_meta = Arc::new(ActorMetaData {
        ident: worker_ident,
        ..ActorMetaData::default()
    });
    graph.all_telemetry_rx.write().push(CollectorDetail {
        ident: worker_ident,
        telemetry_take: VecDeque::from([Box::new(VolumeTel {
            meta: worker_meta,
            rx_meta: Arc::new(ChannelMetaData {
                id: 5,
                capacity: 4,
                ..ChannelMetaData::default()
            }),
            tx_meta: Arc::new(ChannelMetaData {
                id: 6,
                capacity: 4,
                ..ChannelMetaData::default()
            }),
            status_once: AtomicBool::new(false),
            subtitle_once: AtomicBool::new(false),
        }) as Box<dyn RxTel>]),
    });
    graph.all_telemetry_rx.write().push(CollectorDetail {
        ident: ActorIdentity::new(1, COLLECTOR_NAME, None),
        telemetry_take: VecDeque::from([Box::new(VolumeTel {
            meta: Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "SHOULD_NOT_EMIT", None),
                ..ActorMetaData::default()
            }),
            rx_meta: Arc::new(ChannelMetaData {
                id: 1,
                ..ChannelMetaData::default()
            }),
            tx_meta: Arc::new(ChannelMetaData {
                id: 2,
                ..ChannelMetaData::default()
            }),
            status_once: AtomicBool::new(false),
            subtitle_once: AtomicBool::new(false),
        }) as Box<dyn RxTel>]),
    });

    graph
        .actor_builder()
        .with_name("BUDDY")
        .never_simulate(true)
        .build(
            |ctx| async move {
                let mut actor = ctx.into_spotlight([], []);
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(15)).await;
                }
                Ok(())
            },
            SoloAct,
        );

    graph
        .actor_builder()
        .with_name("COLLECTOR_UNDER_TEST")
        .never_simulate(true)
        .build(
            move |ctx| {
                let registry = registry.clone();
                let targets = targets.clone();
                let collector = MetricsCollector::new(registry, targets, 15);
                async move { collector.run(ctx).await }
            },
            SoloAct,
        );

    assert!(graph.start_with_timeout(Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(80));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(3))?;

    let msgs = rx.testing_take_all();
    assert!(!msgs.is_empty(), "collector emitted no diagram data");
    let rendered = format!("{msgs:?}");
    assert!(
        rendered.contains("worker_cov"),
        "missing worker node def: {rendered}"
    );
    assert!(
        !rendered.contains("SHOULD_NOT_EMIT"),
        "internal collector name was not skipped: {rendered}"
    );
    assert!(
        rendered.contains("lane"),
        "missing subtitle update: {rendered}"
    );

    let mut saw_volume = false;
    for msg in &msgs {
        if let DiagramData::ChannelVolumeData(_, rows) = msg {
            let has_take = rows.iter().any(|(id, take, _send)| *id == 5 && *take == 4);
            let has_send = rows.iter().any(|(id, _take, send)| *id == 6 && *send == 9);
            if has_take && has_send {
                saw_volume = true;
            }
        }
    }
    assert!(saw_volume, "missing channel volume rows: {rendered}");
    Ok(())
}
