// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder::ChannelBuilder;
// ss[related channel.backpressure-never-drop]
use crate::*;

/// Tests the basic flow of sending and receiving messages through lazy transmitter and receiver channels.
// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_lazy_flow() {
    let builder = ChannelBuilder::default().with_capacity(3);
    let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();

    let tx = tx_lazy.clone();
    {
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 3);
        assert_eq!(ste_tx.shared_vacant_units(), 3);
    }

    // Clones the transmitter lazily and sends messages.
    tx_lazy.testing_send_all(vec![1, 2], false);

    // A full ring has no vacant send slots until the reader consumes.
    let tx = tx_lazy.clone();
    {
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_vacant_units(), 1);
    }

    // Locks and peeks at the receiver's next message.
    let rx = rx_lazy.clone();
    let ste_rx = core_exec::block_on(rx.lock());
    assert_eq!(ste_rx.try_peek(), Some(&1));
    drop(ste_rx);

    // Locks and retrieves messages from the receiver.
    let rx = rx_lazy.clone();
    let mut ste_rx = core_exec::block_on(rx.lock());
    assert_eq!(ste_rx.try_take(), Some(1));
    assert_eq!(ste_rx.try_take(), Some(2));
    assert_eq!(ste_rx.try_take(), None);
}

// ss[verify bundle.index-wait-readiness]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_steady_rx_bundle_wait_avail_index() {
crate::core_exec::block_on(async {

    // ss[related channel.backpressure-never-drop]
    use crate::SteadyRxBundleTrait;
    let b0 = ChannelBuilder::default().with_capacity(4);
    let (tx0, rx0) = b0.build_channel::<i32>();
    let b1 = ChannelBuilder::default().with_capacity(4);
    let (tx1, rx1) = b1.build_channel::<i32>();
    tx0.testing_send_all(vec![1], false);
    tx1.testing_send_all(vec![2], false);
    let bundle: SteadyRxBundle<i32, 2> = Arc::new([rx0.clone(), rx1.clone()]);
    let idx = bundle.wait_avail_index(&[1, 1]).await;
    assert!(idx == 0 || idx == 1);
    });
}

// ss[verify channel.backpressure-never-drop]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_steady_tx_bundle_wait_vacant_index() {
crate::core_exec::block_on(async {

    // ss[related channel.backpressure-never-drop]
    use crate::SteadyTxBundleTrait;
    let b0 = ChannelBuilder::default().with_capacity(4);
    let (tx0, _rx0) = b0.build_channel::<i32>();
    let b1 = ChannelBuilder::default().with_capacity(4);
    let (tx1, _rx1) = b1.build_channel::<i32>();
    let bundle: SteadyTxBundle<i32, 2> = Arc::new([tx0.clone(), tx1.clone()]);
    let idx = bundle.wait_vacant_index(&[1, 1]).await;
    assert!(idx == 0 || idx == 1);
    });
}

/// Tests the `width()` and `memory_bytes()` accessors on an established Tx.
// ss[verify channel.memory-usage-telemetry]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_tx_width_and_memory_bytes() {
    let builder = ChannelBuilder::default().with_capacity(10);
    let (tx_lazy, _rx_lazy) = builder.build_channel::<u32>();
    let tx = tx_lazy.clone();
    let ste_tx = core_exec::block_on(tx.lock());
    assert_eq!(ste_tx.width(), 4, "u32 width must be 4 bytes");
    assert_eq!(ste_tx.capacity(), 10);
    assert_eq!(ste_tx.memory_bytes(), 40, "memory must be capacity × width");
}

/// Tests the bundle-level `memory_bytes()` rollup across lanes.
// ss[verify channel.memory-usage-telemetry]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_tx_bundle_memory_bytes() {
    // ss[related philosophy.structural-hierarchy]
    use crate::SteadyTxBundleTrait;
    let b0 = ChannelBuilder::default().with_capacity(4);
    let (tx0, _rx0) = b0.build_channel::<u64>();
    let b1 = ChannelBuilder::default().with_capacity(8);
    let (tx1, _rx1) = b1.build_channel::<u64>();
    let bundle: SteadyTxBundle<u64, 2> = Arc::new([tx0.clone(), tx1.clone()]);
    // lane0: 4 × 8 = 32, lane1: 8 × 8 = 64 → 96 total
    assert_eq!(bundle.memory_bytes(), 96);
}

