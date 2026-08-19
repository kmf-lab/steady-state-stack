// ss[related actor.shadow-spotlight]
#![allow(deprecated)]
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::Ordering;
// ss[related philosophy.structural-hierarchy]
use std::sync::{Arc, Mutex};
// ss[related actor.shadow-spotlight]
use std::time::Duration;

// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;

// ss[related actor.shadow-spotlight]
use crate::proptest_support::{capacity, lane_mask, vote_matrix};
// ss[related philosophy.structural-hierarchy]
use crate::*;

/// Minimal internal_behavior consumer: drains `rx` until shutdown veto accepts.
// ss[related actor.shadow-spotlight]
async fn spotlight_consumer_internal<A: SteadyActor>(
    mut actor: A,
    rx: SteadyRx<i32>,
    votes: Arc<Mutex<Vec<Vec<bool>>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut rx = rx.lock().await;
    let mut round = 0usize;
    while actor.is_running(|| {
        let guard = votes.lock().expect("votes lock");
        let accept = guard
            .get(round)
            .and_then(|row| row.first())
            .copied()
            .unwrap_or(true);
        round += 1;
        accept
    }) {
        let _ = await_for_all!(actor.wait_avail(&mut rx, 1));
        let _ = actor.try_take(&mut rx);
    }
    Ok(())
}

