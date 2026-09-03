//! Tests extracted from `steady_actor.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related actor.run-dispatcher]
// --- from mod steady_actor_tests ---
// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::core_exec;

#[test]
// ss[verify actor.run-dispatcher]
fn test_send_outcome_methods() {
    let success: SendOutcome<i32> = SendOutcome::Success;
    assert!(success.is_sent());
    assert!(success.expect("should not panic"));

    let blocked = SendOutcome::Blocked(42);
    assert!(!blocked.is_sent());

    let timeout = SendOutcome::Timeout(42);
    assert!(!timeout.is_sent());

    let closed = SendOutcome::Closed(42);
    assert!(!closed.is_sent());
}

#[test]
#[should_panic(expected = "test panic")]
// ss[verify actor.run-dispatcher]
fn test_send_outcome_expect_panic() {
    let blocked = SendOutcome::Blocked(42);
    blocked.expect("test panic");
}

#[test]
// ss[verify actor.run-dispatcher]
fn test_blocking_call_future_terminated() {
    let (tx, rx) = crate::oneshot::channel::<i32>();
    let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
    assert!(!fut.is_terminated());
    tx.send(42).expect("send");
    let res = core_exec::block_on(&mut fut);
    assert_eq!(res, 42);
    assert!(fut.is_terminated());
}

#[test]
// ss[verify actor.run-dispatcher]
fn test_blocking_call_future_fetch_timeout() {
    let (_tx, rx) = crate::oneshot::channel::<i32>();
    let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
    let res = core_exec::block_on(fut.fetch(Duration::from_millis(10)));
    assert!(res.is_none());
}

#[test]
// ss[verify actor.run-dispatcher]
fn test_blocking_call_future_fetch_ready() {
    let (tx, rx) = crate::oneshot::channel::<i32>();
    let mut fut = BlockingCallFuture(Box::pin(async move { rx.await.expect("") }.fuse()));
    tx.send(42).expect("send");
    let res = core_exec::block_on(fut.fetch(Duration::from_millis(100)));
    assert_eq!(res, Some(42));
}

// ss[related actor.run-dispatcher]
mod index_wait_pure {
    // ss[related philosophy.structural-hierarchy]
    use super::super::{
        index_wait_avoid_repeat_lane, index_wait_counts_uniform_usize, next_index_wait_start,
    };

    // ss[verify actor.index-wait-round-robin]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn next_index_wait_start_empty_len() {
        assert_eq!(next_index_wait_start(0, 0), 0);
        assert_eq!(next_index_wait_start(5, 0), 0);
    }

    // ss[verify actor.index-wait-round-robin]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn next_index_wait_start_wrap_and_invalid_last() {
        assert_eq!(next_index_wait_start(usize::MAX, 3), 0);
        assert_eq!(next_index_wait_start(3, 3), 0);
        assert_eq!(next_index_wait_start(0, 4), 1);
        assert_eq!(next_index_wait_start(3, 4), 0);
    }

    // ss[related actor.run-dispatcher]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: next scan start always lands in [0, len) when len > 0.
        #[test]
        // ss[verify actor.index-wait-round-robin]
        // ss[verify verify.process.proptest]
        fn proptest_next_index_wait_start_in_range(
            last_stored in any::<usize>(),
            len in 1usize..64,
        ) {
            let start = next_index_wait_start(last_stored, len);
            prop_assert!(start < len);
        }

        /// Property: uniform count helper fills every lane with the same threshold.
        #[test]
        // ss[verify bundle.uniform-counts-helper]
        // ss[verify verify.process.proptest]
        fn proptest_index_wait_counts_uniform(
            per_lane in 0usize..128,
            len in 0usize..32,
        ) {
            let counts = index_wait_counts_uniform_usize(per_lane, len);
            prop_assert_eq!(counts.len(), len);
            for &c in &counts {
                prop_assert_eq!(c, per_lane);
            }
        }

        /// Property: repeat bypass never returns a different lane when len <= 1.
        #[test]
        // ss[verify actor.index-wait-repeat-bypass]
        // ss[verify verify.process.proptest]
        fn proptest_avoid_repeat_lane_singleton(
            candidate in 0usize..8,
            last_stored in 0usize..8,
        ) {
            let result = index_wait_avoid_repeat_lane(1, 0, last_stored, candidate, |_| true);
            prop_assert_eq!(result, candidate);
        }

        /// Property: when len > 1 and candidate repeats last_stored, prefer another ready lane.
        #[test]
        // ss[verify actor.index-wait-repeat-bypass]
        // ss[verify verify.process.proptest]
        fn proptest_avoid_repeat_lane_prefers_alternate(
            len in 2usize..16,
            last_stored in 0usize..16,
            ready_mask in any::<u16>(),
        ) {
            let last = if last_stored >= len { usize::MAX } else { last_stored };
            let candidate = if last < len { last } else { 0 };
            let is_ready = |j: usize| j < len && (ready_mask & (1u16 << j)) != 0;
            let start = next_index_wait_start(last, len);
            let result = index_wait_avoid_repeat_lane(len, start, last, candidate, is_ready);
            prop_assert!(result < len);
            if len > 1 && last < len && candidate == last {
                for j in 0..len {
                    if j != candidate && is_ready(j) {
                        prop_assert_ne!(result, candidate);
                        return Ok(());
                    }
                }
            }
        }
    }
}

