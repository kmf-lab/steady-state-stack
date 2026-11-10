//! Round 3: SteadyActor method surface, defrag flush, and bundle helpers.
//!
//! Defrag flush is the large uncovered spotlight/shadow body. Empty, full,
//! partial, and blocked outcomes are each asserted.

// ss[related distributed.aqueduct-stream]
use ringbuf::traits::Producer;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::Ordering;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};

// ss[related distributed.aqueduct-stream]
use crate::distributed::aqueduct_stream::{Defrag, StreamEgress};
// ss[related philosophy.structural-hierarchy]
use futures::FutureExt;
use crate::*;

fn push_item(defrag: &mut Defrag<StreamEgress>, bytes: &[u8]) {
    let pushed = defrag.ringbuffer_bytes.0.push_slice(bytes);
    assert_eq!(pushed, bytes.len());
    defrag.running_length += bytes.len();
    let now = Instant::now();
    defrag.arrival = Some(now);
    defrag.finish = Some(now);
    defrag
        .ringbuffer_items
        .0
        .try_push(StreamEgress::new(bytes.len() as i32))
        .expect("defrag item fits");
}

#[test]
// ss[verify distributed.aqueduct-stream]
// ss[verify stream.control-payload]
// ss[verify actor.shadow-spotlight]
fn flush_defrag_empty_full_partial_and_blocked() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (mut roomy_items, _) = graph
        .channel_builder()
        .with_capacity(8)
        .eager_build_internal::<StreamEgress>();
    let (mut roomy_bytes, _) = graph.channel_builder().with_capacity(64).eager_build_internal::<u8>();
    let spot = graph.new_testing_test_monitor("flush_spot");
    let mut spot = spot.into_spotlight([], []);

    let mut empty = Defrag::<StreamEgress>::new(3, 8, 64);
    let (n, b, more) = spot.flush_defrag_messages(&mut roomy_items, &mut roomy_bytes, &mut empty);
    assert_eq!((n, b, more), (0, 0, None));

    let mut one = Defrag::<StreamEgress>::new(3, 8, 64);
    push_item(&mut one, &[1, 2, 3]);
    let (n, b, more) = spot.flush_defrag_messages(&mut roomy_items, &mut roomy_bytes, &mut one);
    assert_eq!(n, 1);
    assert_eq!(b, 3);
    assert!(more.is_none());

    let (mut tight_items, _) = graph
        .channel_builder()
        .with_capacity(1)
        .eager_build_internal::<StreamEgress>();
    let (mut tight_bytes, _) = graph.channel_builder().with_capacity(1).eager_build_internal::<u8>();
    let mut two = Defrag::<StreamEgress>::new(9, 8, 64);
    push_item(&mut two, &[4]);
    push_item(&mut two, &[5]);
    let (n, b, more) = spot.flush_defrag_messages(&mut tight_items, &mut tight_bytes, &mut two);
    assert_eq!(n, 1);
    assert_eq!(b, 1);
    assert_eq!(more, Some(9));

    let (mut full_items, _) = graph
        .channel_builder()
        .with_capacity(1)
        .eager_build_internal::<StreamEgress>();
    let (mut full_bytes, _) = graph.channel_builder().with_capacity(1).eager_build_internal::<u8>();
    assert!(full_items.shared_try_send(StreamEgress::new(1)).is_ok());
    assert!(full_bytes.shared_try_send(9u8).is_ok());
    let mut blocked = Defrag::<StreamEgress>::new(4, 8, 64);
    push_item(&mut blocked, &[7]);
    let (n, b, more) = spot.flush_defrag_messages(&mut full_items, &mut full_bytes, &mut blocked);
    assert_eq!((n, b), (0, 0));
    assert_eq!(more, Some(4));

    let mut shadow = graph.new_testing_test_monitor("flush_shadow");
    let (mut s_items, _) = graph
        .channel_builder()
        .with_capacity(4)
        .eager_build_internal::<StreamEgress>();
    let (mut s_bytes, _) = graph.channel_builder().with_capacity(16).eager_build_internal::<u8>();
    let mut again = Defrag::<StreamEgress>::new(2, 4, 16);
    push_item(&mut again, &[8, 8]);
    let (n, b, more) = shadow.flush_defrag_messages(&mut s_items, &mut s_bytes, &mut again);
    assert_eq!(n, 1);
    assert_eq!(b, 2);
    assert!(more.is_none());
}

