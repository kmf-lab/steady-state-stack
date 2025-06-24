use log::*;
use futures_util::{select, task};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use ringbuf::traits::Observer;
use futures_util::future::FusedFuture;
use async_ringbuf::consumer::AsyncConsumer;
use futures_timer::Delay;
use ringbuf::consumer::Consumer;
use crate::monitor_telemetry::SteadyTelemetrySend;
use crate::{steady_config, Rx, MONITOR_NOT};
use crate::distributed::aqueduct_stream::{StreamControlItem, StreamRx};
use crate::steady_rx::{RxDone};
use futures_util::{FutureExt};
use crate::yield_now;

/// Trait providing an interface for working with a pair of slices as a unified data source.
///
/// This trait enables iteration, conversion to a vector, and length calculation over two slices
/// of the same type, typically used to represent the readable portions of a ring buffer in a
/// steady-state system. It supports zero-copy operations by allowing direct access to the data.
pub trait DoubleSlice<'a, T: 'a> {
    /// Returns an iterator over the combined elements of both slices.
    ///
    /// This method provides a way to iterate over all items in the two slices as a single sequence.
    fn as_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a>;

    /// Converts the combined contents of both slices into a single vector.
    ///
    /// This method creates a new vector containing all elements from both slices in order,
    /// requiring the type `T` to be cloneable.
    fn to_vec(&self) -> Vec<T> where T: Clone;

    /// Returns the total number of elements across both slices.
    ///
    /// This method calculates the combined length of the two slices.
    fn total_len(&self) -> usize;
}

/// Implementation of `DoubleSlice` for a pair of slices.
///
/// This implementation treats two slices as a single contiguous data source, supporting iteration,
/// vector conversion, and length calculation.
impl<'a, T: 'a> DoubleSlice<'a, T> for (&'a [T], &'a [T]) {
    fn as_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a> {
        Box::new(self.0.iter().chain(self.1.iter()))
    }
    fn to_vec(&self) -> Vec<T> where T: Clone {
        let mut v = Vec::with_capacity(self.0.len() + self.1.len());
        v.extend_from_slice(self.0);
        v.extend_from_slice(self.1);
        v
    }
    fn total_len(&self) -> usize {
        self.0.len() + self.1.len()
    }
}

/// Trait for copying data from a pair of slices into a single target slice.
///
/// This trait defines a method to efficiently copy data from two source slices into a mutable
/// target slice, typically used in batch operations within a steady-state system.
pub trait DoubleSliceCopy<'a, T: 'a> {
    /// Copies data from the pair of slices into the provided target slice.
    ///
    /// This method fills the target slice with as many elements as possible from the two source
    /// slices, returning the number of items copied. It is designed for efficient data transfer
    /// and requires the type `T` to be copyable.
    fn copy_into_slice(&self, target: &mut [T]) -> RxDone where T: Copy;
}

/// Implementation of `DoubleSliceCopy` for a pair of slices.
///
/// This implementation handles copying from two source slices into a single target slice,
/// ensuring the target is filled to its capacity or until the source data is exhausted.
impl<'a, T: 'a> DoubleSliceCopy<'a, T> for (&'a [T], &'a [T]) {
    fn copy_into_slice(&self, target: &mut [T]) -> RxDone where T: Copy {
        let (a, b) = *self;
        let mut copied = 0;

        let n = a.len().min(target.len());
        if n > 0 {
            target[..n].copy_from_slice(&a[..n]);
            copied += n;
        }

        if copied < target.len() {
            let m = b.len().min(target.len() - copied);
            if m > 0 {
                target[copied..copied + m].copy_from_slice(&b[..m]);
                copied += m;
            }
        }

        RxDone::Normal(copied)
    }
}

