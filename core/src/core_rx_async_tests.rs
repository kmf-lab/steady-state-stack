// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::Rx;
// ss[related philosophy.zero-copy-discipline]
use crate::steady_tx::Tx;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related philosophy.zero-copy-discipline]
use futures::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::{ActorIdentity, SendSaturation};
// ss[related philosophy.zero-copy-discipline]
use crate::*;

// ss[related philosophy.zero-copy-discipline]
fn setup_channel<T: Clone + Send + 'static>(
    capacity: usize,
    data: Option<Vec<T>>,
) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>, Graph) {
    let mut graph = GraphBuilder::for_testing().build(());
    let builder = graph.channel_builder();
    let builder = builder.with_capacity(capacity);
    let (tx_lazy, rx_lazy) = builder.build_channel::<T>();
    let tx = tx_lazy.clone();
    let rx = rx_lazy.clone();
    if let Some(values) = data {
        core_exec::block_on(async {
            let mut tx_guard = tx.lock().await;
            for value in values {
                let _ = tx_guard.shared_send_async(value, ActorIdentity::default(), SendSaturation::default()).await;
            }
        });
    }
    graph.start();
    (tx, rx, graph)
}

// ss[verify actor.wait-avail-vacant]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_wait_avail_units() {
    let (tx, rx, _) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");
    let wait_future = rx_guard.shared_wait_avail_units(1);
    core_exec::block_on(async {
        let mut tx_guard = tx.lock().await;
        tx_guard.shared_send_async(42, ActorIdentity::default(), SendSaturation::default()).await;
    });
    assert!(core_exec::block_on(wait_future), "Wait should return true when units become available");
}

// ss[verify actor.wait-avail-vacant]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_wait_closed_or_avail_units() {
    let (tx, rx, _) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");
    let wait_future = rx_guard.shared_wait_closed_or_avail_units(1);
    let mut tx_guard = tx.try_lock().expect("");
    tx_guard.mark_closed();
    let result = core_exec::block_on(wait_future);
    assert!(!result, "Wait should return false when channel is closed with no units available");
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_rx_core_state_boundaries() {
    let (tx, rx, _graph) = setup_channel::<i32>(4, None);
    let mut rx_guard = rx.try_lock().expect("");
    
    let done = rx_guard.shared_advance_index(10);
    assert_eq!(done, RxDone::Normal(0));

    core_exec::block_on(async {
        let mut tx_guard = tx.lock().await;
        for i in 0..4 { tx_guard.shared_try_send(i).unwrap(); }
    });
    assert_eq!(rx_guard.shared_avail_units(), 4);
    rx_guard.shared_advance_index(2);
    assert_eq!(rx_guard.shared_avail_units(), 2);
    
    core_exec::block_on(async {
        let mut tx_guard = tx.lock().await;
        tx_guard.shared_try_send(4).unwrap();
        tx_guard.shared_try_send(5).unwrap();
    });
    assert_eq!(rx_guard.shared_avail_units(), 4);

    assert!(!rx_guard.is_closed_and_empty());
    rx_guard.shared_advance_index(4);
    assert!(!rx_guard.is_closed_and_empty());
    
    let mut tx_guard = tx.try_lock().expect("");
    tx_guard.mark_closed();
    drop(tx_guard);
    
    assert!(rx_guard.is_closed_and_empty());
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_rx_telemetry_warning() {
    let (_tx, rx, graph) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");
    let meta = rx_guard.channel_meta_data.meta_data.clone();
    let mut actor = graph.new_testing_test_monitor("test")
        .into_spotlight([&meta as &dyn RxMetaDataProvider], []);

    if let Some(ref mut tel) = actor.telemetry.send_rx {
        rx_guard.telemetry_inc(RxDone::Stream(1, 10), tel);
    }
}

// ss[verify actor.wait-avail-vacant]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_wait_avail_units_shutdown() {
    let (_tx, rx, _) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");

    let (send_shutdown, recv_shutdown) = futures::channel::oneshot::channel::<()>();
    rx_guard.oneshot_shutdown = recv_shutdown;
    drop(send_shutdown);

    let result = core_exec::block_on(rx_guard.shared_wait_avail_units(1));
    assert!(!result, "Wait should return false when shutdown fires");
}