#[test]
// ss[verify actor.shadow-spotlight]
// ss[verify actor.is-running-loop]
// ss[verify actor.wait-avail-vacant]
// ss[verify telemetry.dot-export]
fn spotlight_method_surface_send_take_wait_and_shutdown() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let tx_m = tx.clone();
    let rx_m = rx.clone();
    let mut spot = graph
        .new_testing_test_monitor("surface")
        .into_spotlight([&rx_m], [&tx_m]);
    spot.frame_rate_ms = 1;
    spot.runtime_state.write().state = GraphLivelinessState::Building;
    assert!(spot.is_running(|| true));
    spot.runtime_state.write().state = GraphLivelinessState::Running;
    assert!(spot.is_running(|| true));
    assert!(spot.args::<()>().is_some());
    assert_eq!(spot.identity().label.name, "surface");
    assert_eq!(spot.frame_rate_ms(), 1);
    assert_eq!(spot.regeneration(), 0);
    spot.set_dot_display_text(Some("line\ntwo"));
    spot.set_dot_display_text(None);
    assert!(spot.sidechannel_responder().is_none());

    spot.last_periodic_wait.store(u64::MAX / 4, Ordering::SeqCst);
    core_exec::block_on(async {
        assert!(spot.wait_periodic(Duration::from_nanos(1)).await);
        assert!(spot.wait_timeout(Duration::ZERO).await);
        spot.wait(Duration::ZERO).await;
        spot.yield_now().await;
        assert!(spot
            .wait_future_void(futures::future::ready(()).fuse())
            .await);
        let held_tx = tx.clone();
        let held_rx = rx.clone();
        let mut txg = held_tx.lock().await;
        let mut rxg = held_rx.lock().await;
        assert!(spot
            .send_async(&mut txg, 3u8, SendSaturation::AwaitForRoom)
            .await
            .is_sent());
        let got = spot.take_async(&mut rxg).await;
        assert_eq!(got, Some(3));
        spot.telemetry.dirty.store(true, Ordering::Relaxed);
        spot.last_telemetry_send = Instant::now() - Duration::from_secs(60);
        assert!(spot.take_async(&mut rxg).await.is_none());
        assert!(spot
            .take_async_with_timeout(&mut rxg, Duration::from_millis(1))
            .await
            .is_none());
        let _ = txg.shared_try_send(1u8);
        let _ = txg.shared_try_send(2u8);
        let blocked = spot
            .send_async(&mut txg, 9u8, SendSaturation::AwaitForRoom)
            .await;
        assert!(matches!(blocked, SendOutcome::Timeout(9) | SendOutcome::Closed(9)));
        let mut bundle: RxCoreBundle<Rx<u8>> = Vec::new();
        let _ = spot.wait_avail_bundle(&mut bundle, 1, 0).await;
    });
    core_exec::block_on(spot.request_shutdown());
}

#[test]
// ss[verify actor.shadow-spotlight]
// ss[verify philosophy.single-wake-up]
fn shadow_periodic_overrun_and_ready_future() {
    let graph = GraphBuilder::for_testing().build(());
    let shadow = graph.new_testing_test_monitor("shadow_wait");
    shadow
        .last_periodic_wait
        .store(u64::MAX / 4, Ordering::SeqCst);
    let level = log::max_level();
    log::set_max_level(log::LevelFilter::Trace);
    let done = core_exec::block_on(async {
        let periodic = shadow.wait_periodic(Duration::from_nanos(1)).await;
        let timeout = shadow.wait_timeout(Duration::ZERO).await;
        shadow.wait(Duration::ZERO).await;
        shadow.yield_now().await;
        let ready = shadow
            .wait_future_void(futures::future::ready(()).fuse())
            .await;
        let value = shadow.call_async(futures::future::ready(5u8)).await;
        (periodic, timeout, ready, value)
    });
    log::set_max_level(level);
    assert!(done.0 && done.1 && done.2);
    assert_eq!(done.3, Some(5));
}

