// ss[related actor.lock-first.channels]
use futures_util::lock::MutexGuard;
// ss[related philosophy.structural-hierarchy]
use std::time::Duration;
// ss[related philosophy.structural-hierarchy]
use crate::{ActorIdentity, SendOutcome, SendSaturation, TxCore, TxDone};
// ss[related actor.lock-first.channels]
use crate::monitor_telemetry::SteadyTelemetrySend;

/// Implementation of `TxCore` for `MutexGuard<'_, T>` where `T: TxCore`.
///
/// This implementation forwards all `TxCore` method calls to the underlying `T`, enabling
/// transmission operations on a channel protected by a mutex lock.
// ss[related actor.lock-first.channels]
impl<T: TxCore> TxCore for MutexGuard<'_, T> {
    /// Inherits the input message type from the underlying `T`.
    // ss[related philosophy.structural-hierarchy]
    type MsgIn<'a> = <T as TxCore>::MsgIn<'a>;

    /// Inherits the output message type from the underlying `T`.
    // ss[related actor.lock-first.channels]
    type MsgOut = <T as TxCore>::MsgOut;

    /// Inherits the message size type from the underlying `T`.
    // ss[related actor.lock-first.channels]
    type MsgSize = <T as TxCore>::MsgSize;

    /// Inherits the slice source type from the underlying `T`.
    // ss[related actor.lock-first.channels]
    type SliceSource<'b> = <T as TxCore>::SliceSource<'b> where Self::MsgOut: 'b;

    /// Inherits the slice target type from the underlying `T`.
    // ss[related actor.lock-first.channels]
    type SliceTarget<'a> = <T as TxCore>::SliceTarget<'a> where Self: 'a;

    /// Forwards the advance index operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {
        <T as TxCore>::shared_advance_index(&mut **self, count)
    }

    /// Forwards the mark closed operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn shared_mark_closed(&mut self) {
        <T as TxCore>::shared_mark_closed(&mut **self)
    }

    /// Forwards the unit value retrieval to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn one(&self) -> Self::MsgSize {
        <T as TxCore>::one(&**self)
    }

    /// Forwards the periodic logging check to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn log_perodic(&mut self) -> bool {
        <T as TxCore>::log_perodic(&mut **self)
    }

    /// Forwards the telemetry increment operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as TxCore>::telemetry_inc(&mut **self, done_count, tel)
    }

    /// Forwards the iterator send operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        <T as TxCore>::shared_send_iter_until_full(&mut **self, iter)
    }

    /// Forwards the monitor notification to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn monitor_not(&mut self) {
        <T as TxCore>::monitor_not(&mut **self)
    }

    /// Forwards the capacity retrieval to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_capacity(&self) -> Self::MsgSize {
        <T as TxCore>::shared_capacity(&**self)
    }

    // ss[related actor.lock-first.channels]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        <T as TxCore>::shared_capacity_for(&**self, size)
    }

    /// Forwards the full check to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_is_full(&self) -> bool {
        <T as TxCore>::shared_is_full(&**self)
    }

    /// Forwards the empty check to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_is_empty(&self) -> bool {
        <T as TxCore>::shared_is_empty(&**self)
    }

    /// Forwards the vacant units retrieval to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        <T as TxCore>::shared_vacant_units(&**self)
    }
    // ss[related actor.lock-first.channels]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        <T as TxCore>::shared_vacant_units_for(&**self,size)
    }

    /// Forwards the shutdown or vacant wait to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_shutdown_or_vacant_units(&mut **self, count).await
    }

    /// Forwards the vacant units wait to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_vacant_units(&mut **self, count).await
    }

    /// Forwards the empty wait to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    async fn shared_wait_empty(&mut self) -> bool {
        <T as TxCore>::shared_wait_empty(&mut **self).await
    }

    /// Forwards the slice send operation to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_send_slice(&mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {
        <T as TxCore>::shared_send_slice(self, slice)
    }

    /// Forwards the slice poke operation to the underlying `T`.
    #[inline]
    // ss[related actor.lock-first.channels]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        <T as TxCore>::shared_poke_slice(&mut **self)
    }

    /// Forwards the try send operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        <T as TxCore>::shared_try_send(&mut **self, msg)
    }

    /// Forwards the core async send operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async_core(&mut **self, msg, ident, saturation, timeout).await
    }

    /// Forwards the async send with timeout to the underlying `T`.
    // ss[related actor.lock-first.channels]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async_timeout(&mut **self, msg, ident, saturation, timeout).await
    }

    /// Forwards the async send without timeout to the underlying `T`.
    // ss[related actor.lock-first.channels]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async(&mut **self, msg, ident, saturation).await
    }

    /// Forwards the done one operation to the underlying `T`.
    // ss[related actor.lock-first.channels]
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        <T as TxCore>::done_one(self, one)
    }
}

