use std::fmt::Debug;
use std::future::pending;
use log::{error, trace, warn};
use futures_util::{select, FutureExt};
use std::time::{Duration, Instant};
use futures::pin_mut;
use futures_timer::Delay;
use futures_util::lock::MutexGuard;
use ringbuf::traits::Observer;
use futures_util::future::{Either, FusedFuture};
use async_ringbuf::producer::AsyncProducer;
use ringbuf::producer::Producer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::steady_tx::TxDone;
use crate::{steady_config, ActorIdentity, SendOutcome, SendSaturation, StreamIngress, StreamEgress, Tx, MONITOR_NOT};
use crate::distributed::aqueduct_stream::{StreamControlItem, StreamTx};
use crate::core_exec;
use crate::yield_now;

/// Trait defining the core functionality for transmitting data in a steady-state system.
///
/// This trait provides a standardized interface for sending messages, managing channel state,
/// and interacting with telemetry in a steady-state actor system. It is designed to be implemented
/// by types that handle data transmission, such as standard channels (`Tx<T>`) and stream-based
/// channels (`StreamTx<StreamControlItem>`). The trait supports both synchronous and asynchronous
/// operations, as well as zero-copy mechanisms through slice-based methods.
pub trait TxCore {
    /// The type of message that can be sent into the channel.
    type MsgIn<'a>;

    /// The type of message that comes out of the channel.
    type MsgOut;

    /// The type used to represent the size or count of messages, typically `usize` for standard
    /// channels or a tuple for streams.
    type MsgSize: Copy + Debug;

    /// The type for a slice of messages to be sent, used in zero-copy operations.
    type SliceSource<'b> where Self::MsgOut: 'b;

    /// The type for the target slices where messages are written, typically for zero-copy writes.
    type SliceTarget<'a> where Self: 'a;

    /// Marks the channel as closed, preventing further sends.
    ///
    /// This method signals that no more messages will be transmitted, often by notifying receivers
    /// through an oneshot channel. It always returns `true` to indicate the request was processed.
    fn shared_mark_closed(&mut self) -> bool;

    /// Sends messages from an iterator until the channel is full.
    ///
    /// This method processes messages from the provided iterator without blocking, stopping when
    /// the channel reaches capacity. It returns the number of messages successfully sent.
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize;

    /// Determines whether it is time to perform periodic logging.
    ///
    /// This method checks if a sufficient amount of time has elapsed since the last log, based on
    /// a predefined interval, to decide if logging should occur.
    fn log_perodic(&mut self) -> bool;

    /// Returns a value representing a single unit for message counting.
    ///
    /// For standard channels, this typically returns `1`. For stream channels, it may return a tuple
    /// representing one control item and an estimated payload size.
    fn one(&self) -> Self::MsgSize;

    /// Increments telemetry data based on the number of messages sent.
    ///
    /// This method updates the telemetry system with the count of messages or bytes transmitted,
    /// depending on the channel type and the provided `TxDone` value.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>);

    /// Notifies or resets the monitor, typically by setting a monitor index to a predefined value.
    ///
    /// This method is used to disable or reset monitoring activity for the channel.
    fn monitor_not(&mut self);

    /// Returns the capacity of the channel.
    ///
    /// This method provides the total number of messages the channel can hold.
    fn shared_capacity(&self) -> Self::MsgSize;

    //TODO: capacity fits
    //TODO: await both fields and auto deref to (x,0) for the single?

    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool;



    /// Checks if the channel is full.
    ///
    /// Returns `true` if the channel has reached its capacity and cannot accept more messages.
    fn shared_is_full(&self) -> bool;

    /// Checks if the channel is empty.
    ///
    /// Returns `true` if there are no messages currently in the channel.
    fn shared_is_empty(&self) -> bool;

    /// Returns the number of vacant units in the channel.
    ///
    /// This method indicates how many more messages can be sent before the channel is full.
    fn shared_vacant_units(&self) -> Self::MsgSize;

    /// Return true if this message size will fit in the vacant space
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool;



    /// Waits for either shutdown or for a specified number of units to become vacant.
    ///
    /// This asynchronous method returns `true` if the specified number of units became available,
    /// or `false` if a shutdown signal was received instead.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool;

    /// Waits until a specified number of units become vacant.
    ///
    /// This asynchronous method blocks until the channel has enough free space to accommodate
    /// the requested number of units, returning `true` when the condition is met.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool;