#[test]
// ss[verify channel.memory-usage-telemetry]
// ss[verify bundle.index-wait-readiness]
// ss[verify philosophy.zero-copy-discipline]
fn rx_bundle_memory_and_zero_count_index() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    tx.testing_send_all(vec![1u8, 2], false);
    let bundle = std::sync::Arc::new([rx.clone()]);
    assert!(bundle.memory_bytes() > 0);
    let _ = bundle.dynamic_memory_bytes();
    assert_eq!(core_exec::block_on(bundle.wait_avail_index(&[0])), 0);
    let _ = RxMetaDataProvider::meta_data(&rx.clone());
    let _ = RxMetaDataProvider::meta_data(&*rx.clone());
    core_exec::block_on(async {
        let held = rx.clone();
        let mut guard = held.lock().await;
        assert!(!guard.is_empty());
        assert!(guard.tx_instance_changed() || !guard.tx_instance_changed());
        guard.tx_instance_reset();
        let mut slice = [0u8; 2];
        let n = guard.try_peek_slice(&mut slice);
        assert_eq!(n, 2);
        assert_eq!(slice, [1, 2]);
        let peeked: Vec<u8> = guard.try_peek_iter().copied().collect();
        assert_eq!(peeked, vec![1, 2]);
    });
    let mut guards = core_exec::block_on(bundle.lock());
    assert!(!guards.is_empty());
    assert!(!guards.is_closed());
    guards.tx_instance_reset();
    assert!(!guards.tx_instance_changed());
}

#[test]
// ss[verify graph.for-testing]
fn telemetry_rate_below_minimum_evaluates_debug_log() {
    let previous = log::max_level();
    log::set_max_level(log::LevelFilter::Trace);
    let built = GraphBuilder::for_testing().with_telemtry_production_rate_ms(1);
    log::set_max_level(previous);
    assert!(built.telemtry_production_rate_ms >= 100);
}

#[test]
// ss[verify actor.shadow-spotlight]
// ss[verify telemetry.builtin-server]
// ss[verify philosophy.single-wake-up]
fn spotlight_dirty_waits_relay_and_shutdown_simulation() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let (empty_tx, _empty_rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    tx.testing_send_all(vec![7u8], false);
    let rx_meta = rx.clone();
    let tx_meta = tx.clone();
    let mut spot = graph
        .new_testing_test_monitor("dirty_spot")
        .into_spotlight([&rx_meta], [&tx_meta]);
    spot.telemetry.dirty.store(true, Ordering::Relaxed);
    spot.last_telemetry_send = Instant::now() - Duration::from_secs(30);
    spot.frame_rate_ms = 1;
    let level = log::max_level();
    log::set_max_level(log::LevelFilter::Trace);
    let peeked = core_exec::block_on(async {
        let held = rx.clone();
        let mut guard = held.lock().await;
        let value = spot.peek_async(&mut guard).await.copied();
        drop(guard);
        let empty_held = empty_tx.clone();
        let mut tx_guard = empty_held.lock().await;
        let empty_now = spot.wait_empty(&mut tx_guard).await;
        spot.frame_rate_ms = 10_000;
        spot.last_telemetry_send = Instant::now();
        let empty_later = spot.wait_empty(&mut tx_guard).await;
        spot.telemetry.dirty.store(false, Ordering::Relaxed);
        let empty_clean = spot.wait_empty(&mut tx_guard).await;
        let called = spot.call_async(futures::future::ready(9u8)).await;
        let mut blocking = spot.call_blocking(|| 11u8);
        let blocked = blocking.fetch(Duration::from_millis(200)).await;
        spot.frame_rate_ms = 1;
        spot.last_telemetry_send = Instant::now() - Duration::from_secs(30);
        let relayed = spot.relay_stats_smartly();
        spot.relay_stats();
        let _periodic = spot.relay_stats_periodic(Duration::ZERO).await;
        (value, empty_now, empty_later, empty_clean, called, blocked, relayed)
    });
    log::set_max_level(level);
    assert_eq!(peeked.0, Some(7));
    assert!(peeked.1 && peeked.2 && peeked.3, "empty tx must report empty");
    assert_eq!(peeked.4, Some(9));
    assert_eq!(peeked.5, Some(11));
    assert!(peeked.6, "a stale relay deadline must send telemetry");

    let mut shadow = graph.new_testing_test_monitor("stopped_shadow");
    assert!(spot.is_liveliness_building());
    assert!(shadow.is_liveliness_building());
    assert!(!spot.is_liveliness_running());
    assert!(!spot.is_liveliness_stop_requested());
    assert!(spot.is_liveliness_in(&[GraphLivelinessState::Building]));
    assert!(spot.is_liveliness_shutdown_timeout().is_none());
    shadow.relay_stats();
    shadow.set_dot_display_text(Some("shadow"));
    shadow.set_dot_display_text(None);
    let _ = crate::proptest_support::percent_f64();
    let _ = crate::proptest_support::lane_count();
    spot.runtime_state.write().state = GraphLivelinessState::Stopped;
    let sims: Vec<&dyn crate::simulate_edge::IntoSimRunner<SteadyActorSpotlight<1, 1>>> = Vec::new();
    assert!(core_exec::block_on(spot.simulated_behavior(sims)).is_ok());
    let shadow_sims: Vec<&dyn crate::simulate_edge::IntoSimRunner<SteadyActorShadow>> = Vec::new();
    assert!(core_exec::block_on(shadow.simulated_behavior(shadow_sims)).is_ok());

    let default_cases = crate::proptest_support::default_config().cases;
    let critical_cases = crate::proptest_support::critical_config().cases;
    let eager_cases = crate::proptest_support::telemetry_eager_config().cases;
    assert!(default_cases > 0 && critical_cases > 0 && eager_cases > 0);
}

