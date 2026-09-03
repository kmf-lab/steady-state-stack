// ss[related channel.backpressure-never-drop]
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use log::{error, trace, warn};
// ss[related philosophy.structural-hierarchy]
use futures_util::{select, FutureExt};
// ss[related channel.backpressure-never-drop]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use futures::pin_mut;
// ss[related philosophy.structural-hierarchy]
use futures_timer::Delay;
// ss[related channel.backpressure-never-drop]
use ringbuf::traits::Observer;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::{FusedFuture};
// ss[related philosophy.structural-hierarchy]
use async_ringbuf::producer::AsyncProducer;
// ss[related channel.backpressure-never-drop]
use ringbuf::producer::Producer;
// ss[related philosophy.structural-hierarchy]
use crate::monitor_telemetry::SteadyTelemetrySend;
// ss[related philosophy.structural-hierarchy]
use crate::steady_tx::TxDone;
// ss[related channel.backpressure-never-drop]
use crate::{steady_config, ActorIdentity, SendOutcome, SendSaturation, Tx, MONITOR_NOT, MONITOR_UNKNOWN};
// ss[related philosophy.structural-hierarchy]
use crate::yield_now;

/// Trait defining the core functionality for transmitting data in a steady-state system.
///
/// This trait provides a standardized interface for sending messages, managing channel state,
/// and interacting with telemetry in a steady-state actor system. It is designed to be implemented
/// by types that handle data transmission, such as standard channels (`Tx<T>`) and stream-based
/// channels (`StreamTx<StreamControlItem>`). The trait supports both synchronous and asynchronous
/// operations, as well as zero-copy mechanisms through slice-based methods.
// ss[related channel.backpressure-never-drop]
pub trait TxCore {
    /// The type of message that can be sent into the channel.
    // ss[related philosophy.structural-hierarchy]
    type MsgIn<'a>;

    /// The type of message that comes out of the channel.
    // ss[related channel.backpressure-never-drop]
    type MsgOut;

    /// The type used to represent the size or count of messages, typically `usize` for standard
    /// channels or a tuple for streams.
    // ss[related channel.backpressure-never-drop]
    type MsgSize: Copy + Debug;

    /// The type for a slice of messages to be sent, used in zero-copy operations.
    // ss[related channel.backpressure-never-drop]
    type SliceSource<'b> where Self::MsgOut: 'b;

    /// The type for the target slices where messages are written, typically for zero-copy writes.
    // ss[related channel.backpressure-never-drop]
    type SliceTarget<'a> where Self: 'a;

    /// Marks the channel as closed, preventing further sends.
    ///
    /// This method signals that no more messages will be transmitted, often by notifying receivers
    /// through an oneshot channel. It always returns `true` to indicate the request was processed.
    // ss[related channel.backpressure-never-drop]
    fn shared_mark_closed(&mut self);

    /// Sends messages from an iterator until the channel is full.
    ///
    /// This method processes messages from the provided iterator without blocking, stopping when
    /// the channel reaches capacity. It returns the number of messages successfully sent.
    // ss[related channel.backpressure-never-drop]
    fn shared_send_iter_until_full<'a, I: Iterator<Item = Self::MsgIn<'a>>>(&mut self, iter: I) -> usize;

    /// Determines whether it is time to perform periodic logging.
    ///
    /// This method checks if a sufficient amount of time has elapsed since the last log, based on
    /// a predefined interval, to decide if logging should occur.
    // ss[related channel.backpressure-never-drop]
    fn log_perodic(&mut self) -> bool;

    /// Returns a value representing a single unit for message counting.
    ///
    /// For standard channels, this typically returns `1`. For stream channels, it may return a tuple
    /// representing one control item and an estimated payload size.
    // ss[related channel.backpressure-never-drop]
    fn one(&self) -> Self::MsgSize;

    /// Increments telemetry data based on the number of messages sent.
    ///
    /// This method updates the telemetry based on the `TxDone` value, expecting `Normal` for
    /// standard channels and logging a warning if `Stream` is received unexpectedly.
    // ss[related channel.backpressure-never-drop]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>);