// ss[related channel.backpressure-never-drop]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, channel_fifo_take, lane_mask, message_vec};

ss_proptest! {

    /// Property: zero-count lane is returned immediately by wait_vacant_index.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify verify.process.proptest]
    fn proptest_wait_vacant_index_zero_lane(
        lane in 0usize..2,
        other_count in 1usize..8,
    ) {
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyTxBundleTrait;
        let b0 = ChannelBuilder::default().with_capacity(8);
        let (tx0, _rx0) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(8);
        let (tx1, _rx1) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> = Arc::new([tx0.clone(), tx1.clone()]);
        let mut counts = [other_count, other_count];
        counts[lane] = 0;
        let idx = core_exec::block_on(bundle.wait_vacant_index(&counts));
        prop_assert_eq!(idx, lane);
    }

    /// Property: wait_vacant_index returns a valid lane when all counts are positive.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_wait_vacant_index_ready_lane(
        count in 1usize..4,
    ) {
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyTxBundleTrait;
        let b0 = ChannelBuilder::default().with_capacity(8);
        let (tx0, _rx0) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(8);
        let (tx1, _rx1) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> = Arc::new([tx0.clone(), tx1.clone()]);
        let idx = core_exec::block_on(bundle.wait_vacant_index(&[count, count]));
        prop_assert!(idx < 2);
    }

    /// Property: mirror RX — zero-count lane is returned immediately by wait_avail_index.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_index_zero_lane(
        lane in 0usize..2,
        other_count in 1usize..8,
    ) {
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyRxBundleTrait;
        let b0 = ChannelBuilder::default().with_capacity(8);
        let (_tx0, rx0) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(8);
        let (_tx1, rx1) = b1.build_channel::<i32>();
        let bundle: SteadyRxBundle<i32, 2> = Arc::new([rx0.clone(), rx1.clone()]);
        let mut counts = [other_count, other_count];
        counts[lane] = 0;
        let idx = core_exec::block_on(bundle.wait_avail_index(&counts));
        prop_assert_eq!(idx, lane);
    }

    /// Property: lane_mask zero-bit lane wins wait_avail_index when channels are primed.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify verify.process.proptest]
    fn proptest_wait_avail_index_lane_mask(
        cap in 2usize..16,
        mask in lane_mask(2),
        per_lane in 1usize..4,
    ) {
        prop_assume!(per_lane <= cap);
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyRxBundleTrait;
        let b0 = ChannelBuilder::default().with_capacity(cap);
        let (tx0, rx0) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(cap);
        let (tx1, rx1) = b1.build_channel::<i32>();
        let bundle: SteadyRxBundle<i32, 2> = Arc::new([rx0.clone(), rx1.clone()]);
        let need = per_lane.min(cap);
        let mut counts = [need, need];
        if mask & 1 == 0 {
            counts[0] = 0;
        }
        if mask & 2 == 0 {
            counts[1] = 0;
        }
        if counts[0] > 0 && mask & 1 != 0 {
            tx0.testing_send_all(vec![1i32; counts[0]], false);
        }
        if counts[1] > 0 && mask & 2 != 0 {
            tx1.testing_send_all(vec![2i32; counts[1]], false);
        }
        let idx = core_exec::block_on(bundle.wait_avail_index(&counts));
        if counts[0] == 0 {
            prop_assert_eq!(idx, 0);
        } else if counts[1] == 0 {
            prop_assert_eq!(idx, 1);
        } else {
            prop_assert!(idx < 2);
        }
    }

    /// Property: lane_mask zero-bit lane wins wait_vacant_index immediately.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify verify.process.proptest]
    fn proptest_wait_vacant_index_lane_mask(
        mask in lane_mask(2),
        per_lane in 1usize..4,
    ) {
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyTxBundleTrait;
        let b0 = ChannelBuilder::default().with_capacity(8);
        let (tx0, _rx0) = b0.build_channel::<i32>();
        let b1 = ChannelBuilder::default().with_capacity(8);
        let (tx1, _rx1) = b1.build_channel::<i32>();
        let bundle: SteadyTxBundle<i32, 2> = Arc::new([tx0.clone(), tx1.clone()]);
        let mut counts = [per_lane, per_lane];
        if mask & 1 == 0 {
            counts[0] = 0;
        }
        if mask & 2 == 0 {
            counts[1] = 0;
        }
        let idx = core_exec::block_on(bundle.wait_vacant_index(&counts));
        if counts[0] == 0 {
            prop_assert_eq!(idx, 0);
        } else if counts[1] == 0 {
            prop_assert_eq!(idx, 1);
        } else {
            prop_assert!(idx < 2);
        }
    }

    /// Property: mark_closed through SteadyTx is idempotent and preserves vacant count.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_mark_closed_vacant_unchanged(
        cap in crate::proptest_support::capacity(),
        messages in message_vec::<u8>(),
    ) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u8>();
        let to_send: Vec<u8> = messages.into_iter().take(cap.saturating_sub(1)).collect();
        if !to_send.is_empty() {
            tx_lazy.testing_send_all(to_send, false);
        }
        let tx = tx_lazy.clone();
        core_exec::block_on(async {
            let mut ste_tx = tx.lock().await;
            let vacant_before = ste_tx.shared_vacant_units();
            ste_tx.shared_mark_closed();
            ste_tx.shared_mark_closed();
            prop_assert_eq!(ste_tx.shared_vacant_units(), vacant_before);
            Ok::<(), TestCaseError>(())
        })
        .expect("async property");
    }

    /// Property: testing_send_all delivery preserves FIFO order on the receiver.
    #[test]
    // ss[verify channel.testing-take-all]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_tx_fifo_via_harness(
        cap in capacity(),
        messages in message_vec::<u32>(),
    ) {
        let messages: Vec<u32> = messages.into_iter().take(cap).collect();
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u32>();
        tx_lazy.testing_send_all(messages.clone(), false);
        let taken = channel_fifo_take(cap, messages.clone());
        prop_assert_eq!(taken, messages);
    }

    /// Property: Tx metadata helpers expose stable id and capacity.
    #[test]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_tx_id_capacity_and_metadata(cap in capacity()) {
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
        let tx = tx_lazy.clone();
        core_exec::block_on(async {
            let ste_tx = tx.lock().await;
            prop_assert_eq!(ste_tx.capacity(), cap);
            let id = ste_tx.id();
            prop_assert_eq!(ste_tx.meta_data().len(), 1);
            prop_assert_eq!(ste_tx.meta_data()[0].meta_data().id, id);
            let debug = format!("{:?}", &*ste_tx);
            prop_assert!(debug.contains("Tx"));
            Ok::<(), TestCaseError>(())
        })
        .expect("async property");
    }

    /// Property: width() and memory_bytes() match build-time metadata for Tx.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify verify.process.proptest]
    fn proptest_tx_width_and_memory_bytes(cap in capacity()) {
        // ss[related philosophy.structural-hierarchy]
        use std::mem::size_of;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, _rx_lazy) = builder.build_channel::<u64>();
        let tx = tx_lazy.clone();
        core_exec::block_on(async {
            let ste_tx = tx.lock().await;
            let expected_width = size_of::<u64>();
            prop_assert_eq!(ste_tx.width(), expected_width);
            prop_assert_eq!(ste_tx.capacity(), cap);
            prop_assert_eq!(ste_tx.memory_bytes(), cap * expected_width);
            Ok::<(), TestCaseError>(())
        })
        .expect("async property");
    }

    /// Property: bundle memory_bytes() equals the sum of per-lane capacity × width.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify verify.process.proptest]
    fn proptest_tx_bundle_memory_bytes_is_lane_sum(
        caps in prop::collection::vec(capacity(), 4),
    ) {
        // ss[related channel.backpressure-never-drop]
        use crate::SteadyTxBundleTrait;
        // ss[related philosophy.structural-hierarchy]
        use std::mem::size_of;
        let b0 = ChannelBuilder::default().with_capacity(caps[0]);
        let (tx0, _rx0) = b0.build_channel::<u64>();
        let b1 = ChannelBuilder::default().with_capacity(caps[1]);
        let (tx1, _rx1) = b1.build_channel::<u64>();
        let b2 = ChannelBuilder::default().with_capacity(caps[2]);
        let (tx2, _rx2) = b2.build_channel::<u64>();
        let b3 = ChannelBuilder::default().with_capacity(caps[3]);
        let (tx3, _rx3) = b3.build_channel::<u64>();
        let bundle: SteadyTxBundle<u64, 4> =
            Arc::new([tx0.clone(), tx1.clone(), tx2.clone(), tx3.clone()]);
        let expected = caps.iter().map(|c| c * size_of::<u64>()).sum::<usize>();
        prop_assert_eq!(bundle.memory_bytes(), expected);
    }
}