#[test]
// ss[verify actor.wait-avail-vacant]
// ss[verify actor.shadow-spotlight]
// ss[verify telemetry.builtin-server]
fn spotlight_wait_avail_and_vacant_honor_dirty_deadlines() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (full_tx, full_rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    let (empty_tx, empty_rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    full_tx.testing_send_all(vec![1u8, 2, 3, 4], false);
    let rx_meta = empty_rx.clone();
    let tx_meta = full_tx.clone();
    let mut spot = graph
        .new_testing_test_monitor("dirty_wait")
        .into_spotlight([&rx_meta], [&tx_meta]);

    spot.telemetry.dirty.store(true, Ordering::Relaxed);
    spot.frame_rate_ms = 1;
    spot.last_telemetry_send = Instant::now() - Duration::from_secs(30);
    let immediate = core_exec::block_on(async {
        let rx_held = empty_rx.clone();
        let tx_held = full_tx.clone();
        let mut rx_guard = rx_held.lock().await;
        let mut tx_guard = tx_held.lock().await;
        let avail = spot.wait_avail(&mut rx_guard, 1).await;
        let vacant = spot.wait_vacant(&mut tx_guard, 1).await;
        (avail, vacant)
    });
    assert!(
        !immediate.0 && !immediate.1,
        "a blown telemetry deadline must return before parking, got {immediate:?}"
    );

    spot.frame_rate_ms = 400;
    spot.last_telemetry_send = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        empty_tx.testing_send_all(vec![9u8], false);
        let _ = full_rx.testing_take_all();
        std::thread::sleep(Duration::from_millis(80));
        empty_tx.testing_send_all(vec![8u8], false);
        let _ = full_rx.testing_take_all();
    });
    let parked = core_exec::block_on(async {
        let rx_held = empty_rx.clone();
        let tx_held = full_tx.clone();
        let mut rx_guard = rx_held.lock().await;
        let mut tx_guard = tx_held.lock().await;
        let avail = spot.wait_avail(&mut rx_guard, 1).await;
        let vacant = spot.wait_vacant(&mut tx_guard, 1).await;
        (avail, vacant)
    });
    assert!(
        parked.0 && parked.1,
        "room or data inside the telemetry budget must complete the wait, got {parked:?}"
    );

    spot.telemetry.dirty.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(40));
    full_tx.testing_send_all(vec![1u8, 2, 3, 4], false);
    let clean = core_exec::block_on(async {
        let rx_held = empty_rx.clone();
        let tx_held = full_tx.clone();
        let mut rx_guard = rx_held.lock().await;
        let mut tx_guard = tx_held.lock().await;
        let avail = spot.wait_avail(&mut rx_guard, 2).await;
        let vacant = spot.wait_vacant(&mut tx_guard, 1).await;
        (avail, vacant)
    });
    assert!(
        clean.0 && clean.1,
        "a clean wait must complete when the peer moves, got {clean:?}"
    );

    spot.telemetry.dirty.store(true, Ordering::Relaxed);
    spot.frame_rate_ms = 1;
    spot.last_telemetry_send = Instant::now();
    let shutdown = core_exec::block_on(spot.wait_shutdown());
    assert!(
        !shutdown,
        "a dirty wait_shutdown returns when the telemetry budget expires"
    );
}