/// Trait for working with four slices representing two data types, typically items and payloads.
///
/// This trait provides methods to iterate over, convert to vectors, and calculate lengths for
/// two pairs of slices: one pair for items and another for payloads. It is designed for use in
/// stream-based systems where control items and their associated payload bytes are handled together.
pub trait QuadSlice<'a, T: 'a, U: 'a> {
    /// Returns an iterator over the combined items from the first pair of slices.
    ///
    /// This method allows iteration over all items as a single sequence.
    fn items_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a>;

    /// Returns an iterator over the combined payload elements from the second pair of slices.
    ///
    /// This method provides a unified view of the payload data across both slices.
    fn payload_iter(&'a self) -> Box<dyn Iterator<Item = &'a U> + 'a>;

    /// Converts the combined items from the first pair of slices into a single vector.
    ///
    /// This method creates a new vector of items, requiring the type `T` to be cloneable.
    fn items_vec(&self) -> Vec<T> where T: Clone;

    /// Converts the combined payloads from the second pair of slices into a single vector.
    ///
    /// This method creates a new vector of payload elements, requiring the type `U` to be cloneable.
    fn payload_vec(&self) -> Vec<U> where U: Clone;

    /// Returns the total number of items across the first pair of slices.
    ///
    /// This method calculates the combined length of the item slices.
    fn items_len(&self) -> usize;

    /// Returns the total number of payload elements across the second pair of slices.
    ///
    /// This method calculates the combined length of the payload slices.
    fn payload_len(&self) -> usize;
}

/// Implementation of `QuadSlice` for four slices representing items and payloads.
///
/// This implementation treats two pairs of slices (items and payloads) as unified data sources,
/// supporting iteration, vector conversion, and length calculation for both item and payload data.
impl<'a, T: 'a, U: 'a> QuadSlice<'a, T, U> for (&'a [T], &'a [T], &'a [U], &'a [U]) {
    fn items_iter(&'a self) -> Box<dyn Iterator<Item = &'a T> + 'a> {
        Box::new(self.0.iter().chain(self.1.iter()))
    }
    fn payload_iter(&'a self) -> Box<dyn Iterator<Item = &'a U> + 'a> {
        Box::new(self.2.iter().chain(self.3.iter()))
    }
    fn items_vec(&self) -> Vec<T> where T: Clone {
        let mut v = Vec::with_capacity(self.0.len() + self.1.len());
        v.extend_from_slice(self.0);
        v.extend_from_slice(self.1);
        v
    }
    fn payload_vec(&self) -> Vec<U> where U: Clone {
        let mut v = Vec::with_capacity(self.2.len() + self.3.len());
        v.extend_from_slice(self.2);
        v.extend_from_slice(self.3);
        v
    }
    fn items_len(&self) -> usize {
        self.0.len() + self.1.len()
    }
    fn payload_len(&self) -> usize {
        self.2.len() + self.3.len()
    }
}

/// Trait for copying items and their payloads from four slices into target slices.
///
/// This trait defines a method to copy control items and their associated payload bytes from
/// a quadruple of slices into two target slices, ensuring synchronized data transfer in stream-based systems.
pub trait StreamQuadSliceCopy<'a, T: StreamControlItem> {
    /// Copies items and their payloads into the provided target slices.
    ///
    /// This method fills the item and payload target slices with as much data as possible,
    /// respecting the length constraints of the payloads associated with each item. It returns
    /// a tuple indicating the number of items and bytes copied.
    fn copy_items_and_payloads(&self, item_target: &mut [T], payload_target: &mut [u8]) -> (usize, usize) where T: Copy;
}

