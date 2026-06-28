// ss[impl actor.lock-first.channels]
use std::time::Duration;
use crate::{RxCore, RxDone};
use crate::monitor_telemetry::SteadyTelemetrySend;

// ss[impl actor.lock-first.channels]
/// Implementation of `RxCore` for `futures_util::lock::MutexGuard<'_, T>` where `T: RxCore`.
///
/// This implementation forwards all `RxCore` method calls to the underlying `T`, enabling
/// reception operations on a channel protected by a mutex lock.
// ss[impl actor.lock-first.channels]
impl<T: RxCore> RxCore for futures_util::lock::MutexGuard<'_, T> {
    /// Inherits the message item type from the underlying `T`.
    type MsgItem = <T as RxCore>::MsgItem;

    /// Inherits the output message type from the underlying `T`.
    // ss[impl actor.lock-first.channels]
    type MsgOut = <T as RxCore>::MsgOut;

    /// Inherits the peek type from the underlying `T`.
    // ss[impl actor.lock-first.channels]
    type MsgPeek<'a> = <T as RxCore>::MsgPeek<'a> where Self: 'a;

    /// Inherits the message size type from the underlying `T`.
    // ss[impl actor.lock-first.channels]
    type MsgSize = <T as RxCore>::MsgSize;

    /// Inherits the slice source type from the underlying `T`.
    // ss[impl actor.lock-first.channels]
    type SliceSource<'a> = <T as RxCore>::SliceSource<'a> where Self: 'a;

    /// Inherits the slice target type from the underlying `T`.
    // ss[impl actor.lock-first.channels]
    type SliceTarget<'b> = <T as RxCore>::SliceTarget<'b> where Self::MsgOut: 'b;

    fn shared_validate_capacity_items(&self, items_count: usize) -> usize {
        <T as RxCore>::shared_validate_capacity_items(& **self, items_count)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_avail_items_count(&mut self) -> usize {
        <T as RxCore>::shared_avail_items_count(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn is_closed_and_empty(&mut self) -> bool {
        <T as RxCore>::is_closed_and_empty(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>> {
        <T as RxCore>::shared_peek_async_timeout(&mut **self, timeout).await
    }

    // ss[impl actor.lock-first.channels]
    fn log_periodic(&mut self) -> bool {
        <T as RxCore>::log_periodic(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as RxCore>::telemetry_inc(&mut **self, done_count, tel)
    }

    // ss[impl actor.lock-first.channels]
    fn monitor_not(&mut self) {
        <T as RxCore>::monitor_not(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_capacity(&self) -> Self::MsgSize {
        <T as RxCore>::shared_capacity(&**self)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_capacity_for(&**self, size)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_is_empty(&self) -> bool {
        <T as RxCore>::shared_is_empty(&**self)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_avail_units(&mut self) -> Self::MsgSize {
        <T as RxCore>::shared_avail_units(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_avail_units_for(&mut self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_avail_units_for(&mut **self, size)
    }

    // ss[impl actor.lock-first.channels]
    async fn shared_wait_shutdown_or_avail_units(&mut self, size: T::MsgSize) -> bool {
        <T as RxCore>::shared_wait_shutdown_or_avail_units(&mut **self, size).await
    }

    // ss[impl actor.lock-first.channels]
    async fn shared_wait_closed_or_avail_units(&mut self, size:usize) -> bool {
        <T as RxCore>::shared_wait_closed_or_avail_units(&mut **self, size).await
    }

    // ss[impl actor.lock-first.channels]
    async fn shared_wait_avail_units(&mut self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_wait_avail_units(&mut **self, size).await
    }

    // ss[impl actor.lock-first.channels]
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        <T as RxCore>::shared_try_take(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {
        <T as RxCore>::shared_advance_index(&mut **self, count)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        <T as RxCore>::shared_take_slice(&mut **self, target)
    }

    // ss[impl actor.lock-first.channels]
    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_> {
        <T as RxCore>::shared_peek_slice(&mut **self)
    }

    // ss[impl actor.lock-first.channels]
    fn one(&self) -> Self::MsgSize {
        <T as RxCore>::one(& **self)
    }
}

#[cfg(test)]
// ss[impl actor.lock-first.channels]
mod tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    // ss[impl actor.lock-first.channels]
    use crate::core_exec;

    // ss[verify actor.lock-first.channels]
    // ss[verify channel.internal-behavior-no-lazy]
    #[test]
    fn mutex_guard_forwards_avail_items_count() {
        let builder = ChannelBuilder::default().with_capacity(4);
        let (tx, rx) = builder.build_channel::<u32>();
        tx.testing_send_all(vec![1, 2], false);
        let rx_est = rx.clone();
        core_exec::block_on(async {
            let mut guard = rx_est.lock().await;
            assert_eq!(guard.shared_avail_items_count(), 2);
        });
    }

    use proptest::prelude::*;
    use crate::proptest_support::{capacity, channel_fifo_take, message_vec};

    ss_proptest! {

        /// Property: MutexGuard Rx drain preserves FIFO order.
        #[test]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.internal-behavior-no-lazy]
        // ss[verify channel.testing-take-all]
        // ss[verify verify.process.proptest]
        fn proptest_guard_fifo_matches_channel(
            cap in capacity(),
            messages in message_vec::<u32>(),
        ) {
            let messages: Vec<u32> = messages.into_iter().take(cap).collect();
            let expected = channel_fifo_take(cap, messages.clone());
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx, rx) = builder.build_channel::<u32>();
            tx.testing_send_all(messages, false);
            let rx_est = rx.clone();
            core_exec::block_on(async {
                let mut guard = rx_est.lock().await;
                prop_assert_eq!(guard.shared_avail_items_count(), expected.len());
                let mut taken = Vec::new();
                while let Some((_, v)) = guard.shared_try_take() {
                    taken.push(v);
                }
                prop_assert_eq!(taken, expected);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: MutexGuard peek_slice does not change avail_units.
        #[test]
        // ss[verify philosophy.zero-copy-discipline]
        // ss[verify actor.lock-first.channels]
        // ss[verify verify.process.proptest]
        fn proptest_guard_peek_preserves_avail(
            cap in 2usize..32,
            messages in message_vec::<i32>(),
        ) {
            let messages: Vec<i32> = messages.into_iter().take(cap).collect();
            prop_assume!(!messages.is_empty());
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx, rx) = builder.build_channel::<i32>();
            tx.testing_send_all(messages, false);
            let rx_est = rx.clone();
            core_exec::block_on(async {
                let mut guard = rx_est.lock().await;
                let avail_before = guard.shared_avail_units();
                let (a, b) = guard.shared_peek_slice();
                prop_assert_eq!(a.len() + b.len(), avail_before);
                prop_assert_eq!(guard.shared_avail_units(), avail_before);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: MutexGuard advance_index never exceeds avail_units.
        #[test]
        // ss[verify philosophy.zero-copy-discipline]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_guard_advance_index_bounded(
            cap in 2usize..32,
            messages in message_vec::<i32>(),
            take_count in 1usize..32,
        ) {
            let messages: Vec<i32> = messages.into_iter().take(cap).collect();
            prop_assume!(!messages.is_empty());
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx, rx) = builder.build_channel::<i32>();
            tx.testing_send_all(messages, false);
            let rx_est = rx.clone();
            core_exec::block_on(async {
                let mut guard = rx_est.lock().await;
                let avail = guard.shared_avail_units();
                let done = guard.shared_advance_index(take_count);
                prop_assert!(done.item_count() <= avail);
                prop_assert!(done.item_count() <= take_count);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }
    }
}