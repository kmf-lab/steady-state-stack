//! Property tests for spotlight wait helpers in `wait.rs`.
//! Intentionally exercises deprecated bundle waits alongside index waits.
#![allow(deprecated)]

use std::sync::atomic::Ordering;
use std::time::Duration;

use proptest::prelude::*;

use crate::*;

ss_proptest! {
    /// Property: `wait_avail` dirty telemetry with zero remaining yields without blocking.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_dirty_telemetry_zero_remaining(
        cap in 2usize..16,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(40)
            .build(());
        let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("wait_avail_dirty");
        let rx_steady = rx.clone();
        let mut spotlight = shadow.into_spotlight([&rx_steady], []);
        spotlight.telemetry.dirty.store(true, Ordering::Relaxed);
        spotlight.last_telemetry_send =
            std::time::Instant::now() - Duration::from_millis(spotlight.frame_rate_ms + 5);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_avail(&mut rx_guard, 1));
        prop_assert!(!ready);
    }

    /// Property: `wait_avail` dirty telemetry with positive remaining uses delay branch.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_dirty_telemetry_with_delay(
        cap in 4usize..16,
        rate_ms in 20u64..80,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(rate_ms)
            .build(());
        let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("wait_avail_delay");
        let rx_steady = rx.clone();
        let mut spotlight = shadow.into_spotlight([&rx_steady], []);
        spotlight.telemetry.dirty.store(true, Ordering::Relaxed);
        spotlight.last_telemetry_send = std::time::Instant::now();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_avail(&mut rx_guard, 1));
        prop_assert!(!ready);
    }

    /// Property: `wait_vacant_bundle` succeeds when all lanes have room.
    #[test]
    // ss[verify bundle.deprecated-bundle-waits]
    // ss[verify verify.process.proptest]
    fn proptest_wait_vacant_bundle_when_room(
        cap in 2usize..16,
        need in 1usize..4,
        lanes in 1usize..3,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, _rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, _rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("wait_vac_bundle");
        let tx0_steady = tx0.clone();
        let tx1_steady = tx1.clone();
        let spotlight = shadow.into_spotlight([], [&tx0_steady, &tx1_steady]);
        let ready = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0_steady.lock().await);
            if lanes > 1 {
                bundle.push(tx1_steady.lock().await);
            }
            spotlight.wait_vacant_bundle(&mut bundle, need, lanes).await
        });
        prop_assert!(ready);
    }

    /// Property: `wait_shutdown` dirty telemetry while running returns false when overdue.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_wait_shutdown_dirty_overdue_while_running(
        rate_ms in 10u64..60,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(rate_ms)
            .build(());
        graph.start();
        let shadow = graph.new_testing_test_monitor("wait_shut_dirty");
        let mut spotlight = shadow.into_spotlight([], []);
        spotlight.telemetry.dirty.store(true, Ordering::Relaxed);
        spotlight.last_telemetry_send =
            std::time::Instant::now() - Duration::from_millis(rate_ms + 5);
        let done = core_exec::block_on(spotlight.wait_shutdown());
        prop_assert!(!done);
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(2)).expect("stop");
    }

    /// Property: deprecated `wait_avail_bundle` aborts when graph shutdown is requested.
    #[test]
    // ss[verify bundle.deprecated-bundle-waits]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_bundle_aborts_on_shutdown(
        cap in 2usize..16,
        lanes in 1usize..3,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (_tx0, rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (_tx1, rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("avail_bundle_shut");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let ready = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([&rx0_steady, &rx1_steady], []);
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.try_lock().expect("rx0"));
            if lanes > 1 {
                bundle.push(rx1_steady.try_lock().expect("rx1"));
            }
            spotlight.wait_avail_bundle(&mut bundle, 1, lanes).await
        });
        prop_assert!(!ready);
    }

    /// Property: `wait_avail_index` returns a lane that already has data.
    #[test]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_index_picks_ready_lane(
        cap in 2usize..16,
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (_tx1, rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx0.testing_send_all(vec![1i32; need], false);
        let shadow = graph.new_testing_test_monitor("avail_idx");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let picked = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([&rx0_steady, &rx1_steady], []);
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.try_lock().expect("rx0"));
            bundle.push(rx1_steady.try_lock().expect("rx1"));
            spotlight.wait_avail_index(&mut bundle, &[need, need]).await
        });
        prop_assert_eq!(picked, Some(0));
    }
}