/// Implementation of `StreamQuadSliceCopy` for four slices representing items and payloads.
///
/// This implementation handles the synchronized copying of control items and their payload bytes
/// from two pairs of slices into respective target slices, ensuring that payload lengths match
/// the requirements of each item.
impl<'a, T: StreamControlItem> StreamQuadSliceCopy<'a, T> for (&'a [T], &'a [T], &'a [u8], &'a [u8]) {
    fn copy_items_and_payloads(&self, item_target: &mut [T], payload_target: &mut [u8]) -> (usize, usize) where T: Copy {
        let (a, b, c, d) = self;
        let items_iter = a.iter().chain(b.iter());
        let mut items_copied = 0;
        let mut bytes_needed = 0;

        for item in items_iter.clone() {
            let item_len = item.length() as usize;
            if items_copied < item_target.len() && bytes_needed + item_len <= payload_target.len() {
                items_copied += 1;
                bytes_needed += item_len;
            } else {
                break;
            }
        }

        let mut copied = 0;
        for item in items_iter.take(items_copied) {
            item_target[copied] = *item;
            copied += 1;
        }

        let mut payload_copied = 0;
        let n = c.len().min(bytes_needed);
        if n > 0 {
            payload_target[..n].copy_from_slice(&c[..n]);
            payload_copied += n;
        }
        if payload_copied < bytes_needed {
            let m = d.len().min(bytes_needed - payload_copied);
            if m > 0 {
                payload_target[payload_copied..payload_copied + m].copy_from_slice(&d[..m]);
                payload_copied += m;
            }
        }

        (items_copied, payload_copied)
    }
}

/// Trait defining the core functionality for receiving data in a steady-state system.
///
/// This trait provides a standardized interface for receiving messages, managing channel state,
/// and interacting with telemetry in a steady-state actor system. It is designed to be implemented
/// by types that handle data reception, such as standard channels (`Rx<T>`) and stream-based
/// channels (`StreamRx<StreamControlItem>`). The trait supports both synchronous and asynchronous
/// operations, as well as zero-copy mechanisms through slice-based methods.
pub trait RxCore {
    /// The type of message item stored in the channel.
    type MsgItem;

    /// The type of message that is taken out of the channel.
    type MsgOut;

    /// The type used to peek at a message without removing it.
    type MsgPeek<'a> where Self: 'a;

    /// The type used to represent the size or count of messages, typically `usize` for standard
    /// channels or a tuple for streams.
    type MsgSize: Copy;

    /// The type for a slice of messages to be peeked at, used in zero-copy operations.
    type SliceSource<'a> where Self: 'a;

    /// The type for the target slice where messages are copied during take operations.
    type SliceTarget<'b> where Self::MsgOut: 'b;

    /// Copies available messages from the channel into the provided target slice.
    ///
    /// This method attempts to fill the target slice with messages from the channel, returning
    /// the number of messages copied. It is used for efficient batch processing and requires
    /// the message item type to be copyable.
    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy;

    /// Provides direct access to the available slices of the channel for zero-copy peeking.
    ///
    /// This method returns the readable portions of the channel’s buffer, allowing direct access
    /// to the underlying memory without copying.
    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_>;

    /// Asynchronously peeks at the next message with an optional timeout.
    ///
    /// This method waits for a message to become available or for a shutdown signal, returning
    /// a reference to the message if available within the timeout period.
    #[allow(async_fn_in_trait)]
    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>>;