#[cfg(test)]
// ss[related actor.lock-first.channels]
mod core_tx_guard_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::channel_builder::ChannelBuilder;
    // ss[related actor.lock-first.channels]
    use crate::core_rx::RxCore;
    // ss[related philosophy.structural-hierarchy]
    use crate::core_tx::TxCore;
    // ss[related philosophy.structural-hierarchy]
    use crate::core_exec;
    // ss[related actor.lock-first.channels]
    use proptest::prelude::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::proptest_support::{capacity, message_vec};

    ss_proptest! {

        /// Property: MutexGuard vacant_units matches direct channel accounting.
        #[test]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_guard_vacant_matches_direct(
            cap in capacity(),
            n in 0usize..64,
        ) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, _rx_lazy) = builder.build_channel::<u8>();
            let send_n = n.min(cap);
            if send_n > 0 {
                tx_lazy.testing_send_all(vec![0u8; send_n], false);
            }
            let tx_est = tx_lazy.clone();
            core_exec::block_on(async {
                let guard = tx_est.lock().await;
                let expected_vacant = cap.saturating_sub(send_n);
                prop_assert_eq!(guard.shared_vacant_units(), expected_vacant);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: MutexGuard try_send never exceeds channel capacity.
        #[test]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_guard_try_send_bounded(
            cap in 2usize..32,
            extra in 1usize..16,
        ) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, _rx_lazy) = builder.build_channel::<u8>();
            let tx_est = tx_lazy.clone();
            core_exec::block_on(async {
                let mut guard = tx_est.lock().await;
                let mut sent = 0usize;
                for i in 0..(cap + extra) {
                    if guard.shared_try_send(i as u8).is_ok() {
                        sent += 1;
                    } else {
                        break;
                    }
                }
                prop_assert_eq!(sent, cap);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: MutexGuard mark_closed is idempotent and preserves vacant count.
        #[test]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_guard_mark_closed_idempotent(
            cap in capacity(),
            messages in message_vec::<u8>(),
        ) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, _rx_lazy) = builder.build_channel::<u8>();
            let to_send: Vec<u8> = messages.into_iter().take(cap.saturating_sub(1)).collect();
            if !to_send.is_empty() {
                tx_lazy.testing_send_all(to_send, false);
            }
            let tx_est = tx_lazy.clone();
            core_exec::block_on(async {
                let mut guard = tx_est.lock().await;
                let vacant_before = guard.shared_vacant_units();
                guard.shared_mark_closed();
                guard.shared_mark_closed();
                prop_assert_eq!(guard.shared_vacant_units(), vacant_before);
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }

        /// Property: MutexGuard send then drain preserves FIFO (no silent drop).
        #[test]
        // ss[verify actor.lock-first.channels]
        // ss[verify channel.testing-take-all]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_guard_no_silent_drop(
            cap in capacity(),
            messages in message_vec::<u8>(),
        ) {
            let builder = ChannelBuilder::default().with_capacity(cap);
            let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();
            let to_send: Vec<u8> = messages.into_iter().take(cap).collect();
            let tx_est = tx_lazy.clone();
            let rx_est = rx_lazy.clone();
            core_exec::block_on(async {
                let mut guard = tx_est.lock().await;
                let mut sent = 0usize;
                for &msg in &to_send {
                    if guard.shared_try_send(msg).is_ok() {
                        sent += 1;
                    } else {
                        break;
                    }
                }
                drop(guard);
                let mut rx_guard = rx_est.lock().await;
                let mut taken = Vec::new();
                while let Some((_, v)) = rx_guard.shared_try_take() {
                    taken.push(v);
                }
                prop_assert_eq!(taken, to_send.into_iter().take(sent).collect::<Vec<_>>());
                Ok::<(), TestCaseError>(())
            })
            .expect("async property");
        }
    }
}