    /// Waits for the channel to become empty or for a shutdown signal.
    ///
    /// This asynchronous method returns `true` if the channel empties, or `false` if a shutdown
    /// is triggered before the channel becomes empty.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_empty(&mut self) -> bool;

    /// Advances the write index by a specified number of units.
    ///
    /// This method is used in zero-copy operations to manually update the write position after
    /// directly writing to the channel’s buffer. It returns a `TxDone` value indicating the
    /// number of units advanced.
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone;

    /// Sends a slice of messages to the channel.
    ///
    /// This method attempts to send all messages in the provided slice, returning a `TxDone`
    /// value with the number of items successfully sent.
    fn shared_send_slice(&mut self, source: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy;

    /// Provides direct access to the vacant slices of the channel for zero-copy writing.
    ///
    /// This method returns the writable portions of the channel’s buffer, allowing direct
    /// manipulation of the underlying memory.
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_>;

    /// Attempts to send a single message without blocking.
    ///
    /// Returns `Ok(TxDone)` if the message was sent successfully, or `Err(Self::MsgOut)` if
    /// the channel is full and the message could not be sent.
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut>;

    /// Core asynchronous send method with support for timeouts.
    ///
    /// This method attempts to send a message asynchronously, applying the specified saturation
    /// strategy if the channel is full and respecting an optional timeout. It returns a `SendOutcome`
    /// indicating success or failure.
    #[allow(async_fn_in_trait)]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut>;

    /// Asynchronous send with an optional timeout.
    ///
    /// This method delegates to `shared_send_async_core`, providing a convenient interface for
    /// sending with a timeout parameter.
    #[allow(async_fn_in_trait)]
    async fn shared_send_async_timeout(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut>;

    /// Asynchronous send without a timeout.
    ///
    /// This method delegates to `shared_send_async_core` with no timeout, offering a simpler
    /// interface for non-time-sensitive sends.
    #[allow(async_fn_in_trait)]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut>;

    /// Handles the completion of sending one message.
    ///
    /// This method returns a `TxDone` value indicating the result of sending a single message,
    /// typically used to report the number of items or bytes sent.
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone;
}

/// Implementation of `TxCore` for standard channels (`Tx<T>`).
///
/// This implementation provides the transmission functionality for a standard channel, supporting
/// synchronous and asynchronous message sending, zero-copy operations, and telemetry integration.
impl<T> TxCore for Tx<T> {
    /// The type of message that can be sent into the channel, matching the channel’s generic type.
    type MsgIn<'a> = T;

    /// The type of message that comes out of the channel, identical to `MsgIn` for standard channels.
    type MsgOut = T;

    /// The type used to count messages, set to `usize` for standard channels.
    type MsgSize = usize;

    /// The type for a slice of messages to be sent, a reference to an array of `T`.
    type SliceSource<'b> = &'b [T] where T: 'b;

    /// The type for target slices, providing two mutable slices of uninitialized memory for zero-copy writes.
    type SliceTarget<'a> = (&'a mut [std::mem::MaybeUninit<T>], &'a mut [std::mem::MaybeUninit<T>]) where T: 'a;

    /// Advances the write index by the requested number of units, limited by available space.
    ///
    /// This method adjusts the write position in the channel’s buffer, ensuring it does not exceed
    /// the vacant space, and returns the number of units advanced.
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone {
        let avail = self.tx.vacant_len();
        let idx = if request > avail { avail } else { request };
        unsafe { self.tx.advance_write_index(idx); }
        TxDone::Normal(idx)
    }

    /// Returns a `TxDone` value indicating one message was processed.
    ///
    /// For standard channels, this always reports a single message sent.
    fn done_one(&self, _one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Normal(1)
    }

    /// Marks the channel as closed by sending a signal through the oneshot channel.
    ///
    /// If the oneshot sender is already taken, it logs a trace message indicating a redundant call.
    /// This method is idempotent and always returns `true`.
    fn shared_mark_closed(&mut self) -> bool {
        if let Some(c) = self.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        } else {
            trace!("{:?}\n already marked closed, check for redundant calls, ensure mark_closed is called last after all other conditions!", self.channel_meta_data.meta_data);
        }
        true
    }

    /// Returns `1` as the unit value for counting messages.
    ///
    /// This represents a single message in the context of a standard channel.
    fn one(&self) -> Self::MsgSize {
        1
    }

    /// Checks if enough time has elapsed since the last error send to allow periodic logging.
    ///
    /// Returns `true` if the elapsed time exceeds the configured maximum telemetry error rate,
    /// resetting the timer, otherwise returns `false`.
    fn log_perodic(&mut self) -> bool {
        if self.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.last_error_send = Instant::now();
            true
        }
    }