ss_proptest! {

    /// Property: spotlight `wait_avail` succeeds when enough items are queued.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_avail_when_ready(
        cap in capacity(),
        need in 1usize..8,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx.testing_send_all(vec![1i32; need], false);
        let shadow = graph.new_testing_test_monitor("spot_avail");
        let rx_steady = rx.clone();
        let spotlight = shadow.into_spotlight([&rx_steady], []);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_avail(&mut rx_guard, need));
        prop_assert!(ready);
    }

    /// Property: spotlight `wait_avail` returns false once graph shutdown is requested.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_avail_aborts_on_shutdown(
        cap in capacity(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("spot_shutdown");
        let rx_steady = rx.clone();
        let spotlight = shadow.into_spotlight([&rx_steady], []);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_avail(&mut rx_guard, 1));
        prop_assert!(!ready);
    }

    /// Property: spotlight `wait_vacant` succeeds on an empty channel.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_vacant_when_room(
        cap in capacity(),
        need in 1usize..8,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("spot_vacant");
        let tx_steady = tx.clone();
        let spotlight = shadow.into_spotlight([], [&tx_steady]);
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_vacant(&mut tx_guard, need));
        prop_assert!(ready);
    }

    /// Property: dirty telemetry short-circuits `wait_vacant` without blocking forever.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_vacant_dirty_telemetry_yields(
        cap in 2usize..16,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(40)
            .build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        for _ in 0..cap {
            let tx_fill = tx.clone();
            if let Some(mut g) = tx_fill.try_lock() {
                let _ = g.shared_try_send(1i32);
            }
        }
        let shadow = graph.new_testing_test_monitor("spot_dirty");
        let tx_steady = tx.clone();
        let mut spotlight = shadow.into_spotlight([], [&tx_steady]);
        spotlight.telemetry.dirty.store(true, Ordering::Relaxed);
        spotlight.last_telemetry_send =
            std::time::Instant::now() - Duration::from_millis(spotlight.frame_rate_ms + 5);
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let ready = core_exec::block_on(spotlight.wait_vacant(&mut tx_guard, 1));
        prop_assert!(!ready);
    }

    /// Property: `relay_stats_smartly` eventually fires when frame rate elapses.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_relay_stats_smartly_fires(
        rate_ms in 10u64..80,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(rate_ms)
            .build(());
        let (tx, rx) = graph.channel_builder().with_capacity(8).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("spot_relay");
        let rx_steady = rx.clone();
        let tx_steady = tx.clone();
        let mut spotlight = shadow.into_spotlight([&rx_steady], [&tx_steady]);
        spotlight.last_telemetry_send =
            std::time::Instant::now() - Duration::from_millis(rate_ms + 5);
        let sent = spotlight.relay_stats_smartly();
        prop_assert!(sent);
    }

    /// Property: `try_send`/`try_take` through spotlight preserves FIFO order.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_try_send_take_fifo(
        cap in capacity(),
        messages in prop::collection::vec(any::<i32>(), 1..16),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("spot_fifo");
        let tx_steady = tx.clone();
        let rx_steady = rx.clone();
        let mut spotlight = shadow.into_spotlight([], [&tx_steady]);
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        for &m in &messages {
            prop_assert!(spotlight.try_send(&mut tx_guard, m).is_sent());
        }
        drop(tx_guard);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let mut taken = Vec::new();
        while let Some(v) = spotlight.try_take(&mut rx_guard) {
            taken.push(v);
        }
        prop_assert_eq!(taken, messages);
    }

    /// Property: batch slice send/take through spotlight matches message count.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_send_take_slice_count(
        cap in 4usize..32,
        batch in 1usize..8,
    ) {
        prop_assume!(batch <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<u64>();
        let msgs: Vec<u64> = (0..batch as u64).collect();
        let shadow = graph.new_testing_test_monitor("spot_slice");
        let tx_steady = tx.clone();
        let rx_steady = rx.clone();
        let mut spotlight = shadow.into_spotlight([&rx_steady], [&tx_steady]);
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let sent = spotlight.send_slice(&mut tx_guard, &msgs).item_count();
        prop_assert_eq!(sent, batch);
        drop(tx_guard);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let mut buf = vec![0u64; batch];
        let taken = spotlight.take_slice(&mut rx_guard, &mut buf).item_count();
        prop_assert_eq!(taken, batch);
        prop_assert_eq!(&buf[..batch], &msgs[..]);
    }

    /// Property: internal_behavior spotlight consumer honors `vote_matrix` shutdown vetoes.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.shutdown.veto]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_internal_behavior_vote_matrix(
        vote_rounds in vote_matrix(1),
        cap in 2usize..16,
        msg_count in 1usize..8,
    ) {
        prop_assume!(msg_count <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let votes = Arc::new(Mutex::new(vote_rounds));
        let votes_actor = votes.clone();
        let rx_actor = rx.clone();
        graph
            .actor_builder()
            .with_name("SPOT_CONSUMER")
            .build(
                move |ctx| {
                    let votes = votes_actor.clone();
                    let rx = rx_actor.clone();
                    async move {
                        let rx_steady = rx.clone();
                        let actor = ctx.into_spotlight([&rx_steady], []);
                        spotlight_consumer_internal(actor, rx_steady, votes).await
                    }
                },
                SoloAct,
            );
        graph.start();
        tx.testing_send_all(vec![1i32; msg_count], false);
        graph.request_shutdown();
        let stopped = graph.block_until_stopped(Duration::from_secs(3));
        prop_assert!(stopped.is_ok());
    }

    /// Property: `wait_shutdown` completes after graph requests shutdown.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.request-shutdown]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_shutdown_after_request(
        rate_ms in 10u64..80,
    ) {
        let mut graph = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(rate_ms)
            .build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("spot_wait_shutdown");
        let spotlight = shadow.into_spotlight([], []);
        let done = core_exec::block_on(spotlight.wait_shutdown());
        prop_assert!(done);
    }

    /// Property: `wait_periodic` overrun path stores future deadline without panic.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_periodic_overrun(
        overrun_ms in 1u64..50,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("spot_periodic");
        let mut spotlight = shadow.into_spotlight([], []);
        let real_now = spotlight.actor_start_time.elapsed().as_nanos() as u64;
        spotlight
            .last_periodic_wait
            .store(real_now + overrun_ms * 1_000_000, Ordering::SeqCst);
        let ok = core_exec::block_on(spotlight.wait_periodic(Duration::from_millis(10)));
        prop_assert!(ok);
    }

    /// Property: spotlight `wait_vacant_index` picks a lane with sufficient vacancy.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_vacant_index_lane_mask(
        cap in 2usize..16,
        mask in lane_mask(2),
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        prop_assume!(mask != 0);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, _rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, _rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let tx0_steady = tx0.clone();
        let tx1_steady = tx1.clone();
        core_exec::block_on(async {
            if mask & 1 != 0 {
                let mut g = tx0_steady.lock().await;
                for _ in 0..cap.saturating_sub(need) {
                    let _ = g.shared_try_send(1i32);
                }
            } else {
                let mut g = tx0_steady.lock().await;
                for _ in 0..cap {
                    let _ = g.shared_try_send(1i32);
                }
            }
            if mask & 2 != 0 {
                let mut g = tx1_steady.lock().await;
                for _ in 0..cap.saturating_sub(need) {
                    let _ = g.shared_try_send(1i32);
                }
            } else {
                let mut g = tx1_steady.lock().await;
                for _ in 0..cap {
                    let _ = g.shared_try_send(1i32);
                }
            }
        });
        let counts = [need, need];
        let shadow = graph.new_testing_test_monitor("spot_vac_idx");
        let idx = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([], [&tx0_steady, &tx1_steady]);
            let mut bundle = TxBundle::new();
            bundle.push(tx0_steady.lock().await);
            bundle.push(tx1_steady.lock().await);
            spotlight.wait_vacant_index(&mut bundle, &counts).await
        });
        if mask & 1 != 0 && mask & 2 == 0 {
            prop_assert_eq!(idx, Some(0));
        } else if mask & 2 != 0 && mask & 1 == 0 {
            prop_assert_eq!(idx, Some(1));
        } else {
            prop_assert!(idx.is_some());
        }
    }

    /// Property: paired `wait_avail_vacant_index` through spotlight returns ready lane.
    #[test]
    // ss[verify actor.index-wait-paired]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_avail_vacant_index_paired(
        cap in 2usize..16,
        mask in lane_mask(2),
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        prop_assume!(mask != 0);
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx0, in_rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (out_tx0, _out_rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (in_tx1, in_rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (out_tx1, _out_rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        if mask & 1 != 0 {
            in_tx0.testing_send_all(vec![1i32; need], false);
        }
        if mask & 2 != 0 {
            in_tx1.testing_send_all(vec![2i32; need], false);
            let out_tx1_steady = out_tx1.clone();
            if let Some(mut g) = out_tx1_steady.try_lock() {
                for _ in 0..cap.saturating_sub(need) {
                    let _ = g.shared_try_send(1i32);
                }
            }
        }
        let shadow = graph.new_testing_test_monitor("spot_paired");
        let in_rx0_steady = in_rx0.clone();
        let in_rx1_steady = in_rx1.clone();
        let out_tx0_steady = out_tx0.clone();
        let out_tx1_steady = out_tx1.clone();
        let idx = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight(
                [&in_rx0_steady, &in_rx1_steady],
                [&out_tx0_steady, &out_tx1_steady],
            );
            let mut rx_bundle = RxBundle::new();
            rx_bundle.push(in_rx0_steady.try_lock().expect("in0"));
            rx_bundle.push(in_rx1_steady.try_lock().expect("in1"));
            let mut tx_bundle = TxBundle::new();
            tx_bundle.push(out_tx0_steady.try_lock().expect("out0"));
            tx_bundle.push(out_tx1_steady.try_lock().expect("out1"));
            spotlight
                .wait_avail_vacant_index(
                    &mut rx_bundle,
                    &mut tx_bundle,
                    &[need, need],
                    &[need, need],
                )
                .await
        });
        if mask & 1 == 0 && mask & 2 != 0 {
            prop_assert_eq!(idx, Some(1));
        } else if mask & 2 == 0 && mask & 1 != 0 {
            prop_assert_eq!(idx, Some(0));
        } else if mask & 3 == 3 {
            prop_assert!(idx.is_some());
        }
    }

    /// Property: spotlight `wait_empty` succeeds on a drained transmitter.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_empty_on_drained_tx(
        cap in capacity(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("spot_empty");
        let tx_steady = tx.clone();
        let empty = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([], [&tx_steady]);
            let mut tx_guard = tx_steady.lock().await;
            spotlight.wait_empty(&mut tx_guard).await
        });
        prop_assert!(empty);
    }

    /// Property: spotlight `wait_timeout` aborts after shutdown request.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.request-shutdown]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_timeout_aborts_on_shutdown(
        delay_ms in 50u64..150,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("spot_timeout");
        let spotlight = shadow.into_spotlight([], []);
        let ok = core_exec::block_on(spotlight.wait_timeout(Duration::from_millis(delay_ms)));
        prop_assert!(!ok);
    }

    /// Property: `is_showstopper` through spotlight after repeated peeks.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_is_showstopper_threshold(
        threshold in 2usize..6,
        cap in 4usize..16,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<u8>();
        tx.testing_send_all(vec![99], false);
        let shadow = graph.new_testing_test_monitor("spot_showstopper");
        let rx_steady = rx.clone();
        let mut spotlight = shadow.into_spotlight([&rx_steady], []);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        for _ in 0..threshold + 1 {
            let _ = spotlight.try_peek(&mut rx_guard);
        }
        prop_assert!(spotlight.is_showstopper(&mut rx_guard, threshold));
    }

    /// Property: deprecated `wait_avail_bundle` through spotlight succeeds when ready.
    #[test]
    // ss[verify bundle.deprecated-bundle-waits]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_avail_bundle(
        cap in 2usize..16,
        lanes in 1usize..3,
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx0.testing_send_all(vec![1i32; need], false);
        if lanes > 1 {
            tx1.testing_send_all(vec![2i32; need], false);
        }
        let shadow = graph.new_testing_test_monitor("spot_avail_bundle");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let ok = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([&rx0_steady, &rx1_steady], []);
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.try_lock().expect("rx0"));
            if lanes > 1 {
                bundle.push(rx1_steady.try_lock().expect("rx1"));
            }
            spotlight.wait_avail_bundle(&mut bundle, need, lanes).await
        });
        prop_assert!(ok);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Property: two-lane `wait_avail_index` returns lane 0 then advances round-robin to lane 1.
    #[test]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_spotlight_wait_avail_index_round_robin(
        cap in 2usize..8,
        per_lane in 1usize..4,
    ) {
        prop_assume!(per_lane <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx0.testing_send_all(vec![1i32; per_lane], false);
        tx1.testing_send_all(vec![2i32; per_lane], false);
        let counts = [per_lane, per_lane];
        let shadow = graph.new_testing_test_monitor("spot_idx");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let first = core_exec::block_on(async {
            let spotlight = shadow.clone().into_spotlight([&rx0_steady, &rx1_steady], []);
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.lock().await);
            bundle.push(rx1_steady.lock().await);
            spotlight.wait_avail_index(&mut bundle, &counts).await
        });
        prop_assert_eq!(first, Some(0));
        shadow
            .index_wait_last_avail
            .store(0, Ordering::Relaxed);
        let second = core_exec::block_on(async {
            let spotlight = shadow.into_spotlight([&rx0_steady, &rx1_steady], []);
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.lock().await);
            bundle.push(rx1_steady.lock().await);
            spotlight.wait_avail_index(&mut bundle, &counts).await
        });
        prop_assert_eq!(second, Some(1));
    }
}
