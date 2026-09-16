//! OS-thread integration tests for dynamic troupe packing (low case count).

// ss[related troupe.dynamic-finalize-at-start]
use crate::*;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::thread::{self, ThreadId};
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};

/// Shared place to record which OS thread an actor ran on.
#[derive(Default)]
// ss[related troupe.dynamic-no-repack]
struct ThreadProbe {
    id: parking_lot::Mutex<Option<ThreadId>>,
    generations: AtomicUsize,
}

impl ThreadProbe {
    fn record(&self) {
        let tid = thread::current().id();
        let mut g = self.id.lock();
        if g.is_none() {
            *g = Some(tid);
        } else {
            assert_eq!(*g, Some(tid), "packed actor must stay on same OS thread");
        }
        self.generations.fetch_add(1, Ordering::SeqCst);
    }

    fn tid(&self) -> Option<ThreadId> {
        *self.id.lock()
    }
}

#[test]
// ss[verify troupe.dynamic-finalize-at-start]
// ss[verify troupe.dynamic-is-a-bag]
// ss[verify troupe.dynamic-for-testing]
// ss[verify troupe.dynamic-slot-budget]
// ss[verify troupe.dynamic-kruskal-capacity]
// ss[verify troupe.dynamic-hold-until-start]
// ss[verify troupe.dynamic-channel-establish]
// ss[verify philosophy.startup-schedule]
// ss[verify troupe.dynamic-member-of]
fn dynamic_troupe_kruskal_two_slots() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = GraphBuilder::for_testing()
        .with_pack_slots(2)
        .with_telemtry_production_rate_ms(0)
        .build(());

    let probe_a = Arc::new(ThreadProbe::default());
    let probe_b = Arc::new(ThreadProbe::default());
    let probe_c = Arc::new(ThreadProbe::default());

    let (ab_tx, ab_rx) = graph.channel_builder().with_capacity(2).build::<u64>();
    let (bc_tx, bc_rx) = graph.channel_builder().with_capacity(4096).build::<u64>();

    let mut bag = graph.dynamic_troupe().with_name("DynBag");

    let pa = probe_a.clone();
    graph
        .actor_builder()
        .with_name("A")
        .with_pack_ports(vec![], vec![ab_tx.pack_channel_id()])
        .never_simulate(true)
        .build(
            move |ctx| {
                let pa = pa.clone();
                let tx = ab_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([], [&tx]);
                    let mut tx = tx.lock().await;
                    pa.record();
                    let _ = actor.try_send(&mut tx, 1u64);
                    while actor.is_running(|| tx.mark_closed()) {
                        actor.wait_periodic(Duration::from_millis(5)).await;
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut bag),
        );

    let pb = probe_b.clone();
    graph
        .actor_builder()
        .with_name("B")
        .with_pack_ports_from(&[&ab_rx], &[&bc_tx])
        .never_simulate(true)
        .build(
            move |ctx| {
                let pb = pb.clone();
                let rx = ab_rx.clone();
                let tx = bc_tx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], [&tx]);
                    let mut rx = rx.lock().await;
                    let mut tx = tx.lock().await;
                    pb.record();
                    while actor.is_running(|| {
                        i!(rx.is_closed_and_empty()) && i!(tx.mark_closed())
                    }) {
                        let clean = await_for_all!(
                            actor.wait_avail(&mut rx, 1),
                            actor.wait_vacant(&mut tx, 1)
                        );
                        if clean {
                            if let Some(v) = actor.try_take(&mut rx) {
                                let _ = actor.try_send(&mut tx, v);
                            }
                        }
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut bag),
        );

    let pc = probe_c.clone();
    graph
        .actor_builder()
        .with_name("C")
        .with_pack_ports(vec![bc_rx.pack_channel_id()], vec![])
        .never_simulate(true)
        .build(
            move |ctx| {
                let pc = pc.clone();
                let rx = bc_rx.clone();
                async move {
                    let mut actor = ctx.into_spotlight([&rx], []);
                    let mut rx = rx.lock().await;
                    pc.record();
                    while actor.is_running(|| rx.is_closed_and_empty()) {
                        await_for_all!(actor.wait_avail(&mut rx, 1));
                        let _ = actor.try_take(&mut rx);
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut bag),
        );

    // Hold bag across start — finalize must still pack.
    assert!(graph.start_with_timeout(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(120));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;

    let ta = probe_a.tid().expect("A ran");
    let tb = probe_b.tid().expect("B ran");
    let tc = probe_c.tid().expect("C ran");
    assert_eq!(ta, tb, "A and B should share the small-cap edge thread");
    assert_ne!(tb, tc, "C should be on the other packed slot");
    Ok(())
}

#[test]
// ss[verify troupe.dynamic-for-testing]
// ss[verify troupe.dynamic-no-incidence-fallback]
fn dynamic_troupe_for_testing_defaults_one_slot() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = GraphBuilder::for_testing()
        .with_telemtry_production_rate_ms(0)
        .build(());
    assert_eq!(graph.pack_slots, Some(1));

    let probe_a = Arc::new(ThreadProbe::default());
    let probe_b = Arc::new(ThreadProbe::default());
    let mut bag = graph.dynamic_troupe();

    let pa = probe_a.clone();
    graph.actor_builder().with_name("P1").never_simulate(true).build(
        move |ctx| {
            let pa = pa.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                pa.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut bag),
    );
    let pb = probe_b.clone();
    graph.actor_builder().with_name("P2").never_simulate(true).build(
        move |ctx| {
            let pb = pb.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                pb.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut bag),
    );
    drop(bag);
    assert!(graph.start_with_timeout(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(80));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;
    assert_eq!(
        probe_a.tid(),
        probe_b.tid(),
        "default for_testing slots=1 must co-locate packed actors"
    );
    Ok(())
}

#[test]
// ss[verify troupe.dynamic-no-repack]
// ss[verify graph.panic-restart]
// ss[verify actor.regeneration-survives]
fn packed_actor_panic_stays_on_same_thread() -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = GraphBuilder::for_testing()
        .with_pack_slots(1)
        .with_telemtry_production_rate_ms(0)
        .build(());
    let probe = Arc::new(ThreadProbe::default());
    let mut bag = graph.dynamic_troupe();
    let p = probe.clone();
    graph
        .actor_builder()
        .with_name("PANIC_PACK")
        .never_simulate(true)
        .build(
            move |ctx| {
                let p = p.clone();
                async move {
                    let mut actor = ctx.into_spotlight([], []);
                    p.record();
                    if actor.regeneration() == 0 {
                        panic!("intentional packed panic");
                    }
                    while actor.is_running(|| true) {
                        actor.wait_periodic(Duration::from_millis(5)).await;
                    }
                    Ok(())
                }
            },
            ScheduleAs::MemberOf(&mut bag),
        );
    drop(bag);
    assert!(graph.start_with_timeout(Duration::from_secs(10)));
    // Wait for the restart *before* StopRequested. Troupe supervisors do not
    // restart panics that land after shutdown has already been requested
    // (`graph.panic-restart` / `actor.regeneration-survives` apply while running).
    let deadline = Instant::now() + Duration::from_secs(2);
    while probe.generations.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;
    assert!(
        probe.generations.load(Ordering::SeqCst) >= 2,
        "expected restart after panic"
    );
    Ok(())
}

#[test]
// ss[verify graph.pack.incidence-before-start]
fn lazy_pack_channel_id_without_establish() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(7).build::<u32>();
    let id_tx = tx.pack_channel_id();
    let id_rx = rx.pack_channel_id();
    assert_eq!(id_tx, id_rx);
    assert_eq!(tx.pack_capacity(), 7);
    assert_eq!(rx.pack_capacity(), 7);
    assert!(graph.channel_pack_meta.lock().contains_key(&id_tx));
}

