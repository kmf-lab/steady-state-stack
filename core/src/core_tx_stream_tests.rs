//! Tests extracted from `core_tx_stream.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related channel.stream-dual-buffer]
// --- from mod core_tx_stream_tests ---
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use crate::{GraphBuilder, ScheduleAs, SteadyActor, StreamEgress, StreamIngress, SendSaturation, TxCore, TxDone, ActorIdentity, core_exec, SendOutcome, StreamTx, steady_tx::TxMetaDataProvider};
// ss[related channel.stream-dual-buffer]
use crate::distributed::aqueduct_stream::Defrag;
// ss[related philosophy.structural-hierarchy]
use async_ringbuf::traits::Producer;

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_general() -> Result<(),Box<dyn std::error::Error>> {
    let mut graph = GraphBuilder::for_testing().build(());

    let bytes_per_item = 128;
    let mut channel_builder = graph.channel_builder();
    channel_builder = channel_builder.with_capacity(100);
    channel_builder = channel_builder.with_type();
    let (_tx, _rx) = channel_builder.build_stream::<StreamEgress>(bytes_per_item);

    graph.actor_builder().with_name("unit_test").build(
        move |mut actor| {
            Box::pin(async move {
                while actor.is_running(|| true) {
                    actor.wait_periodic(Duration::from_millis(1)).await;
                }
                Ok::<(), Box<dyn std::error::Error>>(())
            })
        },
        ScheduleAs::SoloAct,
    );

    graph.start();
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(5))?;
    Ok(())
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core() -> Result<(), Box<dyn std::error::Error>> {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(10)
            .build_stream::<StreamIngress>(100);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let ident = ActorIdentity::default();

        // Test shared_capacity
        let cap = tx_guard.shared_capacity();
        assert!(cap.0 >= 10);
        assert!(cap.1 >= 1000);

        let now = std::time::Instant::now();
        // Test shared_try_send
        let msg = (StreamIngress::new(5, 0, now, now), &[1, 2, 3, 4, 5][..]);
        let result = tx_guard.shared_try_send(msg);
        assert!(matches!(result, Ok(TxDone::Stream(1, 5))));

        // Test shared_send_slice
        let items = [StreamIngress::new(3, 0, now, now), StreamIngress::new(2, 0, now, now)];
        let payload = [1, 1, 1, 2, 2];
        let done = tx_guard.shared_send_slice((&items, &payload));
        assert!(matches!(done, TxDone::Stream(2, 5)));

        // Test shared_send_async
        let msg_async = (StreamIngress::new(4, 0, now, now), &[9, 9, 9, 9][..]);
        let outcome = tx_guard.shared_send_async(msg_async, ident, SendSaturation::AwaitForRoom).await;
        assert!(matches!(outcome, crate::SendOutcome::Success));

        // Test shared_mark_closed
        tx_guard.shared_mark_closed();
        
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_egress_tx_core() -> Result<(), Box<dyn std::error::Error>> {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(10)
            .build_stream::<StreamEgress>(100);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let ident = ActorIdentity::default();

        // Test shared_try_send
        let payload = &[1, 2, 3, 4][..];
        let result = tx_guard.shared_try_send(payload);
        assert!(matches!(result, Ok(TxDone::Stream(1, 4))));

        // Test shared_send_slice
        let items = [StreamEgress { length: 2 }, StreamEgress { length: 3 }];
        let payload_slice = [7, 7, 8, 8, 8];
        let done = tx_guard.shared_send_slice((&items, &payload_slice));
        assert!(matches!(done, TxDone::Stream(2, 5)));

        // Test shared_send_async
        let payload_async = &[5, 5, 5][..];
        let outcome = tx_guard.shared_send_async(payload_async, ident, SendSaturation::AwaitForRoom).await;
        assert!(matches!(outcome, crate::SendOutcome::Success));

        // Test shared_advance_index
        let done_adv = tx_guard.shared_advance_index((0, 0));
        assert!(matches!(done_adv, TxDone::Stream(0, 0)));

        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_tx_iter_until_full() -> Result<(), Box<dyn std::error::Error>> {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(10)
            .build_stream::<StreamEgress>(100);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        
        let payloads = vec![&[1, 2][..], &[3, 4, 5][..]];
        let count = tx_guard.shared_send_iter_until_full(payloads.into_iter());
        assert_eq!(count, 2);
        
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_defrag_partial_flush() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, _rx) = graph.channel_builder()
        .with_capacity(2) // Small capacity to force partial flush
        .build_stream::<StreamEgress>(10);

    let tx_arc = tx.clone();
    let mut tx_guard = core_exec::block_on(tx_arc.lock());
    let mut defrag = Defrag::<StreamEgress>::new(1, 10, 100);
    
    // Fill defrag with more than the channel can take (3 items)
    for _ in 0..3 {
        defrag.ringbuffer_items.0.try_push(StreamEgress::new(5)).unwrap();
        defrag.ringbuffer_bytes.0.push_slice(&[0u8; 5]);
    }

    let mut actor = graph.new_testing_test_monitor("test");
    let StreamTx { ref mut control_channel, ref mut payload_channel, .. } = *tx_guard;
    let (msgs, bytes, session) = actor.flush_defrag_messages(
        control_channel,
        payload_channel,
        &mut defrag
    );

    // Should have flushed 2 messages (capacity limit)
    assert_eq!(msgs, 2);
    assert_eq!(bytes, 10);
    // Should indicate session 1 still needs work
    assert_eq!(session, Some(1));
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_send_async_timeout_saturation() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, _rx) = graph.channel_builder()
        .with_capacity(1)
        .build_stream::<StreamEgress>(10);
    
    let tx_arc = tx.clone();
    let mut tx_guard = core_exec::block_on(tx_arc.lock());
    let ident = ActorIdentity::default();

    // Fill the channel
    tx_guard.shared_try_send(&[0u8; 5]).unwrap();

    // Try to send again with a tiny timeout
    let start = std::time::Instant::now();
    #[allow(deprecated)]
    let outcome = core_exec::block_on(tx_guard.shared_send_async_timeout(
        &[0u8; 5],
        ident,
        SendSaturation::ReturnBlockedMsg,
        Some(Duration::from_millis(10))
    ));

    // Should return Blocked immediately due to saturation policy
    assert!(matches!(outcome, SendOutcome::Blocked(_)));
    
    // Try again with AwaitForRoom and a timeout
    let outcome_timeout = core_exec::block_on(tx_guard.shared_send_async_timeout(
        &[0u8; 5],
        ident,
        SendSaturation::AwaitForRoom,
        Some(Duration::from_millis(10))
    ));
    assert!(matches!(outcome_timeout, SendOutcome::Timeout(_)));
    assert!(start.elapsed() >= Duration::from_millis(10));
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_saturation_policies() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let ident = ActorIdentity::default();
        let now = std::time::Instant::now();
        let msg = (StreamIngress::new(5, 0, now, now), &[0u8; 5][..]);

        // Fill channel
        tx_guard.shared_try_send(msg).unwrap();

        // Test WarnThenAwait
        let fut = tx_guard.shared_send_async_timeout(msg, ident, SendSaturation::WarnThenAwait, Some(Duration::from_millis(10)));
        assert!(matches!(fut.await, SendOutcome::Timeout(_)));

        // Test DebugWarnThenAwait
        let fut = tx_guard.shared_send_async_timeout(msg, ident, SendSaturation::DebugWarnThenAwait, Some(Duration::from_millis(10)));
        assert!(matches!(fut.await, SendOutcome::Timeout(_)));
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_partial_slice_send() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10); // Payload capacity is small
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let now = std::time::Instant::now();

        // Items that fit in control but payload is too large for the small buffer
        let items = [StreamIngress::new(8, 0, now, now), StreamIngress::new(8, 0, now, now)];
        let payload = [0u8; 16];
        
        let done = tx_guard.shared_send_slice((&items, &payload));
        // With capacity 5 and multiplier 10, we have 50 bytes. Both fit.
        assert_eq!(done.item_count(), 2);
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_advance_fail() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;

        // Try to advance more than capacity
        let done = tx_guard.shared_advance_index((10, 100));
        assert_eq!(done, TxDone::Stream(0, 0));
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_telemetry_warning() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        
        let meta = tx_guard.control_channel.channel_meta_data.meta_data.clone();
        let mut actor = graph.new_testing_test_monitor("test")
            .into_spotlight([], [&meta as &dyn TxMetaDataProvider]);

        // Force the warning branch by passing Normal to an Ingress stream
        if let Some(ref mut tel) = actor.telemetry.send_tx {
            tx_guard.telemetry_inc(TxDone::Normal(1), tel);
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_periodic_log() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;

        // First call is true because constructor backdates the timer for immediate logging
        assert!(tx_guard.log_perodic());
        
        // Manually backdate the timer again to test subsequent trigger
        tx_guard.control_channel.last_error_send = Instant::now() - Duration::from_secs(30);
        assert!(tx_guard.log_perodic());
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_wait_shutdown() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;

        // Fill the channel to force the async wait path
        let now = Instant::now();
        tx_guard.shared_try_send((StreamIngress::new(5, 0, now, now), &[0u8; 5][..])).unwrap();

        // Trigger shutdown signal
        let (shutdown_tx, _) = futures::channel::oneshot::channel::<()>();
        tx_guard.control_channel.oneshot_shutdown = futures::channel::oneshot::channel::<()>().1;
        drop(shutdown_tx); // Close the channel to trigger is_terminated

        let result = tx_guard.shared_wait_shutdown_or_vacant_units((1, 1)).await;
        assert!(!result);
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_egress_tx_core_capacity_checks() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(10)
            .build_stream::<StreamEgress>(100);
        
        let tx_clone = tx.clone();
        let tx_guard = tx_clone.lock().await;

        assert!(tx_guard.shared_capacity_for((5, 50)));
        assert!(!tx_guard.shared_capacity_for((100, 1000)));
        
        assert!(tx_guard.shared_vacant_units_for((5, 50)));
        assert!(!tx_guard.shared_vacant_units_for((100, 1000)));
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_done_one_one_log_and_advance() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph.channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, _eg_rx) = graph.channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(16);

        let in_clone = in_tx.clone();
        let mut ingress = in_clone.lock().await;
        let now = Instant::now();
        let payload = &[1u8, 2, 3][..];
        let item = StreamIngress::new(3, 0, now, now);
        assert_eq!(ingress.done_one(&(item, payload)), TxDone::Stream(1, 3));
        let one = ingress.one();
        assert_eq!(one.0, 1);
        assert!(one.1 >= 1);
        assert!(ingress.shared_capacity_for((1, 1)));
        assert!(!ingress.shared_capacity_for((100, 1000)));
        assert_eq!(ingress.shared_advance_index((100, 1000)), TxDone::Stream(0, 0));
        assert_eq!(ingress.shared_advance_index((1, 2)), TxDone::Stream(1, 2));
        ingress.control_channel.last_error_send = Instant::now();
        assert!(!ingress.log_perodic());
        ingress.monitor_not();

        let eg_clone = eg_tx.clone();
        let mut egress = eg_clone.lock().await;
        assert_eq!(egress.done_one(&payload), TxDone::Stream(1, 3));
        let one = egress.one();
        assert_eq!(one.0, 1);
        assert!(one.1 >= 1);
        assert_eq!(egress.shared_advance_index((100, 1000)), TxDone::Stream(0, 0));
        assert_eq!(egress.shared_advance_index((1, 2)), TxDone::Stream(1, 2));
        egress.control_channel.last_error_send = Instant::now();
        assert!(!egress.log_perodic());
        egress.control_channel.last_error_send =
            Instant::now() - Duration::from_secs(30);
        assert!(egress.log_perodic());
        egress.monitor_not();

        let meta = egress.control_channel.channel_meta_data.meta_data.clone();
        let mut actor = graph.new_testing_test_monitor("stream_tel")
            .into_spotlight([], [&meta as &dyn TxMetaDataProvider]);
        if let Some(ref mut tel) = actor.telemetry.send_tx {
            egress.telemetry_inc(TxDone::Stream(1, 2), tel);
            egress.telemetry_inc(TxDone::Normal(1), tel);
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_mark_closed_dropped() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        
        // Drop the receivers to trigger the trace branches
        drop(rx);
        
        tx_guard.shared_mark_closed();
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
fn test_stream_ingress_tx_core_wait_empty_terminated() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamIngress>(10);
        
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;

        // Trigger shutdown signal
        let (shutdown_tx, _) = futures::channel::oneshot::channel::<()>();
        tx_guard.control_channel.oneshot_shutdown = futures::channel::oneshot::channel::<()>().1;
        drop(shutdown_tx); 

        let result = tx_guard.shared_wait_empty().await;
        assert!(result); // Returns true because it is empty
    });
}