// ss[related actor.wait-avail-vacant]
mod wait_helpers_proptest {
    // ss[related philosophy.structural-hierarchy]
    use super::{
        wait_paired_lane_ready, wait_rx_until_avail_items_ready,
        wait_tx_until_vacant_satisfied,
    };
    // ss[related actor.run-dispatcher]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::proptest_support::{capacity, vote_matrix};
    // ss[related philosophy.structural-hierarchy]
    use crate::{GraphBuilder, SteadyActor};
    // ss[related actor.run-dispatcher]
    use crate::steady_actor_shadow::SteadyActorShadow;
    // ss[related philosophy.structural-hierarchy]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: `wait_rx_until_avail_items_ready` succeeds when items are already queued.
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify verify.process.proptest]
        fn proptest_wait_rx_until_avail_items_ready_immediate(
            cap in capacity(),
            need in 1usize..8,
        ) {
            prop_assume!(need <= cap);
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            tx.testing_send_all(vec![1i32; need], false);
            let rx_steady = rx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = rx_steady.lock().await;
                wait_rx_until_avail_items_ready(&mut rx_guard, need).await
            });
            prop_assert!(ready);
        }

        /// Property: zero required avail is always ready without waiting.
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify verify.process.proptest]
        fn proptest_wait_rx_until_avail_zero_required(
            cap in capacity(),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let rx_steady = rx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = rx_steady.lock().await;
                wait_rx_until_avail_items_ready(&mut rx_guard, 0).await
            });
            prop_assert!(ready);
        }

        /// Property: `wait_tx_until_vacant_satisfied` succeeds when channel has room.
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify verify.process.proptest]
        fn proptest_wait_tx_until_vacant_satisfied_immediate(
            cap in capacity(),
            need in 1usize..8,
        ) {
            prop_assume!(need <= cap);
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let tx_steady = tx.clone();
            let ready = core_exec::block_on(async {
                let mut tx_guard = tx_steady.lock().await;
                wait_tx_until_vacant_satisfied(&mut tx_guard, need).await
            });
            prop_assert!(ready);
        }

        /// Property: `wait_paired_lane_ready` succeeds when rx has data and tx has room.
        #[test]
        // ss[verify actor.index-wait-paired]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify verify.process.proptest]
        fn proptest_wait_paired_lane_ready_immediate(
            cap in 2usize..16,
            need in 1usize..4,
        ) {
            prop_assume!(need <= cap);
            let mut graph = GraphBuilder::for_testing().build(());
            let (in_tx, in_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let (out_tx, _out_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            in_tx.testing_send_all(vec![1i32; need], false);
            let in_rx_steady = in_rx.clone();
            let out_tx_steady = out_tx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = in_rx_steady.lock().await;
                let mut tx_guard = out_tx_steady.lock().await;
                wait_paired_lane_ready(&mut rx_guard, &mut tx_guard, need, need).await
            });
            prop_assert!(ready);
        }

        /// Property: paired wait with zero required avail still requires tx vacancy.
        #[test]
        // ss[verify actor.index-wait-paired]
        // ss[verify verify.process.proptest]
        fn proptest_wait_paired_lane_ready_zero_avail(
            cap in 2usize..16,
            need in 1usize..4,
        ) {
            prop_assume!(need <= cap);
            let mut graph = GraphBuilder::for_testing().build(());
            let (in_tx, in_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let (out_tx, _out_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            in_tx.testing_send_all(vec![1], false);
            let out_tx_steady = out_tx.clone();
            if let Some(mut g) = out_tx_steady.try_lock() {
                for _ in 0..cap.saturating_sub(need) {
                    let _ = g.shared_try_send(1i32);
                }
            }
            let in_rx_steady = in_rx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = in_rx_steady.lock().await;
                let mut tx_guard = out_tx_steady.lock().await;
                wait_paired_lane_ready(&mut rx_guard, &mut tx_guard, 0, need).await
            });
            prop_assert!(ready);
        }

        /// Property: shadow `wait_avail` aborts on shutdown (exercises shutdown wait path).
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify graph.request-shutdown]
        // ss[verify verify.process.proptest]
        fn proptest_shadow_wait_avail_shutdown_path(
            cap in capacity(),
            _votes in vote_matrix(1),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            graph.start();
            graph.request_shutdown();
            let shadow = graph.new_testing_test_monitor("wait_shutdown_path");
            let rx_steady = rx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = rx_steady.lock().await;
                shadow.wait_avail(&mut rx_guard, 1).await
            });
            prop_assert!(!ready);
        }

        /// Property: shadow `wait_vacant` aborts on shutdown (vacant shutdown wait path).
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify graph.request-shutdown]
        // ss[verify verify.process.proptest]
        fn proptest_shadow_wait_vacant_shutdown_path(
            cap in capacity(),
            _votes in vote_matrix(1),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let tx_steady = tx.clone();
            if let Some(mut g) = tx_steady.try_lock() {
                for _ in 0..cap {
                    let _ = g.shared_try_send(1i32);
                }
            }
            graph.start();
            graph.request_shutdown();
            let shadow = graph.new_testing_test_monitor("vacant_shutdown_path");
            let ready = core_exec::block_on(async {
                let mut tx_guard = tx_steady.lock().await;
                shadow.wait_vacant(&mut tx_guard, 1).await
            });
            prop_assert!(!ready);
        }

        /// Property: `wait_rx_until_avail_items_ready` returns false on empty channel at shutdown.
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify graph.request-shutdown]
        // ss[verify verify.process.proptest]
        fn proptest_wait_rx_until_avail_shutdown_empty(
            cap in capacity(),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            graph.start();
            let tx_steady = tx.clone();
            core_exec::block_on(async {
                let mut g = tx_steady.lock().await;
                g.mark_closed();
            });
            let rx_steady = rx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = rx_steady.lock().await;
                wait_rx_until_avail_items_ready(&mut rx_guard, 1).await
            });
            prop_assert!(!ready);
        }

        /// Property: `wait_tx_until_vacant_satisfied` fails when channel is full at shutdown.
        #[test]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify graph.request-shutdown]
        // ss[verify verify.process.proptest]
        fn proptest_wait_tx_until_vacant_shutdown_full(
            cap in 2usize..16,
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let tx_steady = tx.clone();
            if let Some(mut g) = tx_steady.try_lock() {
                for _ in 0..cap {
                    let _ = g.shared_try_send(1i32);
                }
            }
            graph.start();
            graph.request_shutdown();
            let ready = core_exec::block_on(async {
                let mut tx_guard = tx_steady.lock().await;
                wait_tx_until_vacant_satisfied(&mut tx_guard, 1).await
            });
            prop_assert!(!ready);
        }

        /// Property: `wait_paired_lane_ready` aborts when shutdown races an unmet rx wait.
        #[test]
        // ss[verify actor.index-wait-paired]
        // ss[verify graph.request-shutdown]
        // ss[verify verify.process.proptest]
        fn proptest_wait_paired_lane_ready_shutdown_unmet_rx(
            cap in 2usize..16,
            _votes in vote_matrix(1),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (in_tx, in_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            let (out_tx, _out_rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            graph.start();
            let in_tx_steady = in_tx.clone();
            core_exec::block_on(async {
                let mut g = in_tx_steady.lock().await;
                g.mark_closed();
            });
            let in_rx_steady = in_rx.clone();
            let out_tx_steady = out_tx.clone();
            let ready = core_exec::block_on(async {
                let mut rx_guard = in_rx_steady.lock().await;
                let mut tx_guard = out_tx_steady.lock().await;
                wait_paired_lane_ready(&mut rx_guard, &mut tx_guard, 1, 1).await
            });
            prop_assert!(!ready);
        }

        /// Property: spotlight `wait_avail` aborts on shutdown (exercises spotlight wait path).
        #[test]
        // ss[verify actor.shadow-spotlight]
        // ss[verify actor.wait-avail-vacant]
        // ss[verify verify.process.proptest]
        fn proptest_spotlight_wait_avail_shutdown_path(
            cap in capacity(),
        ) {
            let mut graph = GraphBuilder::for_testing().build(());
            let (_tx, rx) = graph.channel_builder().with_capacity(cap).build_channel::<i32>();
            graph.start();
            graph.request_shutdown();
            let shadow = graph.new_testing_test_monitor("spot_wait_shutdown_path");
            let rx_steady = rx.clone();
            let ready = core_exec::block_on(async {
                let spotlight = shadow.into_spotlight([&rx_steady], []);
                let mut rx_guard = rx_steady.lock().await;
                spotlight.wait_avail(&mut rx_guard, 1).await
            });
            prop_assert!(!ready);
        }
    }
}

