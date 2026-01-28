use futures_util::lock::MutexGuard;
use std::time::Duration;
use crate::{ActorIdentity, SendOutcome, SendSaturation, TxCore, TxDone};
use crate::monitor_telemetry::SteadyTelemetrySend;

/// Implementation of `TxCore` for `MutexGuard<'_, T>` where `T: TxCore`.
///
/// This implementation forwards all `TxCore` method calls to the underlying `T`, enabling
/// transmission operations on a channel protected by a mutex lock.
impl<T: TxCore> TxCore for MutexGuard<'_, T> {
    /// Inherits the input message type from the underlying `T`.
    type MsgIn<'a> = <T as TxCore>::MsgIn<'a>;

    /// Inherits the output message type from the underlying `T`.
    type MsgOut = <T as TxCore>::MsgOut;

    /// Inherits the message size type from the underlying `T`.
    type MsgSize = <T as TxCore>::MsgSize;

    /// Inherits the slice source type from the underlying `T`.
    type SliceSource<'b> = <T as TxCore>::SliceSource<'b> where Self::MsgOut: 'b;

    /// Inherits the slice target type from the underlying `T`.
    type SliceTarget<'a> = <T as TxCore>::SliceTarget<'a> where Self: 'a;

    /// Forwards the advance index operation to the underlying `T`.
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {
        <T as TxCore>::shared_advance_index(&mut **self, count)
    }

    /// Forwards the mark closed operation to the underlying `T`.
    fn shared_mark_closed(&mut self) {
        <T as TxCore>::shared_mark_closed(&mut **self)
    }

    /// Forwards the unit value retrieval to the underlying `T`.
    fn one(&self) -> Self::MsgSize {
        <T as TxCore>::one(&**self)
    }

    /// Forwards the periodic logging check to the underlying `T`.
    fn log_perodic(&mut self) -> bool {
        <T as TxCore>::log_perodic(&mut **self)
    }

    /// Forwards the telemetry increment operation to the underlying `T`.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as TxCore>::telemetry_inc(&mut **self, done_count, tel)
    }

    /// Forwards the iterator send operation to the underlying `T`.
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        <T as TxCore>::shared_send_iter_until_full(&mut **self, iter)
    }

    /// Forwards the monitor notification to the underlying `T`.
    fn monitor_not(&mut self) {
        <T as TxCore>::monitor_not(&mut **self)
    }

    /// Forwards the capacity retrieval to the underlying `T`.
    #[inline]
    fn shared_capacity(&self) -> Self::MsgSize {
        <T as TxCore>::shared_capacity(&**self)
    }

    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        <T as TxCore>::shared_capacity_for(&**self, size)
    }

    /// Forwards the full check to the underlying `T`.
    #[inline]
    fn shared_is_full(&self) -> bool {
        <T as TxCore>::shared_is_full(&**self)
    }

    /// Forwards the empty check to the underlying `T`.
    #[inline]
    fn shared_is_empty(&self) -> bool {
        <T as TxCore>::shared_is_empty(&**self)
    }

    /// Forwards the vacant units retrieval to the underlying `T`.
    #[inline]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        <T as TxCore>::shared_vacant_units(&**self)
    }
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        <T as TxCore>::shared_vacant_units_for(&**self,size)
    }

    /// Forwards the shutdown or vacant wait to the underlying `T`.
    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_shutdown_or_vacant_units(&mut **self, count).await
    }

    /// Forwards the vacant units wait to the underlying `T`.
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        <T as TxCore>::shared_wait_vacant_units(&mut **self, count).await
    }

    /// Forwards the empty wait to the underlying `T`.
    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        <T as TxCore>::shared_wait_empty(&mut **self).await
    }

    /// Forwards the slice send operation to the underlying `T`.
    #[inline]
    fn shared_send_slice(&mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {
        <T as TxCore>::shared_send_slice(self, slice)
    }

    /// Forwards the slice poke operation to the underlying `T`.
    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        <T as TxCore>::shared_poke_slice(&mut **self)
    }

    /// Forwards the try send operation to the underlying `T`.
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        <T as TxCore>::shared_try_send(&mut **self, msg)
    }

    /// Forwards the core async send operation to the underlying `T`.
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
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        <T as TxCore>::shared_send_async(&mut **self, msg, ident, saturation).await
    }

    /// Forwards the done one operation to the underlying `T`.
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        <T as TxCore>::done_one(self, one)
    }
}