    /// Increments telemetry data based on the number of messages received.
    ///
    /// This method updates the telemetry system with the count of messages or bytes received,
    /// depending on the channel type and the provided `RxDone` value.
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>);

    /// Notifies or resets the monitor, typically by setting a monitor index to a predefined value.
    ///
    /// This method is used to disable or reset monitoring activity for the channel.
    fn monitor_not(&mut self);

    /// Returns the capacity of the channel.
    ///
    /// This method provides the total number of messages the channel can hold.
    fn shared_capacity(&self) -> usize;

    /// Determines whether it is time to perform periodic logging.
    ///
    /// This method checks if a sufficient amount of time has elapsed since the last log, based on
    /// a predefined interval, to decide if logging should occur.
    fn log_periodic(&mut self) -> bool;

    /// Checks if the channel is empty.
    ///
    /// Returns `true` if there are no messages currently in the channel.
    fn shared_is_empty(&self) -> bool;

    /// Returns the number of available units in the channel.
    ///
    /// This method indicates how many messages are ready to be taken from the channel.
    fn shared_avail_units(&mut self) -> Self::MsgSize;

    /// Waits for either shutdown or for a specified number of units to become available.
    ///
    /// This asynchronous method returns `true` if the specified number of units became available,
    /// or `false` if a shutdown signal was received instead.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool;

    /// Waits for either the channel to close or for a specified number of units to become available.
    ///
    /// This method returns `true` if the units became available, or `false` if the channel was closed
    /// before the units were available.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool;

    /// Waits until a specified number of units become available.
    ///
    /// This asynchronous method blocks until the channel has at least the requested number of units,
    /// returning `true` when the condition is met.
    #[allow(async_fn_in_trait)]
    async fn shared_wait_avail_units(&mut self, count: usize) -> bool;

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// Returns `Some((RxDone, Self::MsgOut))` if a message was taken, or `None` if the channel is empty.
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)>;

    /// Advances the read index by a specified number of units.
    ///
    /// This method is used in zero-copy operations to manually update the read position after
    /// directly accessing the channel’s buffer. It returns an `RxDone` value indicating the
    /// number of units advanced.
    fn shared_advance_index(&mut self, request: Self::MsgSize) -> RxDone;

    /// Checks if the channel is both closed and empty.
    ///
    /// This method returns `true` if the channel has been closed and contains no more messages,
    /// indicating that no further data will be received.
    fn is_closed_and_empty(&mut self) -> bool;
}

/// Implementation of `RxCore` for standard channels (`Rx<T>`).
///
/// This implementation provides the reception functionality for a standard channel, supporting
/// synchronous and asynchronous message receiving, zero-copy operations, and telemetry integration.
impl<T> RxCore for Rx<T> {
    /// The type of message item stored in the channel, matching the channel’s generic type.
    type MsgItem = T;

    /// The type of message that is taken out of the channel, identical to `MsgItem`.
    type MsgOut = T;

    /// The type used to peek at a message, a reference to `T`.
    type MsgPeek<'a> = &'a T where T: 'a;

    /// The type used to count messages, set to `usize` for standard channels.
    type MsgSize = usize;

    /// The type for a slice of messages to be peeked at, a pair of references to arrays of `T`.
    type SliceSource<'a> = (&'a [T], &'a [T]) where Self: 'a;

    /// The type for the target slice where messages are copied, a mutable reference to an array of `T`.
    type SliceTarget<'b> = &'b mut [T] where T: 'b;