// ss[related channel.stream-dual-buffer]
use proptest::prelude::*;

ss_proptest! {

    /// Property: stream egress vacant control slots plus sent never exceed capacity.
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_stream_egress_vacant_plus_sent_le_capacity(
        cap in 2usize..32,
        payload_len in 1usize..16,
        send_count in 1usize..8,
    ) {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamEgress>(100);
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            let (ctrl_cap, _payload_cap) = tx_guard.shared_capacity();
            let mut sent = 0usize;
            for _ in 0..send_count.min(ctrl_cap) {
                let payload = vec![0u8; payload_len];
                if tx_guard.shared_try_send(payload.as_slice()).is_ok() {
                    sent += 1;
                } else {
                    break;
                }
            }
            let (vacant_ctrl, _) = tx_guard.shared_vacant_units();
            prop_assert!(vacant_ctrl + sent <= ctrl_cap);
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }

    /// Property: stream egress send_slice never exceeds vacant control slots.
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_stream_egress_send_slice_within_vacant(
        cap in 2usize..16,
        payload_len in 1usize..8,
        extra in 1usize..4,
    ) {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamEgress>(100);
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            let (vacant_ctrl, _) = tx_guard.shared_vacant_units();
            let item_count = vacant_ctrl + extra;
            let items: Vec<StreamEgress> =
                (0..item_count).map(|_| StreamEgress::new(payload_len as i32)).collect();
            let payload = vec![0u8; item_count * payload_len];
            let done = tx_guard.shared_send_slice((items.as_slice(), payload.as_slice()));
            prop_assert!(done.item_count() <= vacant_ctrl);
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }

    /// Property: stream ingress send_slice respects control and payload vacancy.
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_send_slice_within_vacant(
        cap in 2usize..16,
        item_bytes in 1usize..8,
        extra in 1usize..4,
    ) {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamIngress>(item_bytes * cap);
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            let (vacant_ctrl, vacant_payload) = tx_guard.shared_vacant_units();
            let item_count = vacant_ctrl + extra;
            let now = Instant::now();
            let items: Vec<StreamIngress> = (0..item_count)
                .map(|_| StreamIngress::new(item_bytes as i32, 0, now, now))
                .collect();
            let payload = vec![0u8; item_count * item_bytes];
            let done = tx_guard.shared_send_slice((items.as_slice(), payload.as_slice()));
            prop_assert!(done.item_count() <= vacant_ctrl);
            if let Some(bytes_sent) = done.payload_count() {
                prop_assert!(bytes_sent <= vacant_payload);
            }
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }

    /// Property: stream ingress sent payload bytes equal received bytes.
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_stream_ingress_no_silent_drop(
        cap in 2usize..12,
        item_bytes in 1usize..6,
        send_count in 1usize..4,
    ) {
        core_exec::block_on(async {
            // ss[related channel.stream-dual-buffer]
            use crate::RxCore;
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamIngress>(cap * item_bytes * 4);
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            let now = Instant::now();
            let mut sent_bytes = 0usize;
            for _ in 0..send_count.min(cap) {
                let payload = vec![0u8; item_bytes];
                let msg = (
                    StreamIngress::new(item_bytes as i32, 0, now, now),
                    payload.as_slice(),
                );
                if tx_guard.shared_try_send(msg).is_ok() {
                    sent_bytes += item_bytes;
                } else {
                    break;
                }
            }
            drop(tx_guard);
            let rx_clone = rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            let mut taken_bytes = 0usize;
            while let Some((done, (_item, payload))) = rx_guard.shared_try_take() {
                let payload: &Box<[u8]> = &payload;
                if let crate::RxDone::Stream(_, b) = done {
                    prop_assert_eq!(b, payload.len());
                    taken_bytes += b;
                }
            }
            prop_assert_eq!(taken_bytes, sent_bytes);
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }

    /// Property: stream egress mark_closed is idempotent and preserves vacant count.
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_stream_egress_mark_closed_idempotent(
        cap in crate::proptest_support::capacity(),
        payload_len in 1usize..8,
    ) {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (tx, _rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamEgress>(cap * 16);
            let tx_clone = tx.clone();
            let mut tx_guard = tx_clone.lock().await;
            let payload = vec![0u8; payload_len];
            let _ = tx_guard.shared_try_send(payload.as_slice());
            let (vacant_ctrl, vacant_payload) = tx_guard.shared_vacant_units();
            tx_guard.shared_mark_closed();
            tx_guard.shared_mark_closed();
            let (after_ctrl, after_payload) = tx_guard.shared_vacant_units();
            prop_assert_eq!((after_ctrl, after_payload), (vacant_ctrl, vacant_payload));
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }
}

// #[test]
// fn test_stream_ingress_tx_core_send_iter_payload_full() {
//     core_exec::block_on(async {
//         let mut graph = GraphBuilder::for_testing().build(());
//         let (tx, rx) = graph.channel_builder()
//             .with_capacity(10)
//             .build_stream::<StreamIngress>(1); // 10 items, 10 bytes total
//
//         let tx_clone = tx.clone();
//         let rx_clone = rx.clone();
//
//         core_exec::spawn_detached(async move {
//             Delay::new(Duration::from_millis(100)).await;
//             let mut rx_guard = rx_clone.lock().await;
//             rx_guard.shared_advance_index((0, 10));
//         });
//
//         let mut tx_guard = tx_clone.lock().await;
//         let now = Instant::now();
//
//         // Fill payload partially
//         tx_guard.payload_channel.tx.push_slice(&[0u8; 8]);
//
//         // Item needs 5 bytes. Only 2 left.
//         let items = vec![(StreamIngress::new(5, 0, now, now), &[0u8; 5][..])];
//         let count = tx_guard.shared_send_iter_until_full(items.into_iter());
//         assert_eq!(count, 1);
//     });
// }

