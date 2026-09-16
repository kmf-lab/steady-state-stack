//! Tests extracted from `steady_rx.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related philosophy.zero-copy-discipline]
// --- from mod rx_tests ---
// ss[related philosophy.structural-hierarchy]
use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related philosophy.zero-copy-discipline]
use super::*;
// ss[related philosophy.structural-hierarchy]
use crate::*;

// ss[verify bundle.index-wait-readiness]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_bundle() {
    let mut graph = GraphBuilder::for_testing().build(());
    let channel_builder = graph.channel_builder();
    let (lazy_tx_bundle, lazy_rx_bundle) = channel_builder.build_channel_bundle::<String, 3>();
    let (steady_tx_bundle0, steady_rx_bundle0) = (lazy_tx_bundle[0].clone(), lazy_rx_bundle[0].clone());
    let (steady_tx_bundle1, steady_rx_bundle1) = (lazy_tx_bundle[1].clone(), lazy_rx_bundle[1].clone());
    let (steady_tx_bundle2, steady_rx_bundle2) = (lazy_tx_bundle[2].clone(), lazy_rx_bundle[2].clone());

    let (steady_tx_bundle, steady_rx_bundle) = (
        SteadyTxBundle::new([steady_tx_bundle0, steady_tx_bundle1, steady_tx_bundle2]),
        SteadyRxBundle::new([steady_rx_bundle0, steady_rx_bundle1, steady_rx_bundle2]),
    );

    let array_tx_meta_data = steady_tx_bundle.meta_data();
    let array_rx_meta_data = steady_rx_bundle.meta_data();
    assert_eq!(array_rx_meta_data[0].meta_data().id, array_tx_meta_data[0].meta_data().id);
    assert_eq!(array_rx_meta_data[1].meta_data().id, array_tx_meta_data[1].meta_data().id);
    assert_eq!(array_rx_meta_data[2].meta_data().id, array_tx_meta_data[2].meta_data().id);

    let mut vec_tx_bundle = core_exec::block_on(steady_tx_bundle.lock());
    assert!(vec_tx_bundle[0].shared_try_send("0".to_string()).is_ok());
    assert!(vec_tx_bundle[1].shared_try_send("1".to_string()).is_ok());
    assert!(vec_tx_bundle[2].shared_try_send("2".to_string()).is_ok());

    let mut vec_rx_bundle = core_exec::block_on(async {
        steady_rx_bundle.wait_avail_units(1, 3).await;
        steady_rx_bundle.lock().await
    });
    assert_eq!(3, vec_rx_bundle.len());
    assert!(!RxBundleTrait::is_empty(&mut vec_rx_bundle));
    assert!(!RxBundleTrait::is_closed(&mut vec_rx_bundle));
    assert!(!RxBundleTrait::is_closed_and_empty(&mut vec_rx_bundle));

    vec_tx_bundle[0].mark_closed();
    vec_tx_bundle[1].mark_closed();
    vec_tx_bundle[2].mark_closed();

    assert!(RxBundleTrait::is_closed(&mut vec_rx_bundle));
    assert!(vec_rx_bundle[0].shared_try_take().is_some());
    assert!(vec_rx_bundle[1].shared_try_take().is_some());
    assert!(vec_rx_bundle[2].shared_try_take().is_some());

    assert!(RxBundleTrait::is_closed_and_empty(&mut vec_rx_bundle));
    assert!(RxBundleTrait::is_empty(&mut vec_rx_bundle));
}


// ss[related philosophy.zero-copy-discipline]
// --- from mod steady_rx_tests ---
// ss[related philosophy.structural-hierarchy]
use crate::channel_builder::ChannelBuilder;
// ss[related philosophy.structural-hierarchy]
use crate::proptest_support::{capacity, channel_fifo_take, lane_mask, message_vec};
// ss[related philosophy.zero-copy-discipline]
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;

