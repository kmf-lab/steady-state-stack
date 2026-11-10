//! Round-2 coverage for spotlight/shadow index waits, slice telemetry,
//! and send-bundle vacancy waits.
//!
//! A lane that is not ready is parked on the ring, then the other end
//! delivers one item. Shutdown that is already fired returns `None`
//! without a timer.

// ss[related actor.index-wait-round-robin]
use std::sync::atomic::Ordering;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};

// ss[related philosophy.structural-hierarchy]
use futures::FutureExt;
use crate::*;

fn deliver_after(tx: LazySteadyTx<u8>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(25));
        tx.testing_send_all(vec![9u8], false);
    });
}

fn consume_after(rx: LazySteadyRx<u8>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(25));
        let _ = rx.testing_take_all();
    });
}

/// Both lanes start empty, so the fast path misses and the wait parks.
/// Lane 1 then receives one byte.
#[test]
// ss[verify actor.index-wait-round-robin]
// ss[verify actor.wait-avail-vacant]
// ss[verify philosophy.pull-reactor]
fn spotlight_wait_avail_index_parks_until_lane_ready() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (_tx0, rx0) = graph.channel_builder().with_capacity(4).build::<u8>();
    let (tx1, rx1) = graph.channel_builder().with_capacity(4).build::<u8>();
    let rx0_meta = rx0.clone();
    let rx1_meta = rx1.clone();
    let spot = graph
        .new_testing_test_monitor("spot_avail")
        .into_spotlight([&rx0_meta, &rx1_meta], []);
    deliver_after(tx1);
    let idx = core_exec::block_on(async {
        let held0 = rx0.clone();
        let held1 = rx1.clone();
        let g0 = held0.lock().await;
        let g1 = held1.lock().await;
        let mut bundle: RxCoreBundle<Rx<u8>> = vec![g0, g1];
        spot.wait_avail_index(&mut bundle, &[1, 1]).await
    });
    assert_eq!(idx, Some(1));
}

#[test]
// ss[verify actor.index-wait-round-robin]
// ss[verify actor.wait-avail-vacant]
fn shadow_wait_avail_index_parks_until_lane_ready() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (_tx0, rx0) = graph.channel_builder().with_capacity(4).build::<u8>();
    let (tx1, rx1) = graph.channel_builder().with_capacity(4).build::<u8>();
    let shadow = graph.new_testing_test_monitor("shadow_avail");
    deliver_after(tx1);
    let idx = core_exec::block_on(async {
        let held0 = rx0.clone();
        let held1 = rx1.clone();
        let g0 = held0.lock().await;
        let g1 = held1.lock().await;
        let mut bundle: RxCoreBundle<Rx<u8>> = vec![g0, g1];
        shadow.wait_avail_index(&mut bundle, &[1, 1]).await
    });
    assert_eq!(idx, Some(1));
}

#[test]
// ss[verify actor.index-wait-round-robin]
// ss[verify channel.backpressure-never-drop]
fn spotlight_wait_vacant_index_parks_until_a_slot_opens() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx0, _rx0) = graph.channel_builder().with_capacity(1).build::<u8>();
    let (tx1, rx1) = graph.channel_builder().with_capacity(1).build::<u8>();
    tx0.testing_send_all(vec![1u8], false);
    tx1.testing_send_all(vec![2u8], false);
    let tx0_meta = tx0.clone();
    let tx1_meta = tx1.clone();
    let spot = graph
        .new_testing_test_monitor("spot_vacant")
        .into_spotlight([], [&tx0_meta, &tx1_meta]);
    consume_after(rx1);
    let idx = core_exec::block_on(async {
        let held0 = tx0.clone();
        let held1 = tx1.clone();
        let g0 = held0.lock().await;
        let g1 = held1.lock().await;
        let mut bundle: TxCoreBundle<Tx<u8>> = vec![g0, g1];
        spot.wait_vacant_index(&mut bundle, &[1, 1]).await
    });
    assert_eq!(idx, Some(1));
}

