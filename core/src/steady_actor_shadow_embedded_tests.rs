//! Tests extracted from `steady_actor_shadow.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related actor.shadow-spotlight]
// --- from mod tests ---
// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::*;
// ss[related actor.shadow-spotlight]
use std::time::Duration;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::ready;

#[test]
// ss[verify actor.shadow-spotlight]
fn test_wait_periodic() {
    let graph = GraphBuilder::for_testing().build(());
    let shadow = graph.new_testing_test_monitor("test");
    let result = core_exec::block_on(shadow.wait_periodic(Duration::from_millis(10)));
    assert!(result);
}

#[test]
// ss[verify actor.shadow-spotlight]
fn test_wait_timeout() {
    let graph = GraphBuilder::for_testing().build(());
    let shadow = graph.new_testing_test_monitor("test");
    let start = Instant::now();
    let result = core_exec::block_on(shadow.wait_timeout(Duration::from_millis(50)));
    assert!(result);
    assert!(start.elapsed() >= Duration::from_millis(50));
}

#[test]
// ss[verify actor.shadow-spotlight]
fn test_yield_now() {
    let graph = GraphBuilder::for_testing().build(());
    let shadow = graph.new_testing_test_monitor("test");
    core_exec::block_on(shadow.yield_now());
}

#[test]
// ss[verify actor.index-wait-truthful]
// ss[verify bundle.index-wait-readiness]
fn test_shadow_wait_avail_index_direct() {
crate::core_exec::block_on(async {

    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<i32>();
    if let Some(mut t) = tx.clone().try_lock() {
        let _ = t.shared_try_send(42);
    }
    let shadow = graph.new_testing_test_monitor("shadow_wait_idx");
    let rx_arc = rx.clone();
    let idx = {
        let mut rx_bundle = RxBundle::new();
        rx_bundle.push(rx_arc.try_lock().expect("rx"));
        shadow.wait_avail_index(&mut rx_bundle, &[1]).await
    };
    assert_eq!(idx, Some(0));
    });


    // ss[verify actor.index-wait-paired]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_shadow_wait_avail_vacant_index_direct() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(4).build_channel::<i32>();
        if let Some(mut t) = tx.clone().try_lock() {
            let _ = t.shared_try_send(7);
        }
        let shadow = graph.new_testing_test_monitor("paired");
        let tx_arc = tx.clone();
        let rx_arc = rx.clone();
        let idx = {
            let mut rx_bundle = RxBundle::new();
            let mut tx_bundle = TxBundle::new();
            rx_bundle.push(rx_arc.try_lock().expect("rx"));
            tx_bundle.push(tx_arc.try_lock().expect("tx"));
            shadow
                .wait_avail_vacant_index(&mut rx_bundle, &mut tx_bundle, &[1], &[1])
                .await
        };
        assert_eq!(idx, Some(0));
        });
}

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_relay_stats_smartly() {
        let graph = GraphBuilder::for_testing().build(());
        let mut shadow = graph.new_testing_test_monitor("test");
        // Shadow implementation always returns false
        assert!(!shadow.relay_stats_smartly());
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_relay_stats_periodic() {
        let graph = GraphBuilder::for_testing().build(());
        let mut shadow = graph.new_testing_test_monitor("test");
        let result = core_exec::block_on(shadow.relay_stats_periodic(Duration::from_millis(10)));
        assert!(result);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_loglevel() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Should not panic
        shadow.loglevel(LogLevel::Info);
    }

    #[test]
    #[ignore] //not everywhere, still need more research.
    // ss[verify actor.shadow-spotlight]
    fn test_aeron_media_driver_none() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert!(shadow.aeron_media_driver().is_none());
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_is_showstopper() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();

        // Send a message using the lazy transmitter (has `testing_send_all`)
        tx.testing_send_all(vec![42], false);

        let shadow = graph.new_testing_test_monitor("test");

        // Clone the lazy rx to get a steady (Arc<Mutex<Rx<u8>>>) which supports .lock()
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());

        // Initially peek_repeats is 0, so not a showstopper
        assert!(!shadow.is_showstopper(&mut rx_guard, 3));

        // Peek multiple times without taking to trigger showstopper
        for _ in 0..5 {
            shadow.try_peek(&mut rx_guard);
        }
        assert!(shadow.is_showstopper(&mut rx_guard, 3));
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_wait() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let start = Instant::now();
        core_exec::block_on(shadow.wait(Duration::from_millis(30)));
        assert!(start.elapsed() >= Duration::from_millis(30));
    }

    // ── Additional tests for increased coverage ──────────────────────────

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_call_async() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let result = core_exec::block_on(shadow.call_async(ready(42u32)));
        assert_eq!(result, Some(42));
    }


    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_wait_empty() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let shadow = graph.new_testing_test_monitor("test");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let result = core_exec::block_on(shadow.wait_empty(&mut tx_guard));
        assert!(result); // Empty channel returns true
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_wait_future_void() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let fut = ready(()).fuse();
        let result = core_exec::block_on(shadow.wait_future_void(fut));
        assert!(result);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_sidechannel_responder_none() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert!(shadow.sidechannel_responder().is_none());
    }

    // ss[verify bundle.index-wait-shutdown-none]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_request_shutdown() {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        let mut shadow = graph.new_testing_test_monitor("test");
        core_exec::block_on(shadow.request_shutdown());
        assert!(shadow.is_liveliness_stop_requested());
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_args() {
        let graph = GraphBuilder::for_testing().build(42i32);
        let shadow = graph.new_testing_test_monitor("test");
        // The graph was built with i32
        let value: Option<&i32> = shadow.args();
        assert_eq!(value, Some(&42));
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_identity() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test_monitor");
        let id = shadow.identity();
        assert_eq!(id.label.name, "test_monitor");
        assert_eq!(id.id, usize::MAX);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_frame_rate_ms() {
        let mut graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // The default telemetry production rate for testing is 0 ms (telemetry disabled)
        assert_eq!(shadow.frame_rate_ms(), 0);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_regeneration() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert_eq!(shadow.regeneration(), 0);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_liveliness_states() {
        let mut graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Initial state is Building until start
        assert!(shadow.is_liveliness_building());
        assert!(!shadow.is_liveliness_running());
        assert!(!shadow.is_liveliness_stop_requested());
        graph.start();
        assert!(!shadow.is_liveliness_building());
        assert!(shadow.is_liveliness_running());
        graph.request_shutdown();
        assert!(shadow.is_liveliness_stop_requested());
    }

    // ss[verify bundle.index-wait-shutdown-none]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_liveliness_shutdown_timeout() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Default has no shutdown_timeout
        assert!(shadow.is_liveliness_shutdown_timeout().is_none());
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_flush_defrag_empty() {
        // Create a shadow and call flush_defrag_messages with an empty defrag
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamEgress>(1);
        let mut shadow = graph.new_testing_test_monitor("test");

        let mut defrag = Defrag::<StreamEgress>::new(1, 10, 100);
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        // Workaround: borrow fields through separate deref to satisfy borrow checker
        let tx_inner = &mut *tx_guard;
        let (msgs, bytes, session) = shadow.flush_defrag_messages(
            &mut tx_inner.control_channel,
            &mut tx_inner.payload_channel,
            &mut defrag,
        );
        assert_eq!(msgs, 0);
        assert_eq!(bytes, 0);
        assert_eq!(session, None);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_try_send_and_try_take() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let send_result = shadow.try_send(&mut tx_guard, 42);
        assert!(send_result.is_sent());
        drop(tx_guard);

        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let take_result = shadow.try_take(&mut rx_guard);
        assert_eq!(take_result, Some(42));
    }

    // ss[verify actor.wait-avail-vacant]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    fn test_is_full_and_vacant_units() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(3).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        // Fresh channel should not be full
        assert!(!shadow.is_full(&mut tx_guard));
        assert_eq!(shadow.vacant_units(&mut tx_guard), 3);
        drop(tx_guard);

        // Fill the channel
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        for i in 0..3 {
            let _ = tx_guard.shared_try_send(i);
        }
        assert!(shadow.is_full(&mut tx_guard));
        assert_eq!(shadow.vacant_units(&mut tx_guard), 0);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_send_iter_until_full() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(4).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let iter = vec![1, 2, 3].into_iter();
        let count = shadow.send_iter_until_full(&mut tx_guard, iter);
        assert_eq!(count, 3);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_is_empty_and_avail_units() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        assert!(shadow.is_empty(&mut rx_guard));
        assert_eq!(shadow.avail_units(&mut rx_guard), 0);
        drop(rx_guard);

        // Send a message and check again
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let _ = tx_guard.shared_try_send(99);
        drop(tx_guard);

        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        assert!(!shadow.is_empty(&mut rx_guard));
        assert_eq!(shadow.avail_units(&mut rx_guard), 1);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_call_blocking() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let fut = shadow.call_blocking(|| 42u32);
        let result = core_exec::block_on(fut);
        assert_eq!(result, 42);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_take_async_with_timeout() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![10, 20], false);
        let mut shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let taken = core_exec::block_on(shadow.take_async_with_timeout(&mut rx_guard, Duration::from_millis(100)));
        assert_eq!(taken, Some(10));
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_take_into_iter() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![1, 2, 3], false);
        let mut shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let items: Vec<u8> = shadow.take_into_iter(&mut rx_guard).collect();
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    // ss[verify actor.shadow-spotlight]
    fn test_peek_async() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![77], false);
        let shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let peeked = core_exec::block_on(shadow.peek_async(&mut rx_guard));
        assert!(peeked.is_some());
    }
}

#[path = "steady_actor_shadow/tests/mod.rs"]
// ss[related actor.shadow-spotlight]
mod shadow_proptest_suite;
