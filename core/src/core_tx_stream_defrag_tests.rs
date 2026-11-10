// ss[related channel.stream-dual-buffer]
// Defrag, saturation, close, and property tests split out of core_tx_stream_tests.rs.

// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use crate::{
    ActorIdentity, GraphBuilder, MONITOR_NOT, ScheduleAs, SendOutcome, SendSaturation, SteadyActor,
    StreamEgress, StreamIngress, StreamTx, TxCore, TxDone, core_exec, steady_config,
    steady_tx::TxMetaDataProvider, RxCore,
};
use crate::channel_builder::ChannelBuilder;
use crate::distributed::aqueduct_stream::Defrag;
use crate::logging_util::steady_logger::start_log_capture;
use crate::logging_util::TEST_CONTEXTS;
use crate::monitor_telemetry::SteadyTelemetrySend;
use async_ringbuf::traits::Producer;

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
        assert_eq!(
            tx_guard.shared_advance_index((1, 10_000)),
            TxDone::Stream(0, 0),
            "control vacant without payload vacant must not advance"
        );
        assert_eq!(
            tx_guard.shared_advance_index((10_000, 1)),
            TxDone::Stream(0, 0)
        );
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

fn dummy_stream_tel() -> SteadyTelemetrySend<4> {
    let (dummy_tx, _dummy_rx) = ChannelBuilder::default()
        .with_capacity(1)
        .eager_build::<[usize; 4]>();
    SteadyTelemetrySend::new(dummy_tx, [0; 4], [0; 4], Instant::now())
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_telemetry_inc_updates_control_and_payload_counts() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(8)
            .build_stream::<StreamIngress>(32);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(8)
            .build_stream::<StreamEgress>(32);

        let in_clone = in_tx.clone();
        let mut ingress = in_clone.lock().await;
        let mut tel = dummy_stream_tel();
        ingress.control_channel.local_monitor_index = 0;
        ingress.payload_channel.local_monitor_index = 1;
        ingress.telemetry_inc(TxDone::Stream(2, 9), &mut tel);
        assert_eq!(tel.count[0], 2);
        assert_eq!(tel.count[1], 9);
        ingress.telemetry_inc(TxDone::Normal(3), &mut tel);
        assert_eq!(tel.count[0], 5);
        ingress.monitor_not();
        assert_eq!(ingress.control_channel.local_monitor_index, MONITOR_NOT);
        assert_eq!(ingress.payload_channel.local_monitor_index, MONITOR_NOT);

        let eg_clone = eg_tx.clone();
        let mut egress = eg_clone.lock().await;
        let mut tel = dummy_stream_tel();
        egress.control_channel.local_monitor_index = 0;
        egress.payload_channel.local_monitor_index = 1;
        egress.telemetry_inc(TxDone::Stream(4, 6), &mut tel);
        assert_eq!(tel.count[0], 4);
        assert_eq!(tel.count[1], 6);
        egress.telemetry_inc(TxDone::Normal(1), &mut tel);
        assert_eq!(tel.count[0], 5);
        egress.monitor_not();
        assert_eq!(egress.control_channel.local_monitor_index, MONITOR_NOT);
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_mark_closed_consumes_make_closed_senders() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, in_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, eg_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(16);

        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            assert!(ingress.control_channel.make_closed.is_some());
            assert!(ingress.payload_channel.make_closed.is_some());
            ingress.shared_mark_closed();
            assert!(ingress.control_channel.make_closed.is_none());
            assert!(ingress.payload_channel.make_closed.is_none());
        }
        {
            let rx_clone = in_rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            assert!(rx_guard.is_closed_and_empty());
        }

        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            assert!(egress.control_channel.make_closed.is_some());
            egress.shared_mark_closed();
            assert!(egress.control_channel.make_closed.is_none());
            assert!(egress.payload_channel.make_closed.is_none());
        }
        {
            let rx_clone = eg_rx.clone();
            let mut rx_guard = rx_clone.lock().await;
            assert!(rx_guard.is_closed_and_empty());
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
        assert!(!tx_guard.log_perodic());
        tx_guard.control_channel.last_error_send = Instant::now() - Duration::from_secs(30);
        assert!(tx_guard.log_perodic());
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_log_periodic_logs_when_elapsed_equals_max() {
    core_exec::block_on(async {
        let max = steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64;
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(16);

        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            ingress.control_channel.last_error_send = Instant::now() - Duration::from_secs(max);
            assert_eq!(
                ingress.control_channel.last_error_send.elapsed().as_secs(),
                max,
                "setup must land in the equality second so < vs <= differ"
            );
            assert!(
                ingress.log_perodic(),
                "elapsed.as_secs() == MAX must log; <= would suppress"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            egress.control_channel.last_error_send = Instant::now() - Duration::from_secs(max);
            assert_eq!(
                egress.control_channel.last_error_send.elapsed().as_secs(),
                max
            );
            assert!(
                egress.log_perodic(),
                "egress elapsed.as_secs() == MAX must log; <= would suppress"
            );
        }
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

        let cap = tx_guard.shared_capacity();
        assert!(tx_guard.shared_capacity_for((5, 50)));
        assert!(
            tx_guard.shared_capacity_for(cap),
            "size == capacity must pass (<=); a < mutant rejects the exact fit"
        );
        assert!(!tx_guard.shared_capacity_for((100, 1000)));

        let vacant = tx_guard.shared_vacant_units();
        assert!(tx_guard.shared_vacant_units_for((5, 50)));
        assert!(
            tx_guard.shared_vacant_units_for(vacant),
            "size == vacant must pass (>=); a > mutant rejects the exact fit"
        );
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
        let (ctrl_cap, payload_cap) = ingress.shared_capacity();
        let one = ingress.one();
        assert_eq!(one.0, 1);
        assert_eq!(
            one.1,
            payload_cap / ctrl_cap,
            "one() payload estimate is capacity ratio, not product"
        );
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
        let (eg_ctrl_cap, eg_payload_cap) = egress.shared_capacity();
        let one = egress.one();
        assert_eq!(one.0, 1);
        assert_eq!(
            one.1,
            eg_payload_cap / eg_ctrl_cap,
            "egress one() payload estimate is capacity ratio, not product"
        );
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

    /// Property: `one()` is one control slot plus payload/control capacity (division, not multiply).
    #[test]
    // ss[verify channel.stream-dual-buffer]
    // ss[verify verify.process.mutants]
    // ss[verify verify.process.proptest]
    fn proptest_stream_tx_one_is_payload_over_control_capacity(
        cap in 2usize..16,
        bytes_per_item in 2usize..12,
    ) {
        core_exec::block_on(async {
            let mut graph = GraphBuilder::for_testing().build(());
            let (in_tx, _in_rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamIngress>(bytes_per_item);
            let (eg_tx, _eg_rx) = graph.channel_builder()
                .with_capacity(cap)
                .build_stream::<StreamEgress>(bytes_per_item);

            let in_clone = in_tx.clone();
            let ingress = in_clone.lock().await;
            let (ctrl, payload) = ingress.shared_capacity();
            prop_assert!(ctrl >= 2, "control cap must make / differ from *");
            let one = ingress.one();
            prop_assert_eq!(one.0, 1);
            prop_assert_eq!(one.1, payload / ctrl);
            drop(ingress);

            let eg_clone = eg_tx.clone();
            let egress = eg_clone.lock().await;
            let (eg_ctrl, eg_payload) = egress.shared_capacity();
            prop_assert!(eg_ctrl >= 2);
            let eg_one = egress.one();
            prop_assert_eq!(eg_one.0, 1);
            prop_assert_eq!(eg_one.1, eg_payload / eg_ctrl);
            Ok::<(), TestCaseError>(())
        }).expect("async property");
    }
}

#[test]
// ss[verify distributed.aqueduct-stream]
fn missing_defrag_session_is_skipped() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(8);
        let established = tx.clone();
        let mut guard = established.lock().await;
        guard.ready_msg_session.push_back(99);
        let mut actor = graph
            .new_testing_test_monitor("flush_missing")
            .into_spotlight([], []);
        let (msgs, bytes) = guard.fragment_flush_ready(&mut actor);
        assert_eq!((msgs, bytes), (0, 0));
        assert!(guard.ready_msg_session.is_empty());
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify channel.backpressure-never-drop]
fn stream_send_async_waits_for_room_then_succeeds_or_times_out() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let ident = ActorIdentity::new(0, "stream_wait", None);
        let now = Instant::now();

        let (in_tx, in_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(8);
        let in_tx_held = in_tx.clone();
        let mut ingress = in_tx_held.lock().await;
        ingress
            .shared_try_send((StreamIngress::new(1, 0, now, now), &[1u8][..]))
            .expect("fill ingress");
        assert!(ingress.shared_is_full());
        let timed = ingress
            .shared_send_async_timeout(
                (StreamIngress::new(1, 0, now, now), &[2u8][..]),
                ident,
                SendSaturation::AwaitForRoom,
                Some(Duration::from_millis(15)),
            )
            .await;
        assert!(
            matches!(timed, SendOutcome::Timeout(_)),
            "a full stream with no consumer must time out, got {timed:?}"
        );
        let rx_held = in_rx.clone();
        core_exec::spawn_detached(async move {
            std::thread::sleep(Duration::from_millis(20));
            let mut rx = rx_held.lock().await;
            let _ = rx.shared_try_take();
        });
        let sent = ingress
            .shared_send_async(
                (StreamIngress::new(1, 0, now, now), &[3u8][..]),
                ident,
                SendSaturation::AwaitForRoom,
            )
            .await;
        assert!(
            matches!(sent, SendOutcome::Success),
            "room after a take must complete the parked send, got {sent:?}"
        );
        drop(ingress);

        let (eg_tx, eg_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamEgress>(8);
        let eg_held = eg_tx.clone();
        let mut egress = eg_held.lock().await;
        egress.shared_try_send(&[4u8][..]).expect("fill egress");
        assert!(egress.shared_is_full());
        let eg_rx_held = eg_rx.clone();
        core_exec::spawn_detached(async move {
            std::thread::sleep(Duration::from_millis(20));
            let mut rx = eg_rx_held.lock().await;
            let _ = rx.shared_try_take();
        });
        let sent = egress
            .shared_send_async(&[5u8][..], ident, SendSaturation::AwaitForRoom)
            .await;
        assert!(
            matches!(sent, SendOutcome::Success),
            "egress must publish once a control slot is free, got {sent:?}"
        );
        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
        drop(shutdown_tx);
        egress.control_channel.oneshot_shutdown = shutdown_rx;
        egress.shared_try_send(&[6u8][..]).ok();
        let closed = egress
            .shared_send_async_timeout(
                &[7u8][..],
                ident,
                SendSaturation::AwaitForRoom,
                Some(Duration::from_millis(30)),
            )
            .await;
        assert!(
            matches!(closed, SendOutcome::Closed(_)),
            "a terminated shutdown must close the parked egress send, got {closed:?}"
        );
    });
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