#[test]
// ss[verify actor.index-wait-paired]
// ss[verify actor.wait-avail-vacant]
fn spotlight_paired_index_waits_only_for_the_missing_side() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let (free_tx, _free_rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let rx_meta = rx.clone();
    let free_meta = free_tx.clone();
    let spot = graph
        .new_testing_test_monitor("spot_pair")
        .into_spotlight([&rx_meta], [&free_meta]);
    deliver_after(tx);
    let idx = core_exec::block_on(async {
        let held_rx = rx.clone();
        let held_tx = free_meta.clone();
        let rxg = held_rx.lock().await;
        let txg = held_tx.lock().await;
        let mut rx_bundle: RxCoreBundle<Rx<u8>> = vec![rxg];
        let mut tx_bundle: TxCoreBundle<Tx<u8>> = vec![txg];
        spot.wait_avail_vacant_index(&mut rx_bundle, &mut tx_bundle, &[1], &[1])
            .await
    });
    assert_eq!(idx, Some(0));
}

#[test]
// ss[verify actor.index-wait-round-robin]
// ss[verify graph.shutdown.veto]
fn index_wait_returns_none_when_shutdown_is_already_fired() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (_tx, rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let rx_meta = rx.clone();
    let mut spot = graph
        .new_testing_test_monitor("spot_shut")
        .into_spotlight([&rx_meta], []);
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    drop(sender);
    spot.oneshot_shutdown = receiver.shared();
    let idx = core_exec::block_on(async {
        let held = rx.clone();
        let g = held.lock().await;
        let mut bundle: RxCoreBundle<Rx<u8>> = vec![g];
        spot.wait_avail_index(&mut bundle, &[1]).await
    });
    assert!(idx.is_none());
    let idx = core_exec::block_on(async {
        let held = rx.clone();
        let g = held.lock().await;
        let mut bundle: RxCoreBundle<Rx<u8>> = vec![g];
        spot.wait_avail_index(&mut bundle, &[0]).await
    });
    assert!(idx.is_none(), "all-zero counts wait on no lane");
}

#[test]
// ss[verify actor.wait-avail-vacant]
// ss[verify telemetry.prometheus-metrics]
fn dirty_telemetry_short_circuits_a_wait_on_an_empty_lane() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(2).build::<u8>();
    let rx_meta = rx.clone();
    let tx_meta = tx.clone();
    let mut spot = graph
        .new_testing_test_monitor("spot_dirty")
        .into_spotlight([&rx_meta], [&tx_meta]);
    spot.telemetry.dirty.store(true, Ordering::Relaxed);
    spot.last_telemetry_send = Instant::now() - Duration::from_secs(30);
    spot.frame_rate_ms = 1;
    spot.runtime_state.write().state = GraphLivelinessState::Running;
    core_exec::block_on(async {
        let held_rx = rx.clone();
        let held_tx = tx.clone();
        let mut rxg = held_rx.lock().await;
        let mut txg = held_tx.lock().await;
        assert!(
            !spot.wait_avail(&mut rxg, 1).await,
            "overdue telemetry must not park on an empty lane"
        );
        // Positive remaining budget: the delay arm returns before any byte arrives.
        spot.last_telemetry_send = Instant::now();
        spot.frame_rate_ms = 1;
        assert!(!spot.wait_avail(&mut rxg, 1).await);
        drop(txg);
        tx.testing_send_all(vec![1u8, 2], false);
        let held_tx = tx.clone();
        let mut txg = held_tx.lock().await;
        assert!(
            !spot.wait_vacant(&mut txg, 1).await,
            "overdue telemetry must not park on a full lane"
        );
        assert!(!spot.wait_shutdown().await);
    });
}