// ss[verify actor.wait-avail-vacant]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_wait_shutdown_or_avail_units_shutdown() {
    let (_tx, rx, _) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");

    let (send_shutdown, recv_shutdown) = futures::channel::oneshot::channel::<()>();
    rx_guard.oneshot_shutdown = recv_shutdown;
    drop(send_shutdown);

    let result = core_exec::block_on(rx_guard.shared_wait_shutdown_or_avail_units(1));
    assert!(!result, "Wait should return false when shutdown fires");
}

// ss[verify philosophy.zero-copy-discipline]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_try_take_empty() {
    let (_tx, rx, _) = setup_channel::<i32>(1, None);
    let mut rx_guard = rx.try_lock().expect("");
    assert!(rx_guard.shared_try_take().is_none(), "try_take should return None on empty channel");
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_advance_index_valid() {
    let (_tx, rx, _) = setup_channel::<i32>(4, Some(vec![10, 20, 30]));
    let mut rx_guard = rx.try_lock().expect("");
    let done = rx_guard.shared_advance_index(2);
    assert_eq!(done, RxDone::Normal(2));
    assert_eq!(rx_guard.shared_avail_units(), 1);
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_log_periodic() {
    let (_tx, rx, _) = setup_channel::<i32>(4, None);
    let mut rx_guard = rx.try_lock().expect("");
    rx_guard.last_error_send = Instant::now() - Duration::from_secs((steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS + 1) as u64);
    assert!(rx_guard.log_periodic());
    assert!(!rx_guard.log_periodic());
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_monitor_not() {
    let (_tx, rx, _) = setup_channel::<i32>(4, None);
    let mut rx_guard = rx.try_lock().expect("");
    rx_guard.monitor_not();
    assert_eq!(rx_guard.local_monitor_index, MONITOR_NOT);
}

#[test]
// ss[verify philosophy.zero-copy-discipline]
fn test_one() {
    let (_tx, rx, _) = setup_channel::<i32>(4, None);
    let rx_guard = rx.try_lock().expect("");
    assert_eq!(rx_guard.one(), 1);
}

// ss[related philosophy.zero-copy-discipline]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder::ChannelBuilder;
// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, channel_fifo_take, message_vec};

ss_proptest! {

    /// Property: RxCore delivery preserves FIFO order for any message sequence.
    #[test]
    // ss[verify channel.testing-take-all]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_rx_fifo_order(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let taken = channel_fifo_take(cap, messages.clone());
        prop_assert_eq!(taken, messages);
    }

    /// Property: peek_slice and peek_async do not change avail_units.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_peek_preserves_avail_length(
        cap in 1usize..64,
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        prop_assume!(!messages.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        tx.testing_send_all(messages, false);
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let avail_before = rx_guard.shared_avail_units();
        let (a, b) = rx_guard.shared_peek_slice();
        prop_assert_eq!(a.len() + b.len(), avail_before);
        let avail_after_slice = rx_guard.shared_avail_units();
        prop_assert_eq!(avail_before, avail_after_slice);
        let peeked = core_exec::block_on(rx_guard.shared_peek_async_timeout(None));
        prop_assert!(peeked.is_some());
        prop_assert_eq!(rx_guard.shared_avail_units(), avail_before);
    }

    /// Property: avail_units plus items taken never exceeds channel capacity.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify verify.process.proptest]
    fn proptest_avail_plus_taken_le_capacity(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        tx.testing_send_all(messages.clone(), false);
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let channel_capacity = rx_guard.shared_capacity();
        let mut taken = 0usize;
        loop {
            let avail = rx_guard.shared_avail_units();
            prop_assert!(avail <= channel_capacity);
            prop_assert!(taken + avail <= channel_capacity);
            if let Some((done, _)) = rx_guard.shared_try_take() {
                taken += done.item_count();
            } else {
                break;
            }
        }
        prop_assert_eq!(taken, messages.len());
    }

    /// Property: after transmitter drop, RxCore operations complete without panic.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_shutdown_operations_no_panic(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let (tx, rx, _graph) = setup_channel(cap, Some(messages));
        drop(tx);
        let mut rx_guard = rx.try_lock().expect("");
        let _ = rx_guard.shared_peek_slice();
        let _ = rx_guard.shared_try_take();
        let _ = rx_guard.shared_avail_units();
        let mut buf = [0i32; 4];
        let _ = rx_guard.shared_take_slice(&mut buf[..cap.min(4)]);
        let _ = core_exec::block_on(
            rx_guard.shared_peek_async_timeout(Some(Duration::from_millis(1))),
        );
        let _ = core_exec::block_on(rx_guard.shared_wait_avail_units(1));
    }

    /// Property: take_slice never copies more items than avail_units.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_take_slice_never_exceeds_avail(
        cap in 2usize..64,
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        prop_assume!(!messages.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        tx.testing_send_all(messages, false);
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let avail = rx_guard.shared_avail_units();
        let mut buf = vec![0i32; avail + 8];
        let done = rx_guard.shared_take_slice(&mut buf);
        prop_assert!(done.item_count() <= avail);
        prop_assert_eq!(done.item_count(), avail);
    }

    /// Property: advance_index never moves more items than currently available.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_advance_index_bounded(
        cap in 2usize..64,
        messages in message_vec::<i32>(),
        take_count in 1usize..32,
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        prop_assume!(!messages.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        tx.testing_send_all(messages, false);
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let avail = rx_guard.shared_avail_units();
        let done = rx_guard.shared_advance_index(take_count);
        let advanced = done.item_count();
        prop_assert!(advanced <= avail);
        prop_assert!(advanced <= take_count);
    }

    /// Property: advance_index on an empty channel returns zero progress.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_advance_index_empty_returns_zero(
        cap in capacity(),
        overshoot in 1usize..32,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx, rx_lazy) = builder.build_channel::<i32>();
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let done = rx_guard.shared_advance_index(overshoot);
        prop_assert_eq!(done, RxDone::Normal(0));
    }

    /// Property: validate_capacity_items never exceeds channel capacity.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_validate_capacity_items_clamped(
        cap in capacity(),
        request in 1usize..256,
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx, rx_lazy) = builder.build_channel::<i32>();
        let rx = rx_lazy.clone();
        let rx_guard = core_exec::block_on(rx.lock());
        let clamped = rx_guard.shared_validate_capacity_items(request);
        prop_assert!(clamped <= cap);
        prop_assert_eq!(clamped, cap.min(request));
    }

    /// Property: capacity_for and avail_units_for agree with shared_capacity and avail_units.
    #[test]
    // ss[verify actor.wait-avail-vacant]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_capacity_for_matches_avail(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx, rx_lazy) = builder.build_channel::<i32>();
        tx.testing_send_all(messages, false);
        let rx = rx_lazy.clone();
        let mut rx_guard = core_exec::block_on(rx.lock());
        let avail = rx_guard.shared_avail_units();
        prop_assert_eq!(rx_guard.shared_capacity(), cap);
        prop_assert!(rx_guard.shared_capacity_for(cap));
        prop_assert!(!rx_guard.shared_capacity_for(cap + 1));
        prop_assert_eq!(rx_guard.shared_avail_items_count(), avail);
        prop_assert!(rx_guard.shared_avail_units_for(avail));
        if avail > 0 {
            prop_assert!(!rx_guard.shared_avail_units_for(avail + 1));
        }
    }
}
