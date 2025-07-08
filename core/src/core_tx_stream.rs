use log::{error, trace, warn};
use std::time::{Duration, Instant};
use futures_util::{select, FutureExt};
use futures_util::future::{Either, FusedFuture};
use futures_timer::Delay;
use std::future::pending;
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use crate::{steady_config, ActorIdentity, SendOutcome, SendSaturation, StreamControlItem, StreamEgress, StreamIngress, StreamTx, TxCore, TxDone, MONITOR_NOT};
use crate::abstract_executor_async_std::core_exec;
use crate::loop_driver::pin_mut;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::yield_now::yield_now;

/// Implementation of `TxCore` for stream-based channels with `StreamIngress`.
///
/// This implementation manages a dual-channel system with a control channel for `StreamIngress`
/// items and a payload channel for byte data, ensuring synchronized transmission of control
/// messages and their associated payloads.
impl TxCore for StreamTx<StreamIngress> {
    /// The type of message sent into the channel, a tuple of a `StreamIngress` item and its payload bytes.
    type MsgIn<'a> = (StreamIngress, &'a [u8]);

    /// The type of message that comes out of the channel, the `StreamIngress` control item.
    type MsgOut = StreamIngress;

    /// The type used to count messages, a tuple of control items and payload bytes.
    type MsgSize = (usize, usize);

    /// The type for a slice of messages, a tuple of control item slices and payload byte slices.
    type SliceSource<'b> = (&'b [StreamIngress], &'b [u8]);

    /// The type for target slices, providing four mutable slices for control and payload buffers.
    type SliceTarget<'a> = (
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<u8>],
        &'a mut [std::mem::MaybeUninit<u8>],
    );

    /// Returns a `TxDone` value indicating one control item and its payload size were processed.
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Stream(1, one.1.len())
    }

    /// Advances the write indices for both control and payload channels if sufficient space exists.
    ///
    /// This method moves the write positions forward for both channels, returning the number of
    /// control items and bytes advanced, or zero if space is insufficient.
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {
        let control_avail = self.control_channel.tx.vacant_len();
        let payload_avail = self.payload_channel.tx.vacant_len();
        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.tx.advance_write_index(count.1);
                self.control_channel.tx.advance_write_index(count.0);
            }
            TxDone::Stream(count.0, count.1)
        } else {
            TxDone::Stream(0, 0)
        }
    }

    /// Marks both control and payload channels as closed.
    ///
    /// Sends closure signals through the oneshot channels for both control and payload, logging
    /// trace messages if the receivers are already dropped. Always returns `true`.
    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.control_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        }
        if let Some(c) = self.payload_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        }
        true
    }

    /// Returns a tuple representing one control item and an estimated payload size.
    ///
    /// The payload size is calculated as the ratio of payload channel capacity to control channel capacity.
    fn one(&self) -> Self::MsgSize {
        (1, self.payload_channel.capacity() / self.control_channel.capacity())
    }

    /// Checks if enough time has elapsed since the last error send for periodic logging.
    ///
    /// Uses the control channel’s timer, returning `true` if the interval exceeds the configured
    /// maximum telemetry error rate, resetting the timer.
    fn log_perodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    /// Sends control items and their payloads from an iterator until the channels are full.
    ///
    /// Processes items up to the control channel’s capacity, waiting synchronously for payload
    /// space if needed. Returns the number of control items sent. Includes debug assertions
    /// to ensure the channels are not closed.
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        let mut count = 0;
        let item_limit = self.control_channel.tx.vacant_len();
        let limited_iter = iter.take(item_limit);
        for (item, payload) in limited_iter {
            assert_eq!(item.length(), payload.len() as i32);
            if payload.len() > self.payload_channel.tx.vacant_len() {
                warn!("the payload of the stream should be larger we need {} but found {}", payload.len(), self.payload_channel.tx.vacant_len());
                core_exec::block_on(self.payload_channel.tx.wait_vacant(payload.len()));
            }
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(item);
            count += 1;
        }
        count
    }

    /// Increments telemetry for both control and payload channels.
    ///
    /// Updates telemetry with the number of control items and payload bytes sent, logging a
    /// warning if a `Normal` value is received instead of the expected `Stream`.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(i) => {
                warn!("internal error should have gotten Stream");
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            }
            TxDone::Stream(i, p) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.meta_data.id, p as isize);
            }
        }
    }

    /// Disables monitoring for both control and payload channels.
    ///
    /// Sets the monitor indices to a predefined constant to stop monitoring activity.
    #[inline]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the capacity of the control channel.
    #[inline]
    fn shared_capacity(&self) -> Self::MsgSize {
        (self.control_channel.tx.capacity().get(),self.payload_channel.tx.capacity().get())
    }
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the control channel is full.
    #[inline]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    /// Checks if the control channel is empty.
    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    /// Returns the number of vacant units in the control channel.
    #[inline]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        (self.control_channel.tx.vacant_len(), self.payload_channel.tx.vacant_len())
    }

    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }

    /// Waits for either shutdown or for the specified units to become vacant in both channels.
    ///
    /// Returns `true` if both channels have sufficient space or are empty, otherwise waits
    /// asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if (self.control_channel.tx.is_empty() || self.control_channel.tx.vacant_len() >= count.0) &&
            (self.payload_channel.tx.is_empty() || self.payload_channel.tx.vacant_len() >= count.1) {
            true
        } else {
            let icap = self.control_channel.capacity();
            let pcap = self.payload_channel.capacity();
            let mut one_down = &mut self.control_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let operation = async {
                    self.control_channel.tx.wait_vacant(count.0.min(icap)).await;
                    self.payload_channel.tx.wait_vacant(count.1.min(pcap)).await;
                };
                select! { _ = one_down => false, _ = operation.fuse() => true, }
            } else {
                yield_now().await;
                false
            }
        }
    }

    /// Waits until the specified units become vacant in both channels.
    ///
    /// Returns `true` immediately if enough space exists, otherwise waits asynchronously until
    /// both control and payload channels have the required vacant units.
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.control_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            self.control_channel.tx.wait_vacant(count.0.min(self.control_channel.capacity())).await;
            self.payload_channel.tx.wait_vacant(count.1.min(self.payload_channel.capacity())).await;
            true
        }
    }

    /// Waits for the control channel to become empty or for a shutdown signal.
    ///
    /// Returns `true` if the control channel empties, or `false` if shutdown occurs first.
    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.tx.wait_vacant(usize::from(self.control_channel.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.control_channel.tx.capacity().get() == self.control_channel.tx.vacant_len()
        }
    }

    /// Sends a slice of control items and their corresponding payload bytes.
    ///
    /// Attempts to send as many items as possible, limited by the vacant space in both channels,
    /// returning the number of control items and bytes sent.
    #[inline]
    fn shared_send_slice(&mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {
        let mut items_sent = 0;
        let mut bytes_sent = 0;
        let (ingress_items, payload_bytes) = slice;
        if !ingress_items.is_empty() {
            let item_vacant = self.control_channel.tx.vacant_len();
            let payload_vacant = self.payload_channel.tx.vacant_len();
            for &ingress in ingress_items.iter().take(item_vacant) {
                let len = ingress.length() as usize;
                if bytes_sent + len > payload_bytes.len() || bytes_sent + len > payload_vacant {
                    break;
                }
                let payload_chunk = &payload_bytes[bytes_sent..bytes_sent + len];
                let pushed = self.payload_channel.tx.push_slice(payload_chunk);
                if pushed != len {
                    break;
                }
                if self.control_channel.tx.try_push(ingress).is_err() {
                    break;
                }
                items_sent += 1;
                bytes_sent += len;
            }
        }
        TxDone::Stream(items_sent, bytes_sent)
    }

    /// Provides access to vacant slices in both control and payload channels for zero-copy writing.
    ///
    /// Returns four mutable slices representing the writable portions of both buffers.
    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        let (item_a, item_b) = self.control_channel.tx.vacant_slices_mut();
        let (payload_a, payload_b) = self.payload_channel.tx.vacant_slices_mut();
        (item_a, item_b, payload_a, payload_b)
    }

    /// Attempts to send a single control item and its payload without blocking.
    ///
    /// Returns `Ok` with the number of items and bytes sent if successful, or `Err` with the
    /// control item if either channel lacks sufficient space. Includes debug assertions for closure.
    #[inline]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        let (item, payload) = msg;
        assert_eq!(item.length(), payload.len() as i32);
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        if self.payload_channel.tx.vacant_len() >= item.length() as usize &&
            self.control_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(item);
            Ok(TxDone::Stream(1, payload.len()))
        } else {
            Err(item)
        }
    }

    /// Core asynchronous send method for stream channels with timeout and saturation handling.
    ///
    /// Attempts an immediate send of both control item and payload, applying saturation strategies
    /// if needed, and waits for space, shutdown, or timeout, returning the outcome.
    #[inline]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        let (item, payload) = msg;
        assert_eq!(item.length(), payload.len() as i32);
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        let push_result = {
            let payload_tx = &mut self.payload_channel.tx;
            let item_tx = &mut self.control_channel.tx;
            if payload_tx.vacant_len() >= item.length() as usize && item_tx.vacant_len() >= 1 {
                let payload_size = payload_tx.push_slice(payload);
                debug_assert_eq!(payload_size, item.length() as usize);
                let _ = item_tx.try_push(item);
                Ok(())
            } else {
                Err(msg)
            }
        };
        match push_result {
            Ok(_) => SendOutcome::Success,
            Err((item, payload)) => {
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(item),
                    SendSaturation::WarnThenAwait => {
                        self.control_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        {
                            self.control_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }
                let has_room = async {
                    self.payload_channel.tx.wait_vacant(payload.len()).await;
                    self.control_channel.tx.wait_vacant(1).await;
                }.fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.control_channel.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Either::Left(Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);
                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Blocked(item),
                        _ = has_room => {
                            let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                            if !pushed {
                                error!("channel is closed");
                                return SendOutcome::Blocked(item);
                            }
                            match self.control_channel.tx.push(item).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Blocked(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Blocked(item),
                    }
                } else {
                    SendOutcome::Blocked(item)
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout for stream channels.
    ///
    /// Delegates to the core method with no timeout, simplifying the interface for stream sends.
    #[inline]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, None).await
    }

    /// Performs an asynchronous send with an optional timeout for stream channels.
    ///
    /// Delegates to the core method, allowing specification of a timeout for the stream send operation.
    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, timeout).await
    }
}