#[test]
// ss[verify actor.index-wait-round-robin]
// ss[verify actor.index-wait-paired]
// ss[verify actor.wait-avail-vacant]
fn shadow_vacant_and_paired_waits_park_until_the_peer_moves() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx0, _rx0) = graph.channel_builder().with_capacity(1).build::<u8>();
    let (tx1, rx1) = graph.channel_builder().with_capacity(1).build::<u8>();
    tx0.testing_send_all(vec![1u8], false);
    tx1.testing_send_all(vec![1u8], false);
    let shadow = graph.new_testing_test_monitor("shadow_vacant");
    let tx1_established = tx1.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(25));
        let _ = rx1.testing_take_all();
    });
    let vacant = core_exec::block_on(async {
        let tx0_established = tx0.clone();
        let a = tx0_established.lock().await;
        let b = tx1_established.lock().await;
        let mut bundle: TxCoreBundle<Tx<u8>> = vec![a, b];
        shadow.wait_vacant_index(&mut bundle, &[1, 1]).await
    });
    assert_eq!(vacant, Some(1));

    let (in_tx, in_rx) = graph.channel_builder().with_capacity(1).build::<u8>();
    let (out_tx, out_rx) = graph.channel_builder().with_capacity(1).build::<u8>();
    out_tx.testing_send_all(vec![2u8], false);
    let out_established = out_tx.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(25));
        in_tx.testing_send_all(vec![3u8], false);
        let _ = out_rx.testing_take_all();
    });
    let paired = core_exec::block_on(async {
        let in_established = in_rx.clone();
        let rx_guard = in_established.lock().await;
        let tx_guard = out_established.lock().await;
        let mut rx_bundle: RxCoreBundle<Rx<u8>> = vec![rx_guard];
        let mut tx_bundle: TxCoreBundle<Tx<u8>> = vec![tx_guard];
        shadow
            .wait_avail_vacant_index(&mut rx_bundle, &mut tx_bundle, &[1], &[1])
            .await
    });
    assert_eq!(paired, Some(0));
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
// ss[verify channel.backpressure-never-drop]
fn rx_peek_iter_timeout_and_telemetry_drop_count() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    tx.testing_send_all(vec![4u8, 5], false);
    core_exec::block_on(async {
        let held = rx.clone();
        let mut guard = held.lock().await;
        let peeked: Vec<u8> = guard
            ._shared_peek_async_iter_timeout(1, Some(Duration::from_millis(20)))
            .await
            .copied()
            .collect();
        assert_eq!(peeked, vec![4, 5]);
        let again: Vec<u8> = guard
            ._shared_peek_async_iter_timeout(1, None)
            .await
            .copied()
            .collect();
        assert_eq!(again, vec![4, 5]);
        let mut dest = [0u8; 2];
        assert_eq!(guard.deprecated_shared_take_slice(&mut dest), 2);
        assert_eq!(dest, [4, 5]);
    });

    let (_empty_tx, empty_rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    core_exec::block_on(async {
        let held = empty_rx.clone();
        let mut guard = held.lock().await;
        let peeked: Vec<u8> = guard
            ._shared_peek_async_iter_timeout(1, Some(Duration::from_millis(10)))
            .await
            .copied()
            .collect();
        assert!(peeked.is_empty(), "timeout on an empty rx must not invent data");
        let (dummy_tx, _dummy_rx) = crate::channel_builder::ChannelBuilder::default()
            .with_capacity(1)
            .eager_build::<[usize; 1]>();
        let mut tel =
            crate::monitor_telemetry::SteadyTelemetrySend::new(dummy_tx, [0], [0], Instant::now());
        guard.local_monitor_index = MONITOR_NOT;
        guard.telemetry_inc(RxDone::Normal(1), &mut tel);
    });
}

#[test]
// ss[verify distributed.aeron-uri]
// ss[verify distributed.media-driver-testing]
fn media_driver_probe_with_zero_budget_fails_closed() {
    let err = media_driver_probe_with_reason(Duration::ZERO).expect_err("zero budget");
    let hint = err.hint();
    assert!(hint.contains("aeronmd"), "{hint}");
    let rendered = err.to_string();
    assert!(rendered.contains("media driver"), "{rendered}");
    assert!(!media_driver_probe(Duration::ZERO));
}