    /// Notifies or resets the monitor, typically by setting a monitor index to a predefined value.
    ///
    /// This method is used to disable or reset monitoring activity for the channel.
    // ss[related channel.backpressure-never-drop]
    fn monitor_not(&mut self);

    /// Returns the capacity of the channel.
    ///
    /// This method provides the total number of messages the channel can hold.
    // ss[related channel.backpressure-never-drop]
    fn shared_capacity(&self) -> Self::MsgSize;

    /// Returns true if the size will fit into the capacity
    // ss[related channel.backpressure-never-drop]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool;

    // ss[impl channel.backpressure-never-drop]
    /// Checks if the channel is full.
    ///
    /// Returns `true` if the channel has reached its capacity and cannot accept more messages.
    // ss[related channel.backpressure-never-drop]
    fn shared_is_full(&self) -> bool;

    /// Checks if the channel is empty.
    ///
    /// Returns `true` if there are no messages currently in the channel.
    // ss[related channel.backpressure-never-drop]
    fn shared_is_empty(&self) -> bool;

    /// Returns the number of vacant units in the channel.
    ///
    /// This method indicates how many more messages can be sent before the channel is full.
    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units(&self) -> Self::MsgSize;

    /// Return true if this message size will fit in the vacant space
    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool;

    /// Waits for either shutdown or for a specified number of units to become vacant.
    ///
    /// This asynchronous method returns `true` if the specified number of units became available,
    /// or `false` if a shutdown signal was received instead.
    #[allow(async_fn_in_trait)]
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_shutdown_or_vacant_units(&mut self, count: Self::MsgSize) -> bool;

    /// Waits until a specified number of units become vacant.
    ///
    /// This asynchronous method blocks until the channel has enough free space to accommodate
    /// the requested number of units, returning `true` when the condition is met.
    #[allow(async_fn_in_trait)]
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_vacant_units(&mut self, count: Self::MsgSize) -> bool;

    /// Waits for the channel to become empty or for a shutdown signal.
    ///
    /// This asynchronous method returns `true` if the channel empties, or `false` if a shutdown
    /// is triggered before the channel becomes empty.
    #[allow(async_fn_in_trait)]
    // ss[related channel.backpressure-never-drop]
    async fn shared_wait_empty(&mut self) -> bool;

    /// Advances the write index by a specified number of units.
    ///
    /// This method is used in zero-copy operations to manually update the write position after
    /// directly writing to the channel's buffer. It returns a `TxDone` value indicating the
    /// number of units advanced.
    // ss[related channel.backpressure-never-drop]
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone;

