// ss[related channel.stream-dual-buffer]
use log::{error, trace, warn};
use std::time::{Duration, Instant};
use futures_util::{select, FutureExt};
// ss[related channel.stream-dual-buffer]
use futures_util::future::{Either, FusedFuture};
use std::future::pending;
use ringbuf::traits::Observer;
// ss[related channel.stream-dual-buffer]
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use crate::{steady_config, ActorIdentity, SendOutcome, SendSaturation, StreamControlItem, StreamEgress, StreamIngress, StreamTx, TxCore, TxDone, MONITOR_NOT};
// ss[related channel.stream-dual-buffer]
use crate::loop_driver::pin_mut;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::yield_now::yield_now;
// ss[related channel.stream-dual-buffer]
use crate::core_exec;

/// Implementation of `TxCore` for stream-based channels with `StreamIngress`.
///
/// This implementation manages a dual-channel system with a control channel for `StreamIngress`
/// items and a payload channel for byte data, ensuring synchronized transmission of control
/// messages and their associated payloads.
// ss[related channel.stream-dual-buffer]
impl TxCore for StreamTx<StreamIngress> {
    /// The type of message sent into the channel, a tuple of a `StreamIngress` item and its payload bytes.
    type MsgIn<'a> = (StreamIngress, &'a [u8]);

    /// The type of message that comes out of the channel, the `StreamIngress` control item.
    // ss[related channel.stream-dual-buffer]
    type MsgOut = StreamIngress;

    /// The type used to count messages, a tuple of control items and payload bytes.
    // ss[related channel.stream-dual-buffer]
    type MsgSize = (usize, usize);

    /// The type for a slice of messages, a tuple of control item slices and payload byte slices.
    // ss[related channel.stream-dual-buffer]
    type SliceSource<'b> = (&'b [StreamIngress], &'b [u8]);

    /// The type for target slices, providing four mutable slices for control and payload buffers.
    // ss[related channel.stream-dual-buffer]
    type SliceTarget<'a> = (
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<StreamIngress>],
        &'a mut [std::mem::MaybeUninit<u8>],
        &'a mut [std::mem::MaybeUninit<u8>],
    );

    /// Returns a `TxDone` value indicating one control item and its payload size were processed.
    // ss[related channel.stream-dual-buffer]
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Stream(1, one.1.len())
    }

    /// Advances the write indices for both control and payload channels if sufficient space exists.
    ///
    /// This method moves the write positions forward for both channels, returning the number of
    /// control items and bytes advanced, or zero if space is insufficient.
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
    fn shared_mark_closed(&mut self) {
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
    }

    /// Returns a tuple representing one control item and an estimated payload size.
    ///
    /// The payload size is calculated as the ratio of payload channel capacity to control channel capacity.
    // ss[related channel.stream-dual-buffer]
    fn one(&self) -> Self::MsgSize {
        (1, self.payload_channel.capacity() / self.control_channel.capacity())
    }

    /// Checks if enough time has elapsed since the last error send for periodic logging.
    ///
    /// Uses the control channel’s timer, returning `true` if the interval exceeds the configured
    /// maximum telemetry error rate, resetting the timer.
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the capacity of the control channel.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_capacity(&self) -> Self::MsgSize {
        (self.control_channel.tx.capacity().get(),self.payload_channel.tx.capacity().get())
    }
    // ss[related channel.stream-dual-buffer]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the control channel is full.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    /// Checks if the control channel is empty.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    /// Returns the number of vacant units in the control channel.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        (self.control_channel.tx.vacant_len(), self.payload_channel.tx.vacant_len())
    }
    // ss[related channel.stream-dual-buffer]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }

    /// Waits for either shutdown or for the specified units to become vacant in both channels.
    ///
    /// Returns `true` if both channels have sufficient space or are empty, otherwise waits
    /// asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
                    #[allow(deprecated)]
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
                    Some(duration) => Either::Left(futures_timer::Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);
                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Closed(item),
                        _ = has_room => {
                            let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                            if !pushed {
                                error!("channel is closed");
                                return SendOutcome::Closed(item);
                            }
                            match self.control_channel.tx.push(item).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Closed(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Timeout(item),
                    }
                } else {
                    SendOutcome::Closed(item)
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout for stream channels.
    ///
    /// Delegates to the core method with no timeout, simplifying the interface for stream sends.
    #[inline]
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
// ss[related channel.stream-dual-buffer]
impl TxCore for StreamTx<StreamEgress> {
    /// The type of message sent into the channel, a slice of payload bytes.
    type MsgIn<'a> = &'a [u8];

    /// The type of message that comes out of the channel, a `StreamEgress` control item.
    // ss[related channel.stream-dual-buffer]
    type MsgOut = StreamEgress;

    /// The type used to count messages, a tuple of control items and payload bytes.
    // ss[related channel.stream-dual-buffer]
    type MsgSize = (usize, usize);

    /// The type for a slice of messages, a tuple of `StreamEgress` slices and payload byte slices.
    // ss[related channel.stream-dual-buffer]
    type SliceSource<'b> = (&'b [StreamEgress], &'b [u8]);

    /// The type for target slices, providing four mutable slices for control and payload buffers.
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Stream(1, one.len())
    }

    /// Marks both control and payload channels as closed.
    ///
    /// Sends closure signals through the oneshot channels for both, logging trace messages if
    /// receivers are dropped. Always returns `true`.
    // ss[related channel.stream-dual-buffer]
    fn shared_mark_closed(&mut self) {
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
    }

    /// Returns a tuple representing one control item and an estimated payload size.
    ///
    /// The payload size is based on the ratio of payload channel capacity to control channel capacity.
    // ss[related channel.stream-dual-buffer]
    fn one(&self) -> Self::MsgSize {
        (1, self.payload_channel.capacity() / self.control_channel.capacity())
    }

    /// Checks if enough time has elapsed since the last error send for periodic logging.
    ///
    /// Uses the control channel’s timer, returning `true` if the interval exceeds the configured
    /// maximum telemetry error rate, resetting the timer.
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the capacity of the control channel.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_capacity(&self) -> Self::MsgSize {
        (self.control_channel.tx.capacity().get(), self.payload_channel.tx.capacity().get())
    }
    // ss[related channel.stream-dual-buffer]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the control channel is full.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_is_full(&self) -> bool {
        self.control_channel.tx.is_full()
    }

    /// Checks if the control channel is empty.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_is_empty(&self) -> bool {
        self.control_channel.tx.is_empty()
    }

    /// Returns the number of vacant units in the control channel.
    #[inline]
    // ss[related channel.stream-dual-buffer]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        (self.control_channel.tx.vacant_len(), self.payload_channel.tx.vacant_len())
    }
    // ss[related channel.stream-dual-buffer]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }

    /// Waits for either shutdown or for the specified units to become vacant in both channels.
    ///
    /// Returns `true` if both channels have sufficient space or are empty, otherwise waits
    /// asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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
                    #[allow(deprecated)]
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
                    Some(duration) => Either::Left(futures_timer::Delay::new(duration).fuse()),
                    None => Either::Right(pending::<()>().fuse()),
                };
                pin_mut!(timeout_future);
                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Closed(item),
                        _ = has_room => {
                            let pushed = self.payload_channel.tx.push_slice(payload) == payload.len();
                            if !pushed {
                                error!("channel is closed");
                                return SendOutcome::Closed(item);
                            }
                            match self.control_channel.tx.push(item).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Closed(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Timeout(item),
                    }
                } else {
                    SendOutcome::Closed(item)
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout for `StreamEgress`.
    ///
    /// Delegates to the core method with no timeout, simplifying the interface for stream sends.
    #[inline]
    // ss[related channel.stream-dual-buffer]
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
    // ss[related channel.stream-dual-buffer]
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

#[cfg(test)]
// ss[related channel.stream-dual-buffer]
mod core_tx_stream_tests {
    use std::time::{Duration, Instant};
    use crate::{GraphBuilder, ScheduleAs, SteadyActor, StreamEgress, StreamIngress, SendSaturation, TxCore, TxDone, ActorIdentity, core_exec, SendOutcome, StreamTx, steady_tx::TxMetaDataProvider};
    // ss[related channel.stream-dual-buffer]
    use crate::distributed::aqueduct_stream::Defrag;
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
            
            // Manually backdate the timer again to test subsequent trigger
            tx_guard.control_channel.last_error_send = Instant::now() - Duration::from_secs(30);
            assert!(tx_guard.log_perodic());
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

            assert!(tx_guard.shared_capacity_for((5, 50)));
            assert!(!tx_guard.shared_capacity_for((100, 1000)));
            
            assert!(tx_guard.shared_vacant_units_for((5, 50)));
            assert!(!tx_guard.shared_vacant_units_for((100, 1000)));
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
}