ss_proptest! {

    /// Property: channel delivery preserves FIFO order (i32).
    #[test]
    // ss[verify channel.testing-take-all]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_channel_fifo_order_i32(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let taken = channel_fifo_take(cap, messages.clone());
        prop_assert_eq!(taken, messages);
    }

    /// Property: channel delivery preserves FIFO order (u64).
    #[test]
    // ss[verify channel.testing-take-all]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_channel_fifo_order_u64(
        cap in capacity(),
        messages in message_vec::<u64>(),
    ) {
        let messages: Vec<u64> = messages.into_iter().take(cap).collect();
        let taken = channel_fifo_take(cap, messages.clone());
        prop_assert_eq!(taken, messages);
    }

    /// Property: peek-before-take exposes the full queued sequence without mutation.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_peek_then_take_matches(
        cap in 2usize..32,
        messages in message_vec::<u32>(),
    ) {
        let messages: Vec<u32> = messages.into_iter().take(cap).collect();
        prop_assume!(!messages.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u32>();
        tx_lazy.testing_send_all(messages.clone(), false);
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        let peeked: Vec<_> = ste_rx.try_peek_iter().cloned().collect();
        prop_assert_eq!(peeked, messages);
    }

    /// Property: empty channel peek returns no items.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify verify.process.proptest]
    fn proptest_peek_empty_channel(cap in capacity()) {
        let taken = channel_fifo_take::<i64>(cap, vec![]);
        prop_assert!(taken.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx, rx_lazy) = builder.build_channel::<i64>();
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        prop_assert!(ste_rx.shared_try_peek().is_none());
    }

    /// Property: lane_mask zero-bit lane wins wait_avail_index immediately.
    #[test]
    // ss[verify bundle.index-wait-readiness]
    // ss[verify actor.index-wait-round-robin]
    // ss[verify verify.process.proptest]
    fn proptest_bundle_wait_avail_lane_mask(
        cap in 2usize..16,
        mask in lane_mask(2),
        per_lane in 1usize..4,
    ) {
        prop_assume!(per_lane <= cap);
        // ss[related philosophy.zero-copy-discipline]
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

    /// Property: try_peek_iter exposes the full queued FIFO sequence.
    #[test]
    // ss[verify philosophy.zero-copy-discipline]
    // ss[verify channel.testing-take-all]
    // ss[verify verify.process.proptest]
    fn proptest_peek_iter_len_matches_fifo(
        cap in capacity(),
        messages in message_vec::<i64>(),
    ) {
        let messages: Vec<i64> = messages.into_iter().take(cap).collect();
        prop_assume!(!messages.is_empty());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i64>();
        tx_lazy.testing_send_all(messages.clone(), false);
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        let peeked: Vec<_> = ste_rx.try_peek_iter().cloned().collect();
        prop_assert_eq!(peeked.len(), messages.len());
        prop_assert_eq!(peeked, messages);
    }

    /// Property: try_take drain matches channel_fifo_take harness.
    #[test]
    // ss[verify channel.testing-take-all]
    // ss[verify channel.backpressure-never-drop]
    // ss[verify verify.process.proptest]
    fn proptest_try_take_matches_fifo_harness(
        cap in capacity(),
        messages in message_vec::<i32>(),
    ) {
        let messages: Vec<i32> = messages.into_iter().take(cap).collect();
        let expected = channel_fifo_take(cap, messages.clone());
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(messages, false);
        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        let mut taken = Vec::new();
        while let Some(v) = ste_rx.try_take() {
            taken.push(v);
        }
        prop_assert_eq!(taken, expected);
    }

    /// Property: width() and memory_bytes() match build-time metadata for Rx.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify verify.process.proptest]
    fn proptest_rx_width_and_memory_bytes(cap in capacity()) {
        // ss[related philosophy.structural-hierarchy]
        use std::mem::size_of;
        let builder = ChannelBuilder::default().with_capacity(cap);
        let (_tx_lazy, rx_lazy) = builder.build_channel::<u64>();
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        let expected_width = size_of::<u64>();
        prop_assert_eq!(ste_rx.width(), expected_width);
        prop_assert_eq!(ste_rx.capacity(), cap);
        prop_assert_eq!(ste_rx.memory_bytes(), cap * expected_width);
    }

    /// Property: bundle memory_bytes() equals the sum of per-lane capacity × width.
    #[test]
    // ss[verify channel.memory-usage-telemetry]
    // ss[verify verify.process.proptest]
    fn proptest_rx_bundle_memory_bytes_is_lane_sum(
        caps in prop::collection::vec(capacity(), 4),
    ) {
        // ss[related philosophy.zero-copy-discipline]
        use crate::SteadyRxBundleTrait;
        // ss[related philosophy.structural-hierarchy]
        use std::mem::size_of;
        let b0 = ChannelBuilder::default().with_capacity(caps[0]);
        let (_tx0, rx0) = b0.build_channel::<u64>();
        let b1 = ChannelBuilder::default().with_capacity(caps[1]);
        let (_tx1, rx1) = b1.build_channel::<u64>();
        let b2 = ChannelBuilder::default().with_capacity(caps[2]);
        let (_tx2, rx2) = b2.build_channel::<u64>();
        let b3 = ChannelBuilder::default().with_capacity(caps[3]);
        let (_tx3, rx3) = b3.build_channel::<u64>();
        let bundle: SteadyRxBundle<u64, 4> =
            Arc::new([rx0.clone(), rx1.clone(), rx2.clone(), rx3.clone()]);
        let expected = caps.iter().map(|c| c * size_of::<u64>()).sum::<usize>();
        prop_assert_eq!(bundle.memory_bytes(), expected);
    }
}

/// Tests the `width()` and `memory_bytes()` accessors on an established Rx.
// ss[verify channel.memory-usage-telemetry]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_rx_width_and_memory_bytes() {
    let builder = ChannelBuilder::default().with_capacity(16);
    let (_tx_lazy, rx_lazy) = builder.build_channel::<u16>();
    let rx = rx_lazy.clone();
    let ste_rx = core_exec::block_on(rx.lock());
    assert_eq!(ste_rx.width(), 2, "u16 width must be 2 bytes");
    assert_eq!(ste_rx.capacity(), 16);
    assert_eq!(ste_rx.memory_bytes(), 32, "memory must be capacity × width");
}

/// Tests the bundle-level `memory_bytes()` rollup across lanes.
// ss[verify channel.memory-usage-telemetry]
#[test]
// ss[related philosophy.structural-hierarchy]
fn test_rx_bundle_memory_bytes() {
    // ss[related philosophy.structural-hierarchy]
    use crate::SteadyRxBundleTrait;
    let b0 = ChannelBuilder::default().with_capacity(4);
    let (_tx0, rx0) = b0.build_channel::<u64>();
    let b1 = ChannelBuilder::default().with_capacity(8);
    let (_tx1, rx1) = b1.build_channel::<u64>();
    let bundle: SteadyRxBundle<u64, 2> = Arc::new([rx0.clone(), rx1.clone()]);
    // lane0: 4 × 8 = 32, lane1: 8 × 8 = 64 → 96 total
    assert_eq!(bundle.memory_bytes(), 96);
}