    /// Sends a slice of messages to the channel.
    ///
    /// This method attempts to send all messages in the provided slice, returning a `TxDone`
    /// value with the number of items successfully sent.
    // ss[related channel.backpressure-never-drop]
    fn shared_send_slice(&mut self, source: Self::SliceSource<'_>) -> TxDone where Self::MsgOut: Copy;

    /// Provides direct access to the vacant slices of the channel for zero-copy writing.
    ///
    /// This method returns the writable portions of the channel's buffer, allowing direct
    /// manipulation of the underlying memory.
    // ss[related channel.backpressure-never-drop]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_>;

    /// Attempts to send a single message without blocking.
    ///
    /// Returns `Ok(TxDone)` if the message was sent successfully, or `Err(Self::MsgOut)` if
    /// the channel is full and the message could not be sent.
    // ss[related channel.backpressure-never-drop]
    fn shared_try_send(&mut self, msg: Self::MsgIn<'_>) -> Result<TxDone, Self::MsgOut>;

    /// Core asynchronous send method with support for timeouts.
    ///
    /// This method attempts to send a message asynchronously, applying the specified saturation
    /// strategy if the channel is full and respecting an optional timeout. It returns a `SendOutcome`
    /// indicating success or failure.
    #[allow(async_fn_in_trait)]
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
    fn done_one(&self, one: &Self::MsgIn<'_>) -> TxDone;
}

/// Implementation of `TxCore` for standard channels (`Tx<T>`).
///
/// This implementation provides the transmission functionality for a standard channel, supporting
/// synchronous and asynchronous message sending, zero-copy operations, and telemetry integration.
// ss[related channel.backpressure-never-drop]
impl<T> TxCore for Tx<T> {
    /// The type of message that can be sent into the channel, matching the channel's generic type.
    // ss[related philosophy.structural-hierarchy]
    type MsgIn<'a> = T;

    /// The type of message that comes out of the channel, identical to `MsgIn` for standard channels.
    // ss[related channel.backpressure-never-drop]
    type MsgOut = T;

    /// The type used to count messages, set to `usize` for standard channels.
    // ss[related channel.backpressure-never-drop]
    type MsgSize = usize;

    /// The type for a slice of messages to be sent, a reference to an array of `T`.
    // ss[related channel.backpressure-never-drop]
    type SliceSource<'b> = &'b [T] where T: 'b;

    /// The type for target slices, providing two mutable slices of uninitialized memory for zero-copy writes.
    // ss[related channel.backpressure-never-drop]
    type SliceTarget<'a> = (&'a mut [std::mem::MaybeUninit<T>], &'a mut [std::mem::MaybeUninit<T>]) where T: 'a;

    /// Advances the write index by the requested number of units, limited by available space.
    ///
    /// This method adjusts the write position in the channel's buffer, ensuring it does not exceed
    /// the vacant space, and returns the number of units advanced.
    // ss[related channel.backpressure-never-drop]
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> TxDone {
        let avail = self.tx.vacant_len();
        let idx = if request > avail { avail } else { request };
        unsafe { self.tx.advance_write_index(idx); }
        TxDone::Normal(idx)
    }

    /// Returns a `TxDone` value indicating one message was processed.
    ///
    /// For standard channels, this always reports a single message sent.
    // ss[related channel.backpressure-never-drop]
    fn done_one(&self, _one: &Self::MsgIn<'_>) -> TxDone {
        TxDone::Normal(1)
    }

    /// Marks the channel as closed by sending a signal through the oneshot channel.
    ///
    /// If the oneshot sender is already taken, it logs a trace message indicating a redundant call.
    /// This method is idempotent and always returns `true`.
    // ss[related channel.backpressure-never-drop]
    fn shared_mark_closed(&mut self) {
        if let Some(c) = self.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                trace!("close called but the receiver already dropped");
            }
        } else {
            trace!("{:?}\n already marked closed, check for redundant calls, ensure mark_closed is called last after all other conditions!", self.channel_meta_data.meta_data);
        }
    }

    /// Returns `1` as the unit value for counting messages.
    ///
    /// This represents a single message in the context of a standard channel.
    // ss[related channel.backpressure-never-drop]
    fn one(&self) -> Self::MsgSize {
        1
    }

    /// Checks if enough time has elapsed since the last error send to allow periodic logging.
    ///
    /// Returns `true` if the elapsed time exceeds the configured maximum telemetry error rate,
    /// resetting the timer, otherwise returns `false`.
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: TxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        // CRITICAL FIX: Resolve lazy index if not yet established
        if self.local_monitor_index == MONITOR_UNKNOWN {
            self.local_monitor_index = crate::monitor::find_my_index(tel, self.channel_meta_data.meta_data.id);
        }
        
        // Only record if we have a valid index
        if self.local_monitor_index < MONITOR_NOT {
            match done_count {
                TxDone::Normal(d) => {
                    self.local_monitor_index = tel.process_event(
                        self.local_monitor_index, 
                        self.channel_meta_data.meta_data.id, 
                        d as isize
                    )
                }
                TxDone::Stream(i, _p) => {
                    warn!("internal error should have gotten Normal");
                    self.local_monitor_index = tel.process_event(
                        self.local_monitor_index, 
                        self.channel_meta_data.meta_data.id, 
                        i as isize
                    )
                }
            }
        } else {
            error!("Telemetry TX channel {} has invalid index {} (MONITOR_UNKNOWN or MONITOR_NOT). Count {:?} lost.", 
                   self.channel_meta_data.meta_data.id, self.local_monitor_index, done_count);
        }
    }