#[test]
// ss[verify actor.shadow-spotlight]
// ss[verify philosophy.zero-copy-discipline]
// ss[verify telemetry.prometheus-metrics]
fn spotlight_and_shadow_slice_paths_keep_order() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(8).build::<u8>();
    let tx_s = tx.clone();
    let rx_s = rx.clone();
    let mut spot = graph
        .new_testing_test_monitor("spot_slice")
        .into_spotlight([&rx_s], [&tx_s]);
    core_exec::block_on(async {
        let held_tx = tx.clone();
        let held_rx = rx.clone();
        let mut txg = held_tx.lock().await;
        let mut rxg = held_rx.lock().await;
        assert_eq!(spot.send_slice(&mut txg, &[1u8, 2, 3][..]), TxDone::Normal(3));
        let mut buf = [0u8; 2];
        assert_eq!(spot.take_slice(&mut rxg, &mut buf), RxDone::Normal(2));
        assert_eq!(buf, [1, 2]);
        assert_eq!(spot.try_peek(&mut rxg), Some(&3));
        assert_eq!(spot.try_peek_iter(&mut rxg).copied().collect::<Vec<_>>(), vec![3]);
        assert!(!spot.is_empty(&mut rxg));
        assert!(spot.avail_units(&mut rxg) >= 1);
        let _ = spot.peek_slice(&mut rxg);
        let _ = spot.poke_slice(&mut txg);
        let _ = spot.advance_take_index(&mut rxg, 1);
        assert!(spot.is_empty(&mut rxg));
        let _ = spot.send_iter_until_full(&mut txg, [4u8, 5].into_iter());
        let _ = spot.advance_send_index(&mut txg, 0);
        assert!(!spot.is_full(&mut txg));
        assert!(spot.vacant_units(&mut txg) >= 1);
        let taken: Vec<u8> = spot.take_into_iter(&mut rxg).collect();
        assert_eq!(taken, vec![4, 5]);
    });

    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    let mut shadow = graph.new_testing_test_monitor("shadow_slice");
    core_exec::block_on(async {
        let held_tx = tx.clone();
        let held_rx = rx.clone();
        let mut txg = held_tx.lock().await;
        let mut rxg = held_rx.lock().await;
        assert_eq!(shadow.send_slice(&mut txg, &[8u8, 9][..]), TxDone::Normal(2));
        let mut buf = [0u8; 1];
        assert_eq!(shadow.take_slice(&mut rxg, &mut buf), RxDone::Normal(1));
        assert_eq!(buf, [8]);
        assert_eq!(shadow.try_peek(&mut rxg), Some(&9));
        let _ = shadow.advance_take_index(&mut rxg, 1);
        let sent = shadow.send_iter_until_full(&mut txg, [1u8].into_iter());
        assert_eq!(sent, 1);
        assert_eq!(shadow.try_take(&mut rxg), Some(1));
        assert!(shadow.aeron_media_driver().is_none());
    });
}

#[test]
// ss[verify channel.backpressure-never-drop]
// ss[verify channel.memory-usage-telemetry]
fn tx_bundle_wait_vacant_and_memory_when_room_exists() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx0, _rx0) = graph.channel_builder().with_capacity(4).build::<u8>();
    let (tx1, _rx1) = graph.channel_builder().with_capacity(4).build::<u8>();
    let bundle = std::sync::Arc::new([tx0.clone(), tx1.clone()]);
    core_exec::block_on(bundle.wait_vacant_units(1, 1));
    assert_eq!(core_exec::block_on(bundle.wait_vacant_index(&[0, 1])), 0);
    assert_eq!(core_exec::block_on(bundle.wait_vacant_index(&[1, 1])), 0);
    let _ = bundle.meta_data();
    let _ = bundle.memory_bytes();
    let _ = bundle.dynamic_memory_bytes();
    let steady = tx0.clone();
    let _ = TxMetaDataProvider::meta_data(&steady);
    let _ = TxMetaDataProvider::meta_data(&*steady);
    let mut guards = core_exec::block_on(bundle.lock());
    assert!(guards.mark_closed());
    assert!(guards.is_all_empty());
}