/// Implementation of `TxCore` for stream-based channels with `StreamEgress`.
///
/// This implementation manages a dual-channel system with a control channel for `StreamEgress`
/// items (length markers) and a payload channel for byte data, synchronizing payload sends
/// with control item creation.
impl TxCore for StreamTx<StreamEgress> {
    /// The type of message sent into the channel, a slice of payload bytes.
    type MsgIn<'a> = &'a [u8];

    /// The type of message that comes out of the channel, a `StreamEgress` control item.
    type MsgOut = StreamEgress;

    /// The type used to count messages, a tuple of control items and payload bytes.
    type MsgSize = (usize, usize);

    /// The type for a slice of messages, a tuple of `StreamEgress` slices and payload byte slices.
    type SliceSource<'b> = (&'b [StreamEgress], &'b [u8]);

    /// The type for target slices, providing four mutable slices for control and payload buffers.
    type SliceTarget<'a> = (
        &'a mut [std::mem::MaybeUninit<StreamEgress>],
        &'a mut [std::mem::MaybeUninit<StreamEgress>],
        &'a mut [std::mem::MaybeUninit<u8>],
        &'a mut [std::mem::MaybeUninit<u8>]
    );

    /// Advances the write indices for both control and payload channels if sufficient space exists.
    ///
    /// Moves the write positions forward for both channels, returning the number of control items
    /// and bytes advanced, or zero if space is insufficient.
    fn shared_advance_index(&mut self, count: Self::MsgSize) -> TxDone {
        let control_avail = self.control_channel.tx.vacant_len();
        let payload_avail = self.payload_channel.tx.vacant_len();
        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.tx.advance_write_index(count.1);
                self.control_channel.tx.advance_write_index(count.0);
            }
            TxDone::Stream(count.0, count.1)
        } else {
            TxDone::Stream(0, 0)
        }
    }

    /// Returns a `TxDone` value indicating one payload slice was processed.
    ///
    /// Reports one control item and the length of the payload slice sent.
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Stream(1, one.len())
    }

    /// Marks both control and payload channels as closed.
    ///
    /// Sends closure signals through the oneshot channels for both, logging trace messages if
    /// receivers are dropped. Always returns `true`.
    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.control_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        }
        if let Some(c) = self.payload_channel.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        }
        true
    }

    /// Returns a tuple representing one control item and an estimated payload size.
    ///
    /// The payload size is based on the ratio of payload channel capacity to control channel capacity.
    fn one(&self) -> Self::MsgSize {
        (1, self.payload_channel.capacity() / self.control_channel.capacity())
    }

    /// Checks if enough time has elapsed since the last error send for periodic logging.
    ///
    /// Uses the control channel’s timer, returning `true` if the interval exceeds the configured
    /// maximum telemetry error rate, resetting the timer.
    fn log_perodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    /// Sends payload slices from an iterator, creating corresponding `StreamEgress` control items.
    ///
    /// Processes payloads up to the control channel’s capacity, waiting synchronously for space
    /// if needed. Returns the number of payloads sent. Includes debug assertions for closure.
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        let mut count = 0;
        let item_limit = self.control_channel.tx.vacant_len();
        let limited_iter = iter.take(item_limit);
        for payload in limited_iter {
            if payload.len() > self.payload_channel.tx.vacant_len() {
                warn!("the payload of the stream should be larger we need {} but found {}", payload.len(), self.payload_channel.tx.vacant_len());
                core_exec::block_on(self.payload_channel.tx.wait_vacant(payload.len()));
            }
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(StreamEgress { length: payload.len() as i32 });
            count += 1;
        }
        count
    }

    /// Increments telemetry for both control and payload channels.
    ///
    /// Updates telemetry with the number of control items and payload bytes sent, handling both
    /// `Normal` and `Stream` cases appropriately.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(i) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            }
            TxDone::Stream(i, p) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.meta_data.id, p as isize);
            }
        }
    }

    /// Disables monitoring for both control and payload channels.
    ///
    /// Sets the monitor indices to a predefined constant to stop monitoring activity.
    #[inline]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the capacity of the control channel.
    #[inline]
    fn shared_capacity(&self) -> Self::MsgSize {
        (self.control_channel.tx.capacity().get(), self.payload_channel.tx.capacity().get())
    }
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the control channel is full.
    #[inline]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    /// Checks if the control channel is empty.
    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    /// Returns the number of vacant units in the control channel.
    #[inline]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        (self.control_channel.tx.vacant_len(), self.payload_channel.tx.vacant_len())
    }
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }

    /// Waits for either shutdown or for the specified units to become vacant in both channels.
    ///
    /// Returns `true` if both channels have sufficient space or are empty, otherwise waits
    /// asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if (self.control_channel.tx.is_empty() || self.control_channel.tx.vacant_len() >= count.0) &&
            (self.payload_channel.tx.is_empty() || self.payload_channel.tx.vacant_len() >= count.1) {
            true
        } else {
            let icap = self.control_channel.capacity();
            let pcap = self.payload_channel.capacity();
            let mut one_down = &mut self.control_channel.oneshot_shutdown;
            if !one_down.is_terminated() {
                let operation = async {
                    self.control_channel.tx.wait_vacant(count.0.min(icap)).await;
                    self.payload_channel.tx.wait_vacant(count.1.min(pcap)).await;
                };
                select! { _ = one_down => false, _ = operation.fuse() => true, }
            } else {
                yield_now().await;
                false
            }
        }
    }

    /// Waits until the specified units become vacant in both channels.
    ///
    /// Returns `true` immediately if enough space exists, otherwise waits asynchronously until
    /// both control and payload channels have the required vacant units.
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.control_channel.tx.vacant_len() >= count.0 &&
            self.payload_channel.tx.vacant_len() >= count.1 {
            true
        } else {
            self.control_channel.tx.wait_vacant(count.0.min(self.control_channel.capacity())).await;
            self.payload_channel.tx.wait_vacant(count.1.min(self.payload_channel.capacity())).await;
            true
        }
    }

    /// Waits for the control channel to become empty or for a shutdown signal.
    ///
    /// Returns `true` if the control channel empties, or `false` if shutdown occurs first.
    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.tx.wait_vacant(usize::from(self.control_channel.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.control_channel.tx.capacity().get() == self.control_channel.tx.vacant_len()
        }
    }

    /// Sends a slice of `StreamEgress` items and their corresponding payload bytes.
    ///
    /// Attempts to send as many items as possible, limited by the vacant space in both channels,
    /// returning the number of control items and bytes sent.
    #[inline]
    fn shared_send_slice(&mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {
        let mut items_sent = 0;
        let mut bytes_sent = 0;
        let (egress_items, payload_bytes) = slice;
        if !egress_items.is_empty() {
            let item_vacant = self.control_channel.tx.vacant_len();
            let payload_vacant = self.payload_channel.tx.vacant_len();
            for &egress in egress_items.iter().take(item_vacant) {
                let len = egress.length as usize;
                if bytes_sent + len > payload_bytes.len() || bytes_sent + len > payload_vacant {
                    break;
                }
                let payload_chunk = &payload_bytes[bytes_sent..bytes_sent + len];
                let pushed = self.payload_channel.tx.push_slice(payload_chunk);
                if pushed != len {
                    break;
                }
                if self.control_channel.tx.try_push(egress).is_err() {
                    break;
                }
                items_sent += 1;
                bytes_sent += len;
            }
        }
        TxDone::Stream(items_sent, bytes_sent)
    }

    /// Provides access to vacant slices in both control and payload channels for zero-copy writing.
    ///
    /// Returns four mutable slices representing the writable portions of both buffers.
    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        let (item_a, item_b) = self.control_channel.tx.vacant_slices_mut();
        let (payload_a, payload_b) = self.payload_channel.tx.vacant_slices_mut();
        (item_a, item_b, payload_a, payload_b)
    }

    /// Attempts to send a single payload slice, creating a `StreamEgress` control item.
    ///
    /// Returns `Ok` with the number of items and bytes sent if successful, or `Err` with a
    /// constructed `StreamEgress` if either channel lacks space. Includes debug assertions for closure.
    #[inline]
    fn shared_try_send(&mut self, payload: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        if self.payload_channel.tx.vacant_len() >= payload.len() &&
            self.control_channel.tx.vacant_len() >= 1 {
            let _ = self.payload_channel.tx.push_slice(payload);
            let _ = self.control_channel.tx.try_push(StreamEgress { length: payload.len() as i32 });
            Ok(TxDone::Stream(1, payload.len()))
        } else {
            Err(StreamEgress { length: payload.len() as i32 })
        }
    }

    /// Core asynchronous send method for `StreamEgress` with timeout and saturation handling.
    ///
    /// Attempts an immediate send of the payload and a generated control item, applying saturation
    /// strategies if needed, and waits for space, shutdown, or timeout, returning the outcome.
    #[inline]
    async fn shared_send_async_core(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        debug_assert!(self.control_channel.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(self.payload_channel.make_closed.is_some(), "Send called after channel marked closed");
        let push_result = {
            let payload_tx = &mut self.payload_channel.tx;
            let item_tx = &mut self.control_channel.tx;
            if payload_tx.vacant_len() >= payload.len() && item_tx.vacant_len() >= 1 {
                let payload_size = payload_tx.push_slice(payload);
                debug_assert_eq!(payload_size, payload.len());
                let _ = item_tx.try_push(StreamEgress { length: payload.len() as i32 });
                Ok(())
            } else {
                Err(StreamEgress { length: payload.len() as i32 })
            }
        };
        match push_result {
            Ok(_) => SendOutcome::Success,
            Err(item) => {
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(item),
                    SendSaturation::WarnThenAwait => {
                        self.control_channel.report_tx_full_warning(ident);
                        self.payload_channel.report_tx_full_warning(ident);
                    }
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        {
                            self.control_channel.report_tx_full_warning(ident);
                            self.payload_channel.report_tx_full_warning(ident);
                        }
                    }
                }
                let has_room = async {
                    self.payload_channel.tx.wait_vacant(payload.len()).await;
                    self.control_channel.tx.wait_vacant(1).await;
                }.fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.control_channel.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Either::Left(Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);
                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Blocked(item),
                        _ = has_room => {
                            let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                            if !pushed {
                                error!("channel is closed");
                                return SendOutcome::Blocked(item);
                            }
                            match self.control_channel.tx.push(item).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Blocked(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Blocked(item),
                    }
                } else {
                    SendOutcome::Blocked(item)
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout for `StreamEgress`.
    ///
    /// Delegates to the core method with no timeout, simplifying the interface for stream sends.
    #[inline]
    async fn shared_send_async(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(payload, ident, saturation, None).await
    }

    /// Performs an asynchronous send with an optional timeout for `StreamEgress`.
    ///
    /// Delegates to the core method, allowing specification of a timeout for the stream send operation.
    #[inline]
    async fn shared_send_async_timeout(
        &mut self,
        payload: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(payload, ident, saturation, timeout).await
    }
}