    /// Sends messages from an iterator until the channel is full.
    ///
    /// If the channel is already closed, it logs an error but proceeds with the send operation.
    /// Returns the number of messages successfully sent.
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
        if self.make_closed.is_none() {
            #[cfg(not(test))]
            trace!("Send called after channel marked closed"); //does happen in unit tests
        }
        self.tx.push_iter(iter)
    }

    /// Increments telemetry data with the number of messages sent.
    ///
    /// This method updates the telemetry based on the `TxDone` value, expecting `Normal` for
    /// standard channels and logging a warning if `Stream` is received unexpectedly.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        match done_count {
            TxDone::Normal(d) => {
                self.local_monitor_index = tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, d as isize)
            }
            TxDone::Stream(i, _p) => {
                warn!("internal error should have gotten Normal");
                self.local_monitor_index = tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, i as isize)
            }
        }
    }

    /// Disables monitoring by setting the local monitor index to a predefined constant.
    #[inline]
    fn monitor_not(&mut self) {
        self.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the total capacity of the channel.
    #[inline]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the channel is at full capacity.
    #[inline]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }

    /// Checks if the channel contains no messages.
    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.tx.is_empty()
    }

    /// Calculates the number of vacant units in the channel.
    ///
    /// This method uses modulo arithmetic to determine the available space, accounting for
    /// wrap-around in the ring buffer.
    #[inline]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        let capacity = self.tx.capacity().get();
        let modulus = 2 * capacity;
        let read_idx = self.tx.read_index();
        let write_idx = self.tx.write_index();
        let result = (capacity + read_idx - write_idx) % modulus;
        assert!(result <= capacity);
        result
    }

    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }


    /// Waits for either a shutdown signal or for the specified number of units to become vacant.
    ///
    /// Returns immediately with `true` if the channel is empty or has enough vacant space.
    /// Otherwise, it waits asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.tx.is_empty() || self.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let safe_count = count.min(self.tx.capacity().into());
                let mut operation = &mut self.tx.wait_vacant(safe_count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
                yield_now().await;
                false
            }
        }
    }

    /// Waits until the specified number of units become vacant in the channel.
    ///
    /// Returns `true` immediately if enough space is already available, otherwise waits
    /// asynchronously until the condition is met.
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let safe_count = count.min(self.tx.capacity().into());
            let operation = &mut self.tx.wait_vacant(safe_count);
            operation.await;
            true
        }
    }

    /// Waits for the channel to become empty or for a shutdown signal.
    ///
    /// Returns `true` if the channel empties, or `false` if shutdown occurs first. If the
    /// shutdown signal is already received, it checks the current state directly.
    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.tx.wait_vacant(usize::from(self.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            self.tx.capacity().get() == self.tx.vacant_len()
        }
    }

    /// Sends a slice of messages to the channel.
    ///
    /// If the slice is non-empty, it attempts to send as many messages as possible, returning
    /// the number of items sent. Returns zero if the slice is empty.
    #[inline]
    fn shared_send_slice(&mut self, slice: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy {
        if !slice.is_empty() {
            TxDone::Normal(self.tx.push_slice(slice))
        } else {
            TxDone::Normal(0)
        }
    }

    /// Provides access to the vacant slices of the channel for zero-copy writing.
    ///
    /// Returns two mutable slices representing the available portions of the buffer.
    #[inline]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        self.tx.vacant_slices_mut()
    }

    /// Attempts to send a single message without blocking.
    ///
    /// Returns `Ok` with a `TxDone` value if the message is sent, or `Err` with the message
    /// if the channel is full. Includes a debug assertion to ensure the channel is not closed.
    #[inline]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
        debug_assert!(self.make_closed.is_some(), "Send called after channel marked closed");
        match self.tx.try_push(msg) {
            Ok(_) => Ok(TxDone::Normal(1)),
            Err(m) => Err(m),
        }
    }

    /// Core asynchronous send method with timeout and saturation handling.
    ///
    /// Attempts an immediate send, and if the channel is full, applies the saturation strategy.
    /// It then waits for space, shutdown, or timeout, returning the outcome of the operation.
    #[inline]
    async fn shared_send_async_core(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
        timeout: Option<Duration>,
    ) -> SendOutcome<Self::MsgOut> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        match self.tx.try_push(msg) {
            Ok(_) => SendOutcome::Success,
            Err(msg) => {
                match saturation {
                    SendSaturation::AwaitForRoom => {}
                    SendSaturation::ReturnBlockedMsg => return SendOutcome::Blocked(msg),
                    SendSaturation::WarnThenAwait => self.report_tx_full_warning(ident),
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        self.report_tx_full_warning(ident);
                    }
                }
                let has_room = self.tx.wait_vacant(1).fuse();
                pin_mut!(has_room);
                let mut one_down = &mut self.oneshot_shutdown;
                let timeout_future = match timeout {
                    Some(duration) => Delay::new(duration).fuse(),
                    None => Delay::new(Duration::from_secs(i32::MAX as u64)).fuse(),
                };
                pin_mut!(timeout_future);
                if !one_down.is_terminated() {
                    select! {
                        _ = one_down => SendOutcome::Blocked(msg),
                        _ = has_room => {
                            match self.tx.push(msg).await {
                                Ok(_) => SendOutcome::Success,
                                Err(t) => {
                                    error!("channel is closed");
                                    SendOutcome::Blocked(t)
                                }
                            }
                        }
                        _ = timeout_future => SendOutcome::Blocked(msg),
                    }
                } else {
                    SendOutcome::Blocked(msg)
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout.
    ///
    /// Delegates to the core method with no timeout specified, simplifying the interface for
    /// cases where timing out is not required.
    #[inline]
    async fn shared_send_async(
        &mut self,
        msg: Self::MsgIn<'_>,
        ident: ActorIdentity,
        saturation: SendSaturation,
    ) -> SendOutcome<Self::MsgOut> {
        self.shared_send_async_core(msg, ident, saturation, None).await
    }

    /// Performs an asynchronous send with an optional timeout.
    ///
    /// Delegates to the core method, allowing specification of a timeout for the send operation.
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
    fn shared_mark_closed(&mut self) -> bool {
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

// Unit-tests for the combined TxCore / RxCore behavior
#[cfg(test)]
mod core_tx_rx_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;
    use crate::core_rx::RxCore;

    /// Tests basic send and receive operations using `TxCore` and `RxCore`.
    ///
    /// Verifies that messages can be sent through the channel, checked for availability,
    /// and retrieved correctly, ensuring proper channel state management.
    #[test]
    fn test_tx_rx_basic_flow() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx, rx) = builder.build_channel::<i32>();
        let tx = tx.clone();
        let mut txg = tx.try_lock().expect("");
        assert_eq!(txg.shared_capacity(), 2);
        assert!(txg.shared_is_empty());
        let sent = txg.shared_send_iter_until_full([7, 8].into_iter());
        assert_eq!(sent, 2);
        assert!(txg.shared_is_full());
        drop(txg);
        let rx = rx.clone();
        let mut rxg = rx.try_lock().expect("");
        assert_eq!(rxg.shared_capacity(), 2);
        assert_eq!(rxg.shared_avail_units(), 2);
        assert_eq!(rxg.shared_try_peek(), Some(&7));
        drop(rxg);
        let rx = rx.clone();
        let mut rxg = rx.try_lock().expect("");
        assert_eq!(rxg.shared_try_take().map(|(_, v)| v), Some(7));
        assert_eq!(rxg.shared_try_take().map(|(_, v)| v), Some(8));
        assert!(rxg.shared_is_empty());
    }

    /// Tests detection of potential showstopper conditions in `RxCore`.
    ///
    /// Sends a message and repeatedly peeks at it, verifying that the showstopper condition
    /// is triggered after a specified number of peeks without taking the message.
    #[test]
    fn test_bad_message_detection() {
        let builder = ChannelBuilder::default().with_capacity(1);
        let (tx, rx) = builder.build_channel::<u8>();
        let tx = tx.clone();
        let mut txg = tx.try_lock().expect("");
        assert_eq!(txg.shared_send_iter_until_full([42].into_iter()), 1);
        drop(txg);
        let rx = rx.clone();
        let rxg = rx.try_lock().expect("");
        assert_eq!(rxg.shared_try_peek(), Some(&42));
        assert_eq!(rxg.shared_try_peek(), Some(&42));
        assert!(rxg.is_showstopper(2));
        assert!(!rxg.is_showstopper(5));
    }

    use futures::executor::block_on;
    use futures_util::lock::Mutex;
    use crate::TxCore;
    use crate::{ActorIdentity, SendSaturation, SendOutcome};
    use std::time::Duration;
    use crate::GraphBuilder;

    /// A mock implementation of `TxCore` for testing `MutexGuard` forwarding behavior.
    ///
    /// Provides predictable responses to method calls, allowing verification of correct
    /// delegation through a mutex guard.
    struct FakeTx {
        closed: bool,
        send_count: usize,
        log_calls: usize,
        one_val: usize,
        capacity: usize,
        is_full: bool,
        is_empty: bool,
        vacant: usize,
    }

    impl FakeTx {
        /// Creates a new instance with default values for testing.
        fn new() -> Self {
            FakeTx { closed: false, send_count: 0, log_calls: 0, one_val: 3, capacity: 4, is_full: false, is_empty: true, vacant: 4 }
        }
    }

    impl TxCore for FakeTx {
        type MsgIn<'a> = usize;
        type MsgOut = usize;
        type MsgSize = usize;
        type SliceSource<'b> = &'b [usize];
        type SliceTarget<'a> = (&'a [usize], &'a [usize]);

        /// Marks the channel as closed and returns `true`.
        fn shared_mark_closed(&mut self) -> bool {
            self.closed = true;
            true
        }

        /// Counts and accumulates the number of items sent from an iterator.
        fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize {
            let cnt = iter.count();
            self.send_count += cnt;
            cnt
        }

        /// Toggles logging based on the number of calls.
        fn log_perodic(&mut self) -> bool {
            self.log_calls += 1;
            self.log_calls > 1
        }

        /// Returns a predefined unit value, adjustable by `monitor_not`.
        fn one(&self) -> Self::MsgSize {
            self.one_val
        }

        /// Does nothing with telemetry, maintaining mock simplicity.
        fn telemetry_inc<const LEN: usize>(&mut self, _d: TxDone, _tel: &mut crate::monitor_telemetry::SteadyTelemetrySend<LEN>) {
        }

        /// Resets the unit value to zero.
        fn monitor_not(&mut self) {
            self.one_val = 0;
        }

        /// Returns a fixed capacity value.
        fn shared_capacity(&self) -> usize {
            self.capacity
        }

        fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
            let cap = self.shared_capacity();
            size <= cap
        }
        /// Returns a fixed full status.
        fn shared_is_full(&self) -> bool {
            self.is_full
        }

        /// Returns a fixed empty status.
        fn shared_is_empty(&self) -> bool {
            self.is_empty
        }

        /// Returns a fixed vacant units value.
        fn shared_vacant_units(&self) -> usize {
            self.vacant
        }

        fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
            let vacant = self.shared_vacant_units();
            vacant >= size
        }

        /// Simulates immediate availability for shutdown or vacant wait.
        async fn shared_wait_shutdown_or_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
            true
        }

        /// Simulates immediate availability for vacant units wait.
        async fn shared_wait_vacant_units(&mut self, _count: Self::MsgSize) -> bool {
            true
        }

        /// Simulates immediate availability for empty wait.
        async fn shared_wait_empty(&mut self) -> bool {
            true
        }

        /// Returns zero advancement for simplicity.
        fn shared_advance_index(&mut self, _request: Self::MsgSize) -> TxDone {
            TxDone::Normal(0)
        }

        /// Returns zero items sent for slice operations.
        #[inline]
        fn shared_send_slice(&mut self, _slice: Self::SliceSource<'_>) -> TxDone {
            TxDone::Normal(0)
        }

        /// Returns empty slices for poking.
        #[inline]
        fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
            let (item_a, item_b) = (&[], &[]);
            (item_a, item_b)
        }

        /// Always succeeds, returning the sent message as the number of items.
        fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut> {
            Ok(TxDone::Normal(msg))
        }

        /// Always returns success for async core send.
        async fn shared_send_async_core(
            &mut self,
            _msg: Self::MsgIn<'_>,
            _ident: ActorIdentity,
            _s: SendSaturation,
            _timeout: Option<Duration>,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        /// Always returns success for async send with timeout.
        async fn shared_send_async_timeout(
            &mut self,
            _msg: Self::MsgIn<'_>,
            _id: ActorIdentity,
            _s: SendSaturation,
            _timeout: Option<Duration>,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        /// Always returns success for async send without timeout.
        async fn shared_send_async(
            &mut self,
            _msg: Self::MsgIn<'_>,
            _id: ActorIdentity,
            _s: SendSaturation,
        ) -> SendOutcome<Self::MsgOut> {
            SendOutcome::Success
        }

        /// Returns the input value as the number of items sent.
        fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone {
            TxDone::Normal(*one)
        }
    }

    /// Tests that `MutexGuard` correctly forwards `TxCore` methods to the underlying type.
    ///
    /// Verifies that all trait methods behave as expected when called through a mutex guard,
    /// using a mock implementation to ensure predictable outcomes.
    #[test]
    fn test_mutexguard_txcore_methods() {
        let mtx = Mutex::new(FakeTx::new());
        let mut guard = block_on(mtx.lock());
        assert!(!guard.closed);
        assert!(guard.shared_mark_closed());
        assert!(guard.closed);
        let sent = guard.shared_send_iter_until_full([10, 20, 30].into_iter());
        assert_eq!(sent, 3);
        assert!(!guard.log_perodic());
        assert!(guard.log_perodic());
        assert_eq!(guard.one(), 3);
        guard.monitor_not();
        assert_eq!(guard.one(), 0);
        assert_eq!(guard.shared_capacity(), 4);
        assert!(!guard.shared_is_full());
        assert!(guard.shared_is_empty());
        assert_eq!(guard.shared_vacant_units(), 4);
        assert!(block_on(guard.shared_wait_shutdown_or_vacant_units(1)));
        assert!(block_on(guard.shared_wait_vacant_units(1)));
        assert!(block_on(guard.shared_wait_empty()));
        let try_res = guard.shared_try_send(5);
        assert!(try_res.is_ok());
        let ident = ActorIdentity::new(0, "test", None);
        let res = block_on(guard.shared_send_async(7, ident, SendSaturation::AwaitForRoom));
        assert!(matches!(res, SendOutcome::Success));
        let res_to = block_on(guard.shared_send_async_timeout(8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
        assert!(matches!(res_to, SendOutcome::Success));
        assert_eq!(guard.done_one(&9), TxDone::Normal(9));
    }

    /// Helper function to create a new `Tx<u8>` for testing purposes.
    fn new_tx() -> Tx<u8> {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let (tx, _rx) = builder.eager_build_internal();
        tx
    }

    /// Tests `done_one` and `shared_mark_closed` for `Tx<u8>`.
    ///
    /// Ensures that `done_one` consistently reports one item and that `shared_mark_closed`
    /// behaves correctly on first and subsequent calls.
    #[test]
    fn done_one_and_shared_mark_closed() {
        let mut tx = new_tx();
        assert_eq!(tx.done_one(&42u8), TxDone::Normal(1));
        assert!(tx.shared_mark_closed());
        assert!(tx.shared_mark_closed());
    }

    /// Tests sending after closure and the associated warning behavior.
    ///
    /// Verifies that sending after marking the channel closed triggers a warning and still
    /// processes the operation correctly.
    #[test]
    fn shared_send_iter_until_full_and_warn_after_close() {
        let mut tx = new_tx();
        let pushed = tx.shared_send_iter_until_full([7u8, 8u8].into_iter());
        assert_eq!(pushed, 2);
        let _ = tx.shared_mark_closed();
        let pushed2 = tx.shared_send_iter_until_full(std::iter::empty());
        assert_eq!(pushed2, 0);
    }

    /// Tests `shared_try_send` and async send variants for `Tx<u8>`.
    ///
    /// Confirms that immediate sends succeed and that async methods complete successfully
    /// under normal conditions.
    #[test]
    fn shared_try_send_and_async_variants() {
        let mut tx = new_tx();
        let ident = ActorIdentity::new(0, "me", None);
        assert_eq!(tx.shared_try_send(99u8), Ok(TxDone::Normal(1)));
        let outcome = block_on(tx.shared_send_async_core(5u8, ident, SendSaturation::AwaitForRoom, None));
        assert!(matches!(outcome, SendOutcome::Success));
        let outcome2 = block_on(tx.shared_send_async(6u8, ident, SendSaturation::ReturnBlockedMsg));
        assert!(matches!(outcome2, SendOutcome::Success));
        let outcome3 = block_on(tx.shared_send_async_timeout(7u8, ident, SendSaturation::ReturnBlockedMsg, Some(Duration::from_millis(1))));
        assert!(matches!(outcome3, SendOutcome::Success));
    }
}