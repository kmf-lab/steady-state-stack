// ss[related actor.shadow-spotlight]
#![allow(deprecated)]
// ss[related philosophy.structural-hierarchy]
use std::sync::{Arc, Mutex};
// ss[related philosophy.structural-hierarchy]
use std::time::Duration;

// ss[related actor.shadow-spotlight]
use proptest::prelude::*;

// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, lane_mask, vote_matrix};
// ss[related actor.shadow-spotlight]
use crate::*;

/// Shadow-path internal_behavior: drain `rx` honoring shutdown vote rounds.
// ss[related actor.shadow-spotlight]
async fn shadow_consumer_internal(
    mut actor: SteadyActorShadow,
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

    /// Property: shadow `wait_avail` succeeds when the channel already has data.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_avail_when_ready(
        cap in capacity(),
        need in 1usize..8,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx.testing_send_all(vec![7i32; need], false);
        let shadow = graph.new_testing_test_monitor("shadow_avail");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(shadow.wait_avail(&mut rx_guard, need));
        prop_assert!(ready);
    }

    /// Property: shadow `wait_avail` aborts after shutdown is requested.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_avail_aborts_on_shutdown(
        cap in capacity(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("shadow_shutdown");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let ready = core_exec::block_on(shadow.wait_avail(&mut rx_guard, 1));
        prop_assert!(!ready);
    }

    /// Property: shadow `wait_vacant` succeeds on a channel with room.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_vacant_when_room(
        cap in capacity(),
        need in 1usize..8,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("shadow_vacant");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let ready = core_exec::block_on(shadow.wait_vacant(&mut tx_guard, need));
        prop_assert!(ready);
    }

    /// Property: shadow `wait_empty` returns true on an empty transmitter.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_empty_on_empty_tx(
        cap in capacity(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("shadow_empty");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let empty = core_exec::block_on(shadow.wait_empty(&mut tx_guard));
        prop_assert!(empty);
    }

    /// Property: two-lane `wait_avail_index` respects `lane_mask` zero-count lanes.
    #[test]
    // ss[verify actor.index-wait-truthful]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_avail_index_lane_mask(
        cap in 2usize..16,
        mask in lane_mask(2),
        per_lane in 1usize..4,
    ) {
        prop_assume!(per_lane <= cap);
        prop_assume!(mask != 0);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx0, rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let mut counts = [per_lane, per_lane];
        if mask & 1 == 0 {
            counts[0] = 0;
        }
        if mask & 2 == 0 {
            counts[1] = 0;
        }
        if counts[0] > 0 {
            tx0.testing_send_all(vec![1i32; counts[0]], false);
        }
        if counts[1] > 0 {
            tx1.testing_send_all(vec![2i32; counts[1]], false);
        }
        let shadow = graph.new_testing_test_monitor("shadow_idx");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let idx = core_exec::block_on(async {
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.try_lock().expect("rx0"));
            bundle.push(rx1_steady.try_lock().expect("rx1"));
            shadow.wait_avail_index(&mut bundle, &counts).await
        });
        if counts[0] > 0 && counts[1] == 0 {
            prop_assert_eq!(idx, Some(0));
        } else if counts[1] > 0 && counts[0] == 0 {
            prop_assert_eq!(idx, Some(1));
        } else {
            prop_assert!(idx.is_some());
        }
    }

    /// Property: `wait_vacant_index` picks a lane with sufficient vacancy.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_vacant_index_lane_mask(
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
        let shadow = graph.new_testing_test_monitor("shadow_vac_idx");
        let idx = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0_steady.lock().await);
            bundle.push(tx1_steady.lock().await);
            shadow.wait_vacant_index(&mut bundle, &counts).await
        });
        if mask & 1 != 0 && mask & 2 == 0 {
            prop_assert_eq!(idx, Some(0));
        } else if mask & 2 != 0 && mask & 1 == 0 {
            prop_assert_eq!(idx, Some(1));
        } else {
            prop_assert!(idx.is_some());
        }
    }

    /// Property: paired `wait_avail_vacant_index` returns the ready lane.
    #[test]
    // ss[verify actor.index-wait-paired]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_avail_vacant_index_paired(
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
        let shadow = graph.new_testing_test_monitor("shadow_paired");
        let in_rx0_steady = in_rx0.clone();
        let in_rx1_steady = in_rx1.clone();
        let out_tx0_steady = out_tx0.clone();
        let out_tx1_steady = out_tx1.clone();
        let idx = core_exec::block_on(async {
            let mut rx_bundle = RxBundle::new();
            rx_bundle.push(in_rx0_steady.try_lock().expect("in0"));
            rx_bundle.push(in_rx1_steady.try_lock().expect("in1"));
            let mut tx_bundle = TxBundle::new();
            tx_bundle.push(out_tx0_steady.try_lock().expect("out0"));
            tx_bundle.push(out_tx1_steady.try_lock().expect("out1"));
            shadow
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

    /// Property: deprecated bundle `wait_avail_bundle` succeeds when lanes are ready.
    #[test]
    // ss[verify bundle.deprecated-bundle-waits]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_avail_bundle(
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
        let shadow = graph.new_testing_test_monitor("shadow_bundle");
        let rx0_steady = rx0.clone();
        let rx1_steady = rx1.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = RxBundle::new();
            bundle.push(rx0_steady.try_lock().expect("rx0"));
            if lanes > 1 {
                bundle.push(rx1_steady.try_lock().expect("rx1"));
            }
            shadow.wait_avail_bundle(&mut bundle, need, lanes).await
        });
        prop_assert!(ok);
    }

    /// Property: shadow `try_send`/`try_take` preserves FIFO order.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_try_send_take_fifo(
        cap in capacity(),
        messages in prop::collection::vec(any::<i32>(), 1..16),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let mut shadow = graph.new_testing_test_monitor("shadow_fifo");
        let tx_steady = tx.clone();
        let rx_steady = rx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        for &m in &messages {
            prop_assert!(shadow.try_send(&mut tx_guard, m).is_sent());
        }
        drop(tx_guard);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let mut taken = Vec::new();
        while let Some(v) = shadow.try_take(&mut rx_guard) {
            taken.push(v);
        }
        prop_assert_eq!(taken, messages);
    }

    /// Property: shadow peek/take slice round-trip matches batch length.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_peek_poke_slice_roundtrip(
        cap in 4usize..32,
        batch in 1usize..8,
    ) {
        prop_assume!(batch <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<u64>();
        let mut shadow = graph.new_testing_test_monitor("shadow_slice");
        let tx_steady = tx.clone();
        let rx_steady = rx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let (poke_a, poke_b) = shadow.poke_slice(&mut tx_guard);
        let poke_len = poke_a.len() + poke_b.len();
        prop_assume!(batch <= poke_len);
        for i in 0..poke_a.len().min(batch) {
            poke_a[i].write(i as u64);
        }
        let rem = batch.saturating_sub(poke_a.len());
        for i in 0..rem.min(poke_b.len()) {
            poke_b[i].write((poke_a.len() + i) as u64);
        }
        let sent = shadow.advance_send_index(&mut tx_guard, batch).item_count();
        prop_assert_eq!(sent, batch);
        drop(tx_guard);
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let (peek_a, peek_b) = shadow.peek_slice(&mut rx_guard);
        let peek_len = peek_a.len() + peek_b.len();
        prop_assert!(peek_len >= batch);
        let taken = shadow.advance_take_index(&mut rx_guard, batch).item_count();
        prop_assert_eq!(taken, batch);
    }

    /// Property: `wait_periodic` returns false once shutdown is signaled.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_periodic_aborts_on_shutdown(
        delay_ms in 20u64..100,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("shadow_periodic");
        let ok = core_exec::block_on(shadow.wait_periodic(Duration::from_millis(delay_ms)));
        prop_assert!(!ok);
    }

    /// Property: `wait_timeout` returns false when shutdown races the timer.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_timeout_aborts_on_shutdown(
        delay_ms in 50u64..200,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("shadow_timeout");
        let ok = core_exec::block_on(shadow.wait_timeout(Duration::from_millis(delay_ms)));
        prop_assert!(!ok);
    }

    /// Property: `wait_shutdown` completes after graph shutdown request.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.request-shutdown]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_shutdown_after_request(
        _seed in 0..1u8,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("shadow_wait_shutdown");
        let done = core_exec::block_on(shadow.wait_shutdown());
        prop_assert!(done);
    }

    /// Property: shadow `relay_stats_smartly` always returns false (no telemetry relay).
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_relay_stats_smartly_false(
        _seed in 0..1u8,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let mut shadow = graph.new_testing_test_monitor("shadow_relay");
        prop_assert!(!shadow.relay_stats_smartly());
    }

    /// Property: internal_behavior shadow consumer honors `vote_matrix` vetoes.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.shutdown.veto]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_internal_behavior_vote_matrix(
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
            .with_name("SHADOW_CONSUMER")
            .build(
                move |ctx| {
                    let votes = votes_actor.clone();
                    let rx = rx_actor.clone();
                    async move { shadow_consumer_internal(ctx, rx, votes).await }
                },
                SoloAct,
            );
        graph.start();
        tx.testing_send_all(vec![1i32; msg_count], false);
        graph.request_shutdown();
        let stopped = graph.block_until_stopped(Duration::from_secs(3));
        prop_assert!(stopped.is_ok());
    }

    /// Property: `is_showstopper` triggers after repeated peeks without take.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_is_showstopper_threshold(
        threshold in 2usize..6,
        cap in 4usize..16,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<u8>();
        tx.testing_send_all(vec![42], false);
        let shadow = graph.new_testing_test_monitor("shadow_showstopper");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        for _ in 0..threshold + 1 {
            let _ = shadow.try_peek(&mut rx_guard);
        }
        prop_assert!(shadow.is_showstopper(&mut rx_guard, threshold));
    }

    /// Property: deprecated `wait_vacant_bundle` succeeds when lanes have vacancy.
    #[test]
    // ss[verify bundle.deprecated-bundle-waits]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_vacant_bundle(
        cap in 2usize..16,
        lanes in 1usize..3,
        need in 1usize..4,
    ) {
        prop_assume!(need <= cap);
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        let (tx0, _rx0) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let (tx1, _rx1) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("shadow_vac_bundle");
        let tx0_steady = tx0.clone();
        let tx1_steady = tx1.clone();
        let ok = core_exec::block_on(async {
            let mut bundle = TxBundle::new();
            bundle.push(tx0_steady.try_lock().expect("tx0"));
            if lanes > 1 {
                bundle.push(tx1_steady.try_lock().expect("tx1"));
            }
            shadow.wait_vacant_bundle(&mut bundle, need, lanes).await
        });
        prop_assert!(ok);
    }

    /// Property: `wait_periodic` returns true while the graph is still running.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_periodic_when_running(
        delay_ms in 5u64..30,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        let shadow = graph.new_testing_test_monitor("shadow_periodic_ok");
        let ok = core_exec::block_on(shadow.wait_periodic(Duration::from_millis(delay_ms)));
        prop_assert!(ok);
    }

    /// Property: `peek_async` observes the head message without consuming it.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_peek_async_preserves_message(
        cap in 2usize..16,
        value in any::<i32>(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        tx.testing_send_all(vec![value], false);
        let shadow = graph.new_testing_test_monitor("shadow_peek_async");
        let rx_steady = rx.clone();
        let peeked = core_exec::block_on(async {
            let mut rx_guard = rx_steady.lock().await;
            shadow.peek_async(&mut rx_guard).await.copied()
        });
        prop_assert_eq!(peeked, Some(value));
        prop_assert_eq!(rx.testing_take_all(), vec![value]);
    }

    /// Property: `send_async` with `AwaitForRoom` delivers when capacity exists.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_send_async_await_for_room(
        cap in capacity(),
        value in any::<i32>(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let mut shadow = graph.new_testing_test_monitor("shadow_send_async");
        let tx_steady = tx.clone();
        let outcome = core_exec::block_on(async {
            let mut tx_guard = tx_steady.lock().await;
            shadow
                .send_async(&mut tx_guard, value, SendSaturation::AwaitForRoom)
                .await
        });
        prop_assert!(matches!(outcome, SendOutcome::Success));
        prop_assert_eq!(rx.testing_take_all(), vec![value]);
    }

    /// Property: `call_async` returns `None` once shutdown is requested.
    #[test]
    // ss[verify actor.shadow-spotlight]
    // ss[verify graph.request-shutdown]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_call_async_aborts_on_shutdown(
        _seed in 0..1u8,
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        graph.request_shutdown();
        let shadow = graph.new_testing_test_monitor("shadow_call_async");
        let result = core_exec::block_on(shadow.call_async(async { 42i32 }));
        prop_assert!(result.is_none());
    }

    /// Property: `wait_empty` returns true on an empty transmitter.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_shadow_wait_empty_on_drained_tx(
        cap in capacity(),
    ) {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
        let shadow = graph.new_testing_test_monitor("shadow_wait_empty");
        let tx_steady = tx.clone();
        let empty = core_exec::block_on(async {
            let mut tx_guard = tx_steady.lock().await;
            shadow.wait_empty(&mut tx_guard).await
        });
        prop_assert!(empty);
    }
}