    /// Disables monitoring by setting the local monitor index to a predefined constant.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn monitor_not(&mut self) {
        self.local_monitor_index = MONITOR_NOT;
    }

    /// Returns the total capacity of the channel.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    // ss[related channel.backpressure-never-drop]
    fn shared_capacity_for(&self, size: Self::MsgSize) -> bool {
        let cap = self.shared_capacity();
        size <= cap
    }

    /// Checks if the channel is at full capacity.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }

    /// Checks if the channel contains no messages.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_is_empty(&self) -> bool {
        self.tx.is_empty()
    }

    /// Calculates the number of vacant units in the channel.
    ///
    /// This method uses modulo arithmetic to determine the available space, accounting for
    /// wrap-around in the ring buffer.
    #[inline]
    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units(&self) -> Self::MsgSize {
        let capacity = self.tx.capacity().get();
        let modulus = 2 * capacity;
        let read_idx = self.tx.read_index();
        let write_idx = self.tx.write_index();
        let result = (capacity + read_idx - write_idx) % modulus;
        assert!(result <= capacity);
        result
    }

    // ss[related channel.backpressure-never-drop]
    fn shared_vacant_units_for(&self, size: Self::MsgSize) -> bool {
        let vacant = self.shared_vacant_units();
        vacant >= size
    }


    /// Waits for either a shutdown signal or for the specified number of units to become vacant.
    ///
    /// Returns immediately with `true` if the channel is empty or has enough vacant space.
    /// Otherwise, it waits asynchronously, returning `false` on shutdown or `true` when space is available.
    #[inline]
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
    fn shared_poke_slice(&mut self) -> Self::SliceTarget<'_> {
        self.tx.vacant_slices_mut()
    }

    /// Attempts to send a single message without blocking.
    ///
    /// Returns `Ok` with a `TxDone` value if the message is sent, or `Err` with the message
    /// if the channel is full. Includes a debug assertion to ensure the channel is not closed.
    #[inline]
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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
                    #[allow(deprecated)]
                    SendSaturation::ReturnBlockedMsg => {error!("due to enum"); return SendOutcome::Blocked(msg)},
                    SendSaturation::WarnThenAwait => self.report_tx_full_warning(ident),
                    SendSaturation::DebugWarnThenAwait => {
                        #[cfg(debug_assertions)]
                        self.report_tx_full_warning(ident);
                    }
                }


                let timeout_duration = match timeout {
                    Some(duration) => duration,
                    None => Duration::from_secs(60 * 60 * 24 * 7)
                };

                // CRITICAL FIX: Use .fuse() on the shutdown future to ensure select!
                // correctly tracks termination and avoids spurious readiness.
                let shutdown_fut = (&mut self.oneshot_shutdown).fuse();
                let wait_fut     = self.tx.wait_vacant(1).fuse();
                let timeout_fut  = Delay::new(timeout_duration).fuse();

                pin_mut!(shutdown_fut);
                pin_mut!(wait_fut);
                pin_mut!(timeout_fut);

                select! {
                    // shutdown always wins
                    _ = shutdown_fut => SendOutcome::Closed(msg),

                    // room became available
                    _ = wait_fut => {
                        match self.tx.try_push(msg) {
                            Ok(_) => SendOutcome::Success,
                            Err(t) => SendOutcome::Closed(t), // channel closed or lane reset
                        }
                    }

                    // timeout fallback
                    _ = timeout_fut => SendOutcome::Timeout(msg),
                }
            }
        }
    }

    /// Performs an asynchronous send without a timeout.
    ///
    /// Delegates to the core method with no timeout specified, simplifying the interface for
    /// cases where timing out is not required.
    #[inline]
    // ss[related channel.backpressure-never-drop]
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
    // ss[related channel.backpressure-never-drop]
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

// Unit-tests for the combined TxCore / RxCore behavior


#[cfg(test)]
// ss[related channel.backpressure-never-drop]
#[path = "core_tx_tests.rs"]
mod core_tx_tests;