    fn is_closed_and_empty(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            self.shared_is_empty()
        } else if self.shared_is_empty() {
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            self.is_closed.poll_unpin(&mut context).is_ready()
        } else {
            false
        }
    }

    fn shared_advance_index(&mut self, request: Self::MsgSize) -> RxDone {
        let avail = self.rx.occupied_len();
        let idx = if request > avail { avail } else { request };
        unsafe { self.rx.advance_read_index(idx); }
        RxDone::Normal(idx)
    }

    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(1);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}, _ = operation => {}, _ = timeout => {} };
            } else {
                select! { _ = one_down => {}, _ = operation => {} };
            }
        }
        let result = self.rx.first();
        if result.is_some() {
            let take_count = self.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.cached_take_count.load(Ordering::Relaxed);
            if cached_take_count != take_count {
                self.peek_repeats.store(0, Ordering::Relaxed);
                self.cached_take_count.store(take_count, Ordering::Relaxed);
            } else {
                self.peek_repeats.fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.peek_repeats.store(0, Ordering::Relaxed);
        }
        result
    }

    fn log_periodic(&mut self) -> bool {
        if self.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.last_error_send = Instant::now();
            true
        }
    }

    #[inline]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        self.local_monitor_index = match done_count {
            RxDone::Normal(d) => tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, d as isize),
            RxDone::Stream(i, _p) => {
                warn!("internal error should have gotten Normal");
                tel.process_event(self.local_monitor_index, self.channel_meta_data.meta_data.id, i as isize)
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.local_monitor_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> usize {
        let capacity = self.rx.capacity().get();
        let modulus = 2 * capacity;
        let write_idx = self.rx.write_index();
        let read_idx = self.rx.read_index();
        let result = (modulus + write_idx - read_idx) % modulus;
        assert!(result <= capacity);
        result
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.rx.occupied_len() >= count
        }
    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        if self.rx.occupied_len() >= count {
            true
        } else {
            let mut one_closed = &mut self.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.rx.wait_occupied(count);
                select! { _ = one_closed => self.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await;
                self.rx.occupied_len() >= count
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        if self.rx.occupied_len() >= count {
            true
        } else if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            false
        }
    }

    #[inline]
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        match self.rx.try_pop() {
            Some(r) => {
                self.take_count.fetch_add(1, Ordering::Relaxed);
                Some((RxDone::Normal(1), r))
            },
            None => None
        }
    }

    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        let (a, b) = self.rx.as_slices();
        let mut copied = 0;
        let n = a.len().min(target.len());
        if n > 0 {
            let target = unsafe { &mut *(target as *const [T] as *mut [T]) };
            target[..n].copy_from_slice(&a[..n]);
            copied += n;
        }
        if copied < target.len() {
            let m = b.len().min(target.len() - copied);
            if m > 0 {
                let target = unsafe { &mut *(target as *const [T] as *mut [T]) };
                target[copied..copied + m].copy_from_slice(&b[..m]);
                copied += m;
            }
        }
        unsafe { self.rx.advance_read_index(copied); }
        if copied > 0 {
            self.take_count.fetch_add(1, Ordering::Relaxed);
        }
        RxDone::Normal(copied)
    }

    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_> {
        self.rx.as_slices()
    }
}

/// Implementation of `RxCore` for stream-based channels (`StreamRx<T>`).
///
/// This implementation manages a dual-channel system with a control channel for `T: StreamControlItem`
/// and a payload channel for byte data, ensuring synchronized reception of control messages and
/// their associated payloads.
impl<T: StreamControlItem> RxCore for StreamRx<T> {
    /// The type of message item stored in the control channel.
    type MsgItem = T;

    /// The type of message that is taken out of the channel, a tuple of the control item and its payload.
    type MsgOut = (T, Box<[u8]>);