#[test]
// ss[verify troupe.dynamic-channel-establish]
// ss[verify graph.pack.incidence-before-start]
fn lazy_share_preserves_id_without_establish() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(3).build::<u8>();
    let tx2 = tx.share();
    let rx2 = rx.share();
    assert_eq!(tx.pack_channel_id(), tx2.pack_channel_id());
    assert_eq!(rx.pack_channel_id(), rx2.pack_channel_id());
    assert_eq!(tx.pack_capacity(), tx2.pack_capacity());
    // Meta was recorded at lazy build; share must not create a second channel id.
    assert_eq!(graph.channel_pack_meta.lock().len(), 1);
}

#[test]
// ss[verify troupe.dynamic-is-a-bag]
// ss[verify troupe.dynamic-finalize-at-start]
// ss[verify troupe.dynamic-hold-until-start]
fn dynamic_troupe_drop_does_not_spawn_finalize_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let mut graph = GraphBuilder::for_testing()
        .with_pack_slots(1)
        .with_telemtry_production_rate_ms(0)
        .build(());

    let bag = graph.dynamic_troupe().with_name("BagNoSpawn");
    assert!(matches!(bag.kind(), crate::actor_builder::TroupeKind::Dynamic));
    assert_eq!(graph.pending_dynamic_troupes.lock().len(), 1);
    // Drop before adding actors: bag stays on Graph; Drop must not spawn.
    drop(bag);
    assert_eq!(graph.pending_dynamic_troupes.lock().len(), 1);

    let mut bag2 = graph.dynamic_troupe();
    let probe = Arc::new(ThreadProbe::default());
    let p = probe.clone();
    graph.actor_builder().with_name("ONLY").never_simulate(true).build(
        move |ctx| {
            let p = p.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                p.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut bag2),
    );
    drop(bag2);

    graph.finalize_dynamic_troupes();
    // Idempotent: second call finds empty pending list.
    graph.finalize_dynamic_troupes();

    assert!(graph.start_with_timeout(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(80));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;
    assert!(probe.tid().is_some());
    Ok(())
}

