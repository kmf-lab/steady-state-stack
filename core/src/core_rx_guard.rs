use std::time::Duration;
use crate::{RxCore, RxDone};
use crate::monitor_telemetry::SteadyTelemetrySend;

/// Implementation of `RxCore` for `futures_util::lock::MutexGuard<'_, T>` where `T: RxCore`.
///
/// This implementation forwards all `RxCore` method calls to the underlying `T`, enabling
/// reception operations on a channel protected by a mutex lock.
impl<T: RxCore> RxCore for futures_util::lock::MutexGuard<'_, T> {
    /// Inherits the message item type from the underlying `T`.
    type MsgItem = <T as RxCore>::MsgItem;

    /// Inherits the output message type from the underlying `T`.
    type MsgOut = <T as RxCore>::MsgOut;

    /// Inherits the peek type from the underlying `T`.
    type MsgPeek<'a> = <T as RxCore>::MsgPeek<'a> where Self: 'a;

    /// Inherits the message size type from the underlying `T`.
    type MsgSize = <T as RxCore>::MsgSize;

    /// Inherits the slice source type from the underlying `T`.
    type SliceSource<'a> = <T as RxCore>::SliceSource<'a> where Self: 'a;

    /// Inherits the slice target type from the underlying `T`.
    type SliceTarget<'b> = <T as RxCore>::SliceTarget<'b> where Self::MsgOut: 'b;

    fn shared_validate_capacity_items(&self, items_count: usize) -> usize {
        <T as RxCore>::shared_validate_capacity_items(& **self, items_count)
    }

    fn shared_avail_items_count(&mut self) -> usize {
        <T as RxCore>::shared_avail_items_count(&mut **self)
    }

    fn is_closed_and_empty(&mut self) -> bool {
        <T as RxCore>::is_closed_and_empty(&mut **self)
    }

    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>> {
        <T as RxCore>::shared_peek_async_timeout(&mut **self, timeout).await
    }

    fn log_periodic(&mut self) -> bool {
        <T as RxCore>::log_periodic(&mut **self)
    }

    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        <T as RxCore>::telemetry_inc(&mut **self, done_count, tel)
    }

    fn monitor_not(&mut self) {
        <T as RxCore>::monitor_not(&mut **self)
    }

    fn shared_capacity(&self) -> Self::MsgSize {
        <T as RxCore>::shared_capacity(&**self)
    }

    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_capacity_for(&**self, size)
    }

    fn shared_is_empty(&self) -> bool {
        <T as RxCore>::shared_is_empty(&**self)
    }

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        <T as RxCore>::shared_avail_units(&mut **self)
    }

    fn shared_avail_units_for(&mut self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_avail_units_for(&mut **self, size)
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, size: T::MsgSize) -> bool {
        <T as RxCore>::shared_wait_shutdown_or_avail_units(&mut **self, size).await
    }

    async fn shared_wait_closed_or_avail_units(&mut self, size:usize) -> bool {
        <T as RxCore>::shared_wait_closed_or_avail_units(&mut **self, size).await
    }

    async fn shared_wait_avail_units(&mut self, size: Self::MsgSize) -> bool {
        <T as RxCore>::shared_wait_avail_units(&mut **self, size).await
    }

    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        <T as RxCore>::shared_try_take(&mut **self)
    }

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {
        <T as RxCore>::shared_advance_index(&mut **self, count)
    }

    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        <T as RxCore>::shared_take_slice(&mut **self, target)
    }

    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_> {
        <T as RxCore>::shared_peek_slice(&mut **self)
    }

    fn one(&self) -> Self::MsgSize {
        <T as RxCore>::one(& **self)
    }
}