    /// The type used to peek at a message, a tuple of references to the control item and its payload slices.
    type MsgPeek<'a> = (&'a T, &'a [u8], &'a [u8]) where T: 'a;

    /// The type used to count messages, a tuple of control items and payload bytes.
    type MsgSize = (usize, usize);

    /// The type for a slice of messages to be peeked at, a quadruple of slices for control and payload.
    type SliceSource<'a> = (&'a [T], &'a [T], &'a [u8], &'a [u8]) where T: 'a;

    /// The type for the target slices where messages are copied, a pair of mutable slices for control and payload.
    type SliceTarget<'b> = (&'b mut [T], &'b mut [u8]) where T: 'b;

    fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty() && self.payload_channel.is_closed_and_empty()
    }

    fn shared_advance_index(&mut self, count: Self::MsgSize) -> RxDone {
        let control_avail = self.control_channel.rx.occupied_len();
        let payload_avail = self.payload_channel.rx.occupied_len();
        if count.0 <= control_avail && count.1 <= payload_avail {
            unsafe {
                self.payload_channel.rx.advance_read_index(count.1);
                self.control_channel.rx.advance_read_index(count.0);
            }
            RxDone::Stream(count.0, count.1)
        } else {
            RxDone::Stream(0, 0)
        }
    }

    async fn shared_peek_async_timeout(&mut self, timeout: Option<Duration>) -> Option<Self::MsgPeek<'_>> {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(1);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}, _ = operation => {}, _ = timeout => {} };
            } else {
                select! { _ = one_down => {}, _ = operation => {} };
            }
        }
        let result = self.control_channel.rx.first();
        if let Some(item) = result {
            let take_count = self.control_channel.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.control_channel.cached_take_count.load(Ordering::Relaxed);
            if cached_take_count != take_count {
                self.control_channel.peek_repeats.store(0, Ordering::Relaxed);
                self.control_channel.cached_take_count.store(take_count, Ordering::Relaxed);
            } else {
                self.control_channel.peek_repeats.fetch_add(1, Ordering::Relaxed);
            }
            let (a, b) = self.payload_channel.rx.as_slices();
            let count_a = a.len().min(item.length() as usize);
            let count_b = item.length() as usize - count_a;
            Some((item, &a[0..count_a], &b[0..count_b]))
        } else {
            self.control_channel.peek_repeats.store(0, Ordering::Relaxed);
            None
        }
    }

    fn log_periodic(&mut self) -> bool {
        if self.control_channel.last_error_send.elapsed().as_secs() < steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            false
        } else {
            self.control_channel.last_error_send = Instant::now();
            true
        }
    }

    #[inline]
    fn telemetry_inc<const LEN: usize>(&mut self, done_count: RxDone, tel: &mut SteadyTelemetrySend<LEN>) {
        match done_count {
            RxDone::Normal(i) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
            }
            RxDone::Stream(i, p) => {
                self.control_channel.local_monitor_index = tel.process_event(self.control_channel.local_monitor_index, self.control_channel.channel_meta_data.meta_data.id, i as isize);
                self.payload_channel.local_monitor_index = tel.process_event(self.payload_channel.local_monitor_index, self.payload_channel.channel_meta_data.meta_data.id, p as isize);
            },
        }
    }

    #[inline]
    fn monitor_not(&mut self) {
        self.control_channel.local_monitor_index = MONITOR_NOT;
        self.payload_channel.local_monitor_index = MONITOR_NOT;
    }

    fn shared_capacity(&self) -> usize {
        self.control_channel.rx.capacity().get()
    }

    fn shared_is_empty(&self) -> bool {
        self.control_channel.rx.is_empty()
    }

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        (self.control_channel.rx.occupied_len(), self.payload_channel.rx.occupied_len())
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.control_channel.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.control_channel.rx.wait_occupied(count);
            select! { _ = one_down => false, _ = operation => true }
        } else {
            self.control_channel.rx.occupied_len() >= count
        }
    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        if self.control_channel.rx.occupied_len() >= count {
            true
        } else {
            let mut one_closed = &mut self.control_channel.is_closed;
            if !one_closed.is_terminated() {
                let mut operation = &mut self.control_channel.rx.wait_occupied(count);
                select! { _ = one_closed => self.control_channel.rx.occupied_len() >= count, _ = operation => true }
            } else {
                yield_now::yield_now().await;
                self.control_channel.rx.occupied_len() >= count
            }
        }
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        if self.control_channel.rx.occupied_len() >= count {
            true
        } else {
            let operation = &mut self.control_channel.rx.wait_occupied(count);
            operation.await;
            true
        }
    }

    #[inline]
    fn shared_try_take(&mut self) -> Option<(RxDone, Self::MsgOut)> {
        if let Some(item) = self.control_channel.rx.try_peek() {
            if item.length() <= self.payload_channel.rx.occupied_len() as i32 {
                let mut payload = vec![0u8; item.length() as usize];
                self.payload_channel.rx.peek_slice(&mut payload);
                let payload = payload.into_boxed_slice();
                if let Some(item) = self.control_channel.rx.try_pop() {
                    unsafe { self.payload_channel.rx.advance_read_index(payload.len()); }
                    self.control_channel.take_count.fetch_add(1, Ordering::Relaxed);
                    Some((RxDone::Stream(1, payload.len()), (item, payload)))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    fn shared_take_slice(&mut self, target: Self::SliceTarget<'_>) -> RxDone where Self::MsgItem: Copy {
        let (item_target, payload_target) = target;
        let (item_a, item_b) = self.control_channel.rx.as_slices();
        let mut items_copied = 0;
        let mut payload_bytes_needed = 0;
        let max_items = item_target.len();
        let max_payload = payload_target.len();

        for item in item_a {
            let item_len = item.length() as usize;
            if items_copied < max_items && payload_bytes_needed + item_len <= max_payload {
                item_target[items_copied] = *item;
                items_copied += 1;
                payload_bytes_needed += item_len;
            } else {
                break;
            }
        }
        for item in item_b {
            let item_len = item.length() as usize;
            if items_copied < max_items && payload_bytes_needed + item_len <= max_payload {
                item_target[items_copied] = *item;
                items_copied += 1;
                payload_bytes_needed += item_len;
            } else {
                break;
            }
        }

        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        let mut payload_copied = 0;
        let n = payload_a.len().min(payload_bytes_needed);
        if n > 0 {
            payload_target[..n].copy_from_slice(&payload_a[..n]);
            payload_copied += n;
        }
        if payload_copied < payload_bytes_needed {
            let m = payload_b.len().min(payload_bytes_needed - payload_copied);
            if m > 0 {
                payload_target[payload_copied..payload_copied + m].copy_from_slice(&payload_b[..m]);
                payload_copied += m;
            }
        }

        unsafe {
            self.payload_channel.rx.advance_read_index(payload_copied);
            self.control_channel.rx.advance_read_index(items_copied);
        }

        self.control_channel.take_count.fetch_add(payload_copied as u32, Ordering::Relaxed);
        self.payload_channel.take_count.fetch_add(items_copied as u32, Ordering::Relaxed);

        RxDone::Stream(items_copied, payload_copied)
    }

    fn shared_peek_slice(&mut self) -> Self::SliceSource<'_> {
        let (item_a, item_b) = self.control_channel.rx.as_slices();
        let (payload_a, payload_b) = self.payload_channel.rx.as_slices();
        (item_a, item_b, payload_a, payload_b)
    }
}

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

    fn shared_capacity(&self) -> usize {
        <T as RxCore>::shared_capacity(&**self)
    }

    fn shared_is_empty(&self) -> bool {
        <T as RxCore>::shared_is_empty(&**self)
    }

    fn shared_avail_units(&mut self) -> Self::MsgSize {
        <T as RxCore>::shared_avail_units(&mut **self)
    }

    async fn shared_wait_shutdown_or_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_shutdown_or_avail_units(&mut **self, count).await
    }

    async fn shared_wait_closed_or_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_closed_or_avail_units(&mut **self, count).await
    }

    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        <T as RxCore>::shared_wait_avail_units(&mut **self, count).await
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
}

/// Unit tests for asynchronous operations in `RxCore` implementations.
///
/// This module contains tests to verify the correctness of asynchronous methods in the `RxCore`
/// trait, focusing on peeking, waiting for available units, and handling shutdown or closure scenarios.
#[cfg(test)]
mod core_rx_async_tests {
    use super::*;
    use crate::steady_rx::Rx;
    use crate::steady_tx::Tx;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use futures::lock::Mutex;
    use crate::core_tx::TxCore;
    use crate::{ActorIdentity, SendSaturation};
    use crate::*;

    /// Helper function to set up a channel for tests.
    ///
    /// This function creates a channel with the specified capacity and optionally populates it
    /// with initial data. It returns the sender and receiver wrapped in mutexes for shared access,
    /// along with the graph instance used for testing.
    fn setup_channel<T: Clone + Send + 'static>(
        capacity: usize,
        data: Option<Vec<T>>,
    ) -> (Arc<Mutex<Tx<T>>>, Arc<Mutex<Rx<T>>>, Graph) {
        let mut graph = GraphBuilder::for_testing().build(());
        let builder = graph.channel_builder();
        let builder = builder.with_capacity(capacity);
        let (tx_lazy, rx_lazy) = builder.build_channel::<T>();
        let tx = tx_lazy.clone();
        let rx = rx_lazy.clone();
        if let Some(values) = data {
            core_exec::block_on(async {
                let mut tx_guard = tx.lock().await;
                for value in values {
                    let _ = tx_guard.shared_send_async(value, ActorIdentity::default(), SendSaturation::default()).await;
                }
            });
        }
        graph.start();
        (tx, rx, graph)
    }

    /// Tests peeking with a timeout on an empty channel.
    ///
    /// Verifies that the peek operation times out correctly and returns `None` when no data is available.
    #[test]
    fn test_peek_async_timeout_empty() {
        let (_tx, rx, graph) = setup_channel::<i32>(1, None);
        assert!(graph.runtime_state.read().is_in_state(&[GraphLivelinessState::Running]), "Graph should be Running");
        let mut rx_guard = core_exec::block_on(rx.lock());
        assert!(!rx_guard.oneshot_shutdown.is_terminated());

        let start = Instant::now();
        let peeked = core_exec::block_on(rx_guard.shared_peek_async_timeout(Some(Duration::from_millis(120))));

        assert!(peeked.is_none(), "Peek should return None on an empty channel after timeout or shutdown");
        eprintln!("timeout {:?}", start.elapsed());
        assert!(start.elapsed() >= Duration::from_millis(100), "Timeout duration should be at least 100ms");
    }

    /// Tests peeking with data already present in the channel.
    ///
    /// Ensures that the peek operation immediately returns the available message.
    #[test]
    fn test_peek_async_timeout_with_data() {
        let (_tx, rx, _) = setup_channel(1, Some(vec![42]));
        let mut rx_guard = core_exec::block_on(rx.lock());
        let peeked = core_exec::block_on(rx_guard.shared_peek_async_timeout(None));
        assert_eq!(peeked, Some(&42), "Peek should return the available data");
    }

    /// Tests peeking during a shutdown scenario.
    ///
    /// Verifies that the peek operation returns `None` after the channel is shut down.
    #[test]
    fn test_peek_async_timeout_shutdown() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let peek_future = rx_guard.shared_peek_async_timeout(Some(Duration::from_secs(1)));
        drop(core_exec::block_on(tx.lock()));
        let peeked = core_exec::block_on(peek_future);
        assert!(peeked.is_none(), "Peek should return None after shutdown");
    }

    /// Tests waiting for available units.
    ///
    /// Ensures that the wait operation completes when the required number of units becomes available.
    #[test]
    fn test_wait_avail_units() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let wait_future = rx_guard.shared_wait_avail_units(1);
        core_exec::block_on(async {
            let mut tx_guard = tx.lock().await;
            tx_guard.shared_send_async(42, ActorIdentity::default(), SendSaturation::default()).await;
        });
        assert!(core_exec::block_on(wait_future), "Wait should return true when units become available");
    }

    /// Tests waiting for the channel to close or for units to become available.
    ///
    /// Verifies that the wait operation returns `false` when the channel is closed without the
    /// required units being available.
    #[test]
    fn test_wait_closed_or_avail_units() {
        let (tx, rx, _) = setup_channel::<i32>(1, None);
        let mut rx_guard = core_exec::block_on(rx.lock());
        let wait_future = rx_guard.shared_wait_closed_or_avail_units(1);
        core_exec::block_on(async {
            let mut tx_guard = tx.lock().await;
            tx_guard.mark_closed();
        });
        let result = core_exec::block_on(wait_future);
        assert!(!result, "Wait should return false when channel is closed with no units available");
    }
}