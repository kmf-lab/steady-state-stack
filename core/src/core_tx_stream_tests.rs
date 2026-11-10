//! Tests extracted from `core_tx_stream.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related channel.stream-dual-buffer]
// --- from mod core_tx_stream_tests ---
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
// ss[verify verify.process.mutants]
fn stream_tx_send_iter_until_full_reports_ingress_count() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(16);
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let now = Instant::now();
        let p1 = [1u8, 2];
        let p2 = [3u8, 4, 5];
        let p3 = [6u8];
        let items = vec![
            (StreamIngress::new(2, 0, now, now), p1.as_slice()),
            (StreamIngress::new(3, 0, now, now), p2.as_slice()),
            (StreamIngress::new(1, 0, now, now), p3.as_slice()),
        ];
        let count = tx_guard.shared_send_iter_until_full(items.into_iter());
        assert_eq!(
            count, 3,
            "Ingress send_iter must return the number sent, not 0"
        );

        let extra = vec![
            (StreamIngress::new(1, 0, now, now), &[7u8][..]),
            (StreamIngress::new(1, 0, now, now), &[8u8][..]),
        ];
        let remaining = tx_guard.shared_vacant_units().0;
        let overflow = tx_guard.shared_send_iter_until_full(extra.into_iter());
        assert_eq!(
            overflow, remaining,
            "send_iter must stop at vacant control slots (take(item_limit))"
        );
        drop(tx_guard);

        let rx_clone = rx.clone();
        let mut rx_guard = rx_clone.lock().await;
        let avail = rx_guard.shared_avail_units();
        assert_eq!(avail.0, 4, "all control slots filled by send_iter");
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_send_iter_waits_when_payload_exceeds_vacant() {
    // send_iter calls block_on(wait_vacant) internally — run it off the outer
    // executor so a nested block_on cannot deadlock the wait path.
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph
        .channel_builder()
        .with_capacity(2)
        .build_stream::<StreamIngress>(4);

    let tx_arc = tx.clone();
    let mut tx_guard = core_exec::block_on(tx_arc.lock());
    let now = Instant::now();
    let first = [9u8; 6];
    tx_guard
        .shared_try_send((StreamIngress::new(6, 0, now, now), first.as_slice()))
        .expect("first frame fits");
    let vacant_payload = tx_guard.shared_vacant_units().1;
    let second_len = vacant_payload + 2;
    assert!(second_len > vacant_payload);
    assert_ne!(
        second_len, vacant_payload,
        "== mutant must skip the wait this test needs"
    );

    let rx_clone = rx.clone();
    let helper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        core_exec::block_on(async {
            let mut g = rx_clone.lock().await;
            assert!(
                g.shared_try_take().is_some(),
                "consumer must free payload so wait_vacant can finish"
            );
        });
    });

    let second = vec![1u8; second_len];
    let count = tx_guard.shared_send_iter_until_full(std::iter::once((
        StreamIngress::new(second_len as i32, 0, now, now),
        second.as_slice(),
    )));
    assert_eq!(count, 1, "wait then push the oversized payload in full");
    drop(tx_guard);
    helper.join().expect("payload consumer");

    let rx_arc = rx.clone();
    let mut rx_guard = core_exec::block_on(rx_arc.lock());
    let taken = rx_guard
        .shared_try_take()
        .expect("control+payload stay matched; == skips wait and truncates payload");
    let (_done, (_item, payload)) = taken;
    assert_eq!(payload.len(), second_len);
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_egress_send_iter_waits_when_payload_exceeds_vacant() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph
        .channel_builder()
        .with_capacity(2)
        .build_stream::<StreamEgress>(4);

    let tx_arc = tx.clone();
    let mut tx_guard = core_exec::block_on(tx_arc.lock());
    tx_guard
        .shared_try_send(&[9u8; 6][..])
        .expect("first egress frame fits");
    let vacant_payload = tx_guard.shared_vacant_units().1;
    let second_len = vacant_payload + 2;
    assert!(second_len > vacant_payload);

    let rx_clone = rx.clone();
    let helper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        core_exec::block_on(async {
            let mut g = rx_clone.lock().await;
            assert!(g.shared_try_take().is_some());
        });
    });

    let second = vec![1u8; second_len];
    let count = tx_guard.shared_send_iter_until_full(std::iter::once(second.as_slice()));
    assert_eq!(count, 1);
    drop(tx_guard);
    helper.join().expect("payload consumer");

    let rx_arc = rx.clone();
    let mut rx_guard = core_exec::block_on(rx_arc.lock());
    let taken = rx_guard
        .shared_try_take()
        .expect("egress control+payload stay matched");
    let (_done, (_item, payload)) = taken;
    assert_eq!(payload.len(), second_len);
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_empty_full_vacant_and_wait_predicates() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamIngress>(8);
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let cap = tx_guard.shared_capacity();
        assert_ne!(cap, (0, 0), "shared_capacity must not collapse to Default");
        assert!(tx_guard.shared_is_empty());
        assert!(!tx_guard.shared_is_full());
        assert_eq!(tx_guard.shared_vacant_units(), cap);
        assert!(tx_guard.shared_vacant_units_for((1, 1)));
        assert!(tx_guard.shared_vacant_units_for(cap));
        assert!(
            !tx_guard.shared_vacant_units_for((100, 10_000)),
            "oversize vacant_units_for must be false; a constant true would pass"
        );
        assert!(tx_guard.shared_capacity_for((1, 1)));
        assert!(!tx_guard.shared_capacity_for((100, 10_000)));
        assert!(
            tx_guard.shared_wait_vacant_units(cap).await,
            "already-vacant wait must return true, not a constant false"
        );
        assert!(
            tx_guard.shared_wait_shutdown_or_vacant_units((1, 1)).await,
            "empty stream tx already has room; must not be a constant false"
        );
        assert!(tx_guard.shared_wait_shutdown_or_vacant_units(cap).await);
        assert!(tx_guard.shared_wait_empty().await);

        let now = Instant::now();
        tx_guard
            .shared_try_send((StreamIngress::new(1, 0, now, now), &[1u8][..]))
            .unwrap();
        tx_guard
            .shared_try_send((StreamIngress::new(1, 0, now, now), &[2u8][..]))
            .unwrap();
        assert!(tx_guard.shared_is_full());
        assert!(!tx_guard.shared_is_empty());

        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
        drop(shutdown_tx);
        tx_guard.control_channel.oneshot_shutdown = shutdown_rx;
        assert!(
            !tx_guard.shared_wait_empty().await,
            "terminated oneshot + non-empty must be capacity != vacant, not a constant true"
        );
        drop(tx_guard);

        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamEgress>(8);
        let eg_clone = eg_tx.clone();
        let mut egress = eg_clone.lock().await;
        let eg_cap = egress.shared_capacity();
        assert_ne!(eg_cap, (0, 0), "egress shared_capacity must not collapse to Default");
        assert!(egress.shared_is_empty());
        assert!(
            !egress.shared_is_full(),
            "empty egress must not report full; a constant true would pass fill-only asserts"
        );
        assert_eq!(egress.shared_vacant_units(), eg_cap);
        assert!(egress.shared_vacant_units_for((1, 1)));
        assert!(egress.shared_capacity_for((1, 1)));
        assert!(!egress.shared_capacity_for((100, 10_000)));
        assert!(
            egress.shared_wait_shutdown_or_vacant_units((1, 1)).await,
            "empty egress already has room"
        );
        assert!(!egress.shared_vacant_units_for((100, 10_000)));
        assert!(egress.shared_wait_vacant_units(eg_cap).await);
        assert!(egress.shared_wait_empty().await);
        egress.shared_try_send(&[3u8][..]).unwrap();
        egress.shared_try_send(&[4u8][..]).unwrap();
        assert!(egress.shared_is_full());
        assert!(!egress.shared_is_empty());
        let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
        drop(shutdown_tx);
        egress.control_channel.oneshot_shutdown = shutdown_rx;
        assert!(
            !egress.shared_wait_empty().await,
            "terminated oneshot + non-empty egress must not be a constant true"
        );
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_wait_shutdown_or_vacant_uses_or_not_and() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamIngress>(8);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamEgress>(8);

        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            let now = Instant::now();
            ingress
                .shared_try_send((StreamIngress::new(1, 0, now, now), &[1u8][..]))
                .unwrap();
            assert!(!ingress.shared_is_empty());
            assert!(ingress.shared_vacant_units().0 >= 1);
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            ingress.control_channel.oneshot_shutdown = shutdown_rx;
            assert!(
                ingress.shared_wait_shutdown_or_vacant_units((1, 1)).await,
                "not-empty but vacant>=count must short-circuit true; && would take terminated false"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            egress.shared_try_send(&[2u8][..]).unwrap();
            assert!(!egress.shared_is_empty());
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            egress.control_channel.oneshot_shutdown = shutdown_rx;
            assert!(
                egress.shared_wait_shutdown_or_vacant_units((1, 1)).await,
                "egress vacant>=count must not require is_empty"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_wait_shutdown_or_vacant_false_when_full_and_shutdown() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(8);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamEgress>(8);
        let now = Instant::now();
        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            ingress
                .shared_try_send((StreamIngress::new(1, 0, now, now), &[1u8][..]))
                .unwrap();
            assert!(ingress.shared_is_full());
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            ingress.control_channel.oneshot_shutdown = shutdown_rx;
            assert!(
                !ingress.shared_wait_shutdown_or_vacant_units((1, 1)).await,
                "full + terminated shutdown must return false; a constant true would pass"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            egress.shared_try_send(&[2u8][..]).unwrap();
            assert!(egress.shared_is_full());
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            egress.control_channel.oneshot_shutdown = shutdown_rx;
            assert!(
                !egress.shared_wait_shutdown_or_vacant_units((1, 1)).await,
                "full egress + terminated shutdown must return false; constant true survived"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_wait_vacant_units_blocks_until_room() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(8);
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let now = Instant::now();
        tx_guard
            .shared_try_send((StreamIngress::new(1, 0, now, now), &[7u8][..]))
            .unwrap();
        assert!(tx_guard.shared_is_full());

        let rx_clone = rx.clone();
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            core_exec::block_on(async {
                let mut g = rx_clone.lock().await;
                assert!(g.shared_try_take().is_some());
            });
        });

        let start = Instant::now();
        assert!(
            tx_guard.shared_wait_vacant_units((1, 1)).await,
            "wait_vacant_units returns true after room appears"
        );
        assert!(
            start.elapsed() >= Duration::from_millis(40),
            "constant-true mutant returns before the consumer frees a slot"
        );
        helper.join().expect("consumer");
        drop(tx_guard);

        let (eg_tx, eg_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamEgress>(8);
        let eg_clone = eg_tx.clone();
        let mut egress = eg_clone.lock().await;
        egress.shared_try_send(&[8u8][..]).unwrap();
        let eg_rx_clone = eg_rx.clone();
        let eg_helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            core_exec::block_on(async {
                let mut g = eg_rx_clone.lock().await;
                assert!(g.shared_try_take().is_some());
            });
        });
        let eg_start = Instant::now();
        assert!(egress.shared_wait_vacant_units((1, 1)).await);
        assert!(
            eg_start.elapsed() >= Duration::from_millis(40),
            "egress wait_vacant_units must block; constant true does not"
        );
        eg_helper.join().expect("egress consumer");
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_wait_vacant_checks_payload_not_only_control() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamIngress>(4);
        let tx_clone = tx.clone();
        let mut tx_guard = tx_clone.lock().await;
        let now = Instant::now();
        let (ctrl_cap, payload_cap) = tx_guard.shared_capacity();
        assert!(ctrl_cap >= 2, "need a leftover control slot after filling payload");
        let fill = vec![1u8; payload_cap];
        tx_guard
            .shared_try_send((StreamIngress::new(payload_cap as i32, 0, now, now), fill.as_slice()))
            .expect("one control item can fill the payload ring");
        let (ctrl_vacant, payload_vacant) = tx_guard.shared_vacant_units();
        assert!(ctrl_vacant >= 1);
        assert_eq!(payload_vacant, 0);

        let rx_clone = rx.clone();
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            core_exec::block_on(async {
                let mut g = rx_clone.lock().await;
                assert!(g.shared_try_take().is_some());
            });
        });
        let start = Instant::now();
        assert!(tx_guard.shared_wait_vacant_units((1, 1)).await);
        assert!(
            start.elapsed() >= Duration::from_millis(40),
            "payload vacant==0 must wait; mutating payload >= into < would return immediately"
        );
        helper.join().expect("consumer");
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_egress_wait_vacant_checks_payload_not_only_control() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph
            .channel_builder()
            .with_capacity(2)
            .build_stream::<StreamEgress>(4);
        let tx_clone = tx.clone();
        let mut egress = tx_clone.lock().await;
        let (ctrl_cap, payload_cap) = egress.shared_capacity();
        assert!(ctrl_cap >= 2, "need a leftover control slot after filling payload");
        let fill = vec![1u8; payload_cap];
        egress
            .shared_try_send(fill.as_slice())
            .expect("one egress item can fill the payload ring");
        let (ctrl_vacant, payload_vacant) = egress.shared_vacant_units();
        assert!(ctrl_vacant >= 1);
        assert_eq!(payload_vacant, 0);

        let rx_clone = rx.clone();
        let helper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            core_exec::block_on(async {
                let mut g = rx_clone.lock().await;
                assert!(g.shared_try_take().is_some());
            });
        });
        let start = Instant::now();
        assert!(egress.shared_wait_vacant_units((1, 1)).await);
        assert!(
            start.elapsed() >= Duration::from_millis(40),
            "egress payload vacant==0 must wait; mutating payload >= into < returns immediately"
        );
        helper.join().expect("consumer");
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_send_slice_stops_when_source_or_vacant_exhausted() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(8)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(8)
            .build_stream::<StreamEgress>(16);

        let now = Instant::now();
        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            let items = [
                StreamIngress::new(3, 0, now, now),
                StreamIngress::new(3, 0, now, now),
            ];
            let short_payload = [1u8, 2, 3, 4];
            let done = ingress.shared_send_slice((&items, &short_payload));
            assert_eq!(
                done,
                TxDone::Stream(1, 3),
                "second item needs 6 source bytes; || breaks, && would slice past the source"
            );
            let exact = [
                StreamIngress::new(2, 0, now, now),
                StreamIngress::new(2, 0, now, now),
            ];
            let exact_payload = [9u8, 9, 8, 8];
            let exact_done = ingress.shared_send_slice((&exact, &exact_payload));
            assert_eq!(
                exact_done,
                TxDone::Stream(2, 4),
                "bytes_sent+len == source len must send the last item (> not >=)"
            );
        }

        {
            let mut graph2 = GraphBuilder::for_testing().build(());
            let (tx2, _rx2) = graph2
                .channel_builder()
                .with_capacity(2)
                .build_stream::<StreamIngress>(4);
            let tx2_clone = tx2.clone();
            let mut tx2_guard = tx2_clone.lock().await;
            let (_ctrl_cap, payload_cap) = tx2_guard.shared_capacity();
            let first_len = payload_cap / 2;
            assert!(first_len > 0);
            let first_bytes = vec![7u8; first_len];
            tx2_guard
                .shared_try_send((
                    StreamIngress::new(first_len as i32, 0, now, now),
                    first_bytes.as_slice(),
                ))
                .unwrap();
            let vacant = tx2_guard.shared_vacant_units().1;
            assert!(vacant > 0);
            let fit = [StreamIngress::new(vacant as i32, 0, now, now)];
            let fit_bytes = vec![3u8; vacant];
            let fit_done = tx2_guard.shared_send_slice((&fit, fit_bytes.as_slice()));
            assert_eq!(
                fit_done,
                TxDone::Stream(1, vacant),
                "bytes_sent+len == payload_vacant must send (> vacant is false; == would skip)"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            let items = [StreamEgress { length: 3 }, StreamEgress { length: 3 }];
            let short_payload = [1u8, 2, 3, 4];
            let done = egress.shared_send_slice((&items, &short_payload));
            assert_eq!(done, TxDone::Stream(1, 3));
        }
        {
            let mut graph2 = GraphBuilder::for_testing().build(());
            let (tx2, _rx2) = graph2
                .channel_builder()
                .with_capacity(2)
                .build_stream::<StreamEgress>(4);
            let tx2_clone = tx2.clone();
            let mut tx2_guard = tx2_clone.lock().await;
            let (_ctrl_cap, payload_cap) = tx2_guard.shared_capacity();
            let first_len = payload_cap / 2;
            assert!(first_len > 0);
            let first_bytes = vec![7u8; first_len];
            tx2_guard.shared_try_send(first_bytes.as_slice()).unwrap();
            let vacant = tx2_guard.shared_vacant_units().1;
            assert!(vacant > 0);
            let fit = [StreamEgress { length: vacant as i32 }];
            let fit_bytes = vec![3u8; vacant];
            let fit_done = tx2_guard.shared_send_slice((&fit, fit_bytes.as_slice()));
            assert_eq!(
                fit_done,
                TxDone::Stream(1, vacant),
                "egress bytes_sent+len == payload_vacant must send (> vacant is false; == would skip)"
            );
        }
        {
            let mut graph3 = GraphBuilder::for_testing().build(());
            let (tx3, _rx3) = graph3
                .channel_builder()
                .with_capacity(4)
                .build_stream::<StreamEgress>(16);
            let tx3_clone = tx3.clone();
            let mut tx3_guard = tx3_clone.lock().await;
            let vacant = tx3_guard.shared_vacant_units().1;
            if vacant > 8 {
                let eat = vacant - 8;
                tx3_guard
                    .shared_try_send(&vec![0u8; eat])
                    .expect("shrink payload vacant to 8");
            }
            let vacant = tx3_guard.shared_vacant_units().1;
            assert_eq!(vacant, 8, "two length-3 items: + sends both, * (3*3=9) skips the second");
            let items = [StreamEgress { length: 3 }, StreamEgress { length: 3 }];
            let payload = [1u8, 2, 3, 4, 5, 6];
            let done = tx3_guard.shared_send_slice((&items, &payload));
            assert_eq!(
                done,
                TxDone::Stream(2, 6),
                "bytes_sent+len must not become bytes_sent*len"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_poke_slice_exposes_vacant_rings_not_default() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(8);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(8);

        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            let (ctrl_vacant, payload_vacant) = ingress.shared_vacant_units();
            let (item_a, item_b, payload_a, payload_b) = ingress.shared_poke_slice();
            assert_eq!(
                item_a.len() + item_b.len(),
                ctrl_vacant,
                "poke_slice must expose the control ring, not Default empty slices"
            );
            assert_eq!(payload_a.len() + payload_b.len(), payload_vacant);
            assert!(ctrl_vacant > 0);
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            let (ctrl_vacant, payload_vacant) = egress.shared_vacant_units();
            let (item_a, item_b, payload_a, payload_b) = egress.shared_poke_slice();
            assert_eq!(item_a.len() + item_b.len(), ctrl_vacant);
            assert_eq!(payload_a.len() + payload_b.len(), payload_vacant);
            assert!(payload_vacant > 0);
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_try_send_requires_control_and_payload_room() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamEgress>(16);
        let now = Instant::now();
        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            ingress
                .shared_try_send((StreamIngress::new(1, 0, now, now), &[1u8][..]))
                .unwrap();
            assert!(ingress.shared_is_full());
            let extra = ingress.shared_try_send((
                StreamIngress::new(1, 0, now, now),
                &[2u8][..],
            ));
            assert!(
                extra.is_err(),
                "control full must reject even when payload has room; || would send"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            egress.shared_try_send(&[3u8][..]).unwrap();
            assert!(egress.shared_is_full());
            assert!(
                egress.shared_try_send(&[4u8][..]).is_err(),
                "egress control full must reject"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_send_async_succeeds_immediately_when_there_is_room() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamIngress>(16);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(16);
        let ident = ActorIdentity::default();
        let now = Instant::now();
        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            ingress.control_channel.oneshot_shutdown = shutdown_rx;
            let outcome = ingress
                .shared_send_async(
                    (StreamIngress::new(2, 0, now, now), &[9u8, 8][..]),
                    ident,
                    SendSaturation::AwaitForRoom,
                )
                .await;
            assert!(
                matches!(outcome, SendOutcome::Success),
                "room exists so payload vacant >= len must succeed; < would take Closed on terminated oneshot"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            egress.control_channel.oneshot_shutdown = shutdown_rx;
            let outcome = egress
                .shared_send_async(&[1u8, 2, 3][..], ident, SendSaturation::AwaitForRoom)
                .await;
            assert!(
                matches!(outcome, SendOutcome::Success),
                "egress with room must succeed immediately"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_send_async_returns_blocked_when_full() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (in_tx, _in_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamIngress>(8);
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(1)
            .build_stream::<StreamEgress>(8);
        let ident = ActorIdentity::default();
        let now = Instant::now();
        {
            let in_clone = in_tx.clone();
            let mut ingress = in_clone.lock().await;
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            ingress.control_channel.oneshot_shutdown = shutdown_rx;
            let vacant = ingress.shared_vacant_units();
            assert_eq!(vacant.0, 1, "single control slot distinguishes >= 1 from > 1");
            let payload = vec![7u8; vacant.1];
            let item = StreamIngress::new(payload.len() as i32, 0, now, now);
            let outcome = ingress
                .shared_send_async((item, payload.as_slice()), ident, SendSaturation::AwaitForRoom)
                .await;
            assert!(
                matches!(outcome, SendOutcome::Success),
                "exact payload+control fit must succeed; > vacant would wait then Closed"
            );
            assert!(ingress.shared_is_full());
            let extra = StreamIngress::new(1, 0, now, now);
            #[allow(deprecated)]
            let blocked = ingress
                .shared_send_async((extra, &[1u8][..]), ident, SendSaturation::ReturnBlockedMsg)
                .await;
            assert!(
                matches!(blocked, SendOutcome::Blocked(_)),
                "full + ReturnBlockedMsg must be Blocked; Default::default() of send_async is Success"
            );
        }
        {
            let eg_clone = eg_tx.clone();
            let mut egress = eg_clone.lock().await;
            let (shutdown_tx, shutdown_rx) = futures::channel::oneshot::channel::<()>();
            drop(shutdown_tx);
            egress.control_channel.oneshot_shutdown = shutdown_rx;
            let vacant = egress.shared_vacant_units();
            let payload = vec![3u8; vacant.1];
            let outcome = egress
                .shared_send_async(payload.as_slice(), ident, SendSaturation::AwaitForRoom)
                .await;
            assert!(
                matches!(outcome, SendOutcome::Success),
                "egress exact fit must succeed immediately"
            );
            assert!(egress.shared_is_full());
            #[allow(deprecated)]
            let blocked = egress
                .shared_send_async(&[9u8][..], ident, SendSaturation::ReturnBlockedMsg)
                .await;
            assert!(
                matches!(blocked, SendOutcome::Blocked(_)),
                "egress Default replacement of send_async is Success"
            );
        }
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_egress_advance_index_requires_control_and_payload_room() {
    core_exec::block_on(async {
        let mut graph = GraphBuilder::for_testing().build(());
        let (eg_tx, _eg_rx) = graph
            .channel_builder()
            .with_capacity(4)
            .build_stream::<StreamEgress>(16);
        let eg_clone = eg_tx.clone();
        let mut egress = eg_clone.lock().await;
        assert_eq!(
            egress.shared_advance_index((1, 10_000)),
            TxDone::Stream(0, 0),
            "control room without payload room must not advance; || would"
        );
        assert_eq!(
            egress.shared_advance_index((10_000, 1)),
            TxDone::Stream(0, 0),
            "payload room without control room must not advance"
        );
        let vacant = egress.shared_vacant_units();
        assert_eq!(
            egress.shared_advance_index(vacant),
            TxDone::Stream(vacant.0, vacant.1),
            "exact vacant fit must advance; < would reject equality"
        );
    });
}

#[test]
// ss[verify channel.stream-dual-buffer]
// ss[verify verify.process.mutants]
fn stream_tx_send_iter_does_not_wait_when_payload_exactly_fits() {
    let _guard = start_log_capture();
    let mut graph = GraphBuilder::for_testing().build(());
    let (in_tx, _in_rx) = graph
        .channel_builder()
        .with_capacity(2)
        .build_stream::<StreamIngress>(4);
    let (eg_tx, _eg_rx) = graph
        .channel_builder()
        .with_capacity(2)
        .build_stream::<StreamEgress>(4);

    let in_arc = in_tx.clone();
    let mut ingress = core_exec::block_on(in_arc.lock());
    let now = Instant::now();
    ingress
        .shared_try_send((StreamIngress::new(5, 0, now, now), &[9u8; 5][..]))
        .expect("partial fill");
    let vacant = ingress.shared_vacant_units().1;
    assert!(vacant > 0);
    let exact = vec![3u8; vacant];
    let count = ingress.shared_send_iter_until_full(std::iter::once((
        StreamIngress::new(vacant as i32, 0, now, now),
        exact.as_slice(),
    )));
    assert_eq!(count, 1);
    drop(ingress);

    let eg_arc = eg_tx.clone();
    let mut egress = core_exec::block_on(eg_arc.lock());
    egress
        .shared_try_send(&[8u8; 5][..])
        .expect("egress partial fill");
    let eg_vacant = egress.shared_vacant_units().1;
    assert!(eg_vacant > 0);
    let eg_exact = vec![4u8; eg_vacant];
    let eg_count = egress.shared_send_iter_until_full(std::iter::once(eg_exact.as_slice()));
    assert_eq!(eg_count, 1);
    drop(egress);

    std::thread::sleep(Duration::from_millis(10));
    let thread_id = std::thread::current().id();
    let logged = TEST_CONTEXTS
        .lock()
        .ok()
        .and_then(|ctx| ctx.get(&thread_id).map(|s| s.log_buffer.lock().ok().map(|b| b.clone())))
        .flatten()
        .unwrap_or_default();
    assert!(
        !logged.iter().any(|m| m.contains("payload of the stream should be larger")),
        "exact fit must not wait/warn; >= would warn on vacant == len. logs={logged:?}"
    );
}