#[test]
// ss[verify troupe.dynamic-slot-budget]
// ss[verify troupe.dynamic-member-of]
fn solo_act_reserves_slot_from_pack_budget() -> Result<(), Box<dyn std::error::Error>> {
    // Detected pin=2; one SoloAct consumes a reserved thread → 1 packed slot →
    // both packable actors must share one OS thread.
    let mut graph = GraphBuilder::for_testing()
        .with_pack_slots(2)
        .with_telemtry_production_rate_ms(0)
        .build(());

    let solo_probe = Arc::new(ThreadProbe::default());
    let a_probe = Arc::new(ThreadProbe::default());
    let b_probe = Arc::new(ThreadProbe::default());

    let sp = solo_probe.clone();
    graph.actor_builder().with_name("SOLO").never_simulate(true).build(
        move |ctx| {
            let sp = sp.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                sp.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::SoloAct,
    );

    let mut bag = graph.dynamic_troupe();
    let pa = a_probe.clone();
    graph.actor_builder().with_name("PA").never_simulate(true).build(
        move |ctx| {
            let pa = pa.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                pa.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut bag),
    );
    let pb = b_probe.clone();
    graph.actor_builder().with_name("PB").never_simulate(true).build(
        move |ctx| {
            let pb = pb.clone();
            async move {
                let mut actor = ctx.into_spotlight([], []);
                pb.record();
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(5)).await;
                }
                Ok(())
            }
        },
        ScheduleAs::MemberOf(&mut bag),
    );
    drop(bag);

    assert!(graph.start_with_timeout(Duration::from_secs(10)));
    thread::sleep(Duration::from_millis(100));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(2))?;

    let solo = solo_probe.tid().expect("solo ran");
    let a = a_probe.tid().expect("PA ran");
    let b = b_probe.tid().expect("PB ran");
    assert_ne!(solo, a, "SoloAct must not share the packed sub-troupe thread");
    assert_eq!(a, b, "with one remaining pack slot, packable actors share a thread");
    Ok(())
}

#[test]
// ss[verify graph.pack.incidence-before-start]
fn pack_ports_spotlight_mismatch_warns() {
    use crate::logging_util::steady_logger::start_log_capture;
    use crate::{assert_in_logs, PackPorts};

    let _guard = start_log_capture();
    let graph = GraphBuilder::for_testing()
        .with_telemtry_production_rate_ms(0)
        .build(());
    let mut shadow = graph.new_testing_test_monitor("mismatch");
    shadow.expected_pack_ports = Some(PackPorts {
        rx_channel_ids: vec![42],
        tx_channel_ids: vec![99],
    });
    // Empty spotlight ports disagree with declared pack ports.
    let _spotlight = shadow.into_spotlight([], []);
    assert_in_logs!(["pack RX ports", "pack TX ports"]);
}

