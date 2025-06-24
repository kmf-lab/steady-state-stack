use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use futures_util::{select, task, FutureExt};
use std::sync::Arc;
use futures_util::lock::{Mutex, MutexLockFuture};
use std::time::{Duration, Instant};
use log::error;
use futures::channel::oneshot;
use futures_util::future::{select_all, FusedFuture};
use async_ringbuf::consumer::AsyncConsumer;
use ringbuf::consumer::Consumer;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref};
use futures_timer::Delay;
use crate::channel_builder::InternalReceiver;
use crate::monitor::{ChannelMetaData};
use crate::core_rx::RxCore;
use crate::{RxBundle, SteadyRxBundle};
use crate::distributed::distributed_stream::RxChannelMetaDataWrapper;

/// Represents a receiver that consumes messages from a channel in a steady-state actor system.
///
/// This struct manages the reception of messages from a channel, providing methods for peeking,
/// taking, and checking the state of the channel. It includes support for telemetry, shutdown
/// detection, and error handling, making it suitable for high-performance, distributed systems.
///
/// # Type Parameters
/// - `T`: The type of messages that the channel can hold.
pub struct Rx<T> {
    /// Internal ring buffer consumer for managing message reception.
    pub(crate) rx: InternalReceiver<T>,
    /// Metadata wrapper for the channel, used in logging and telemetry.
    pub(crate) channel_meta_data: RxChannelMetaDataWrapper,
    /// Index for local monitoring, set on first usage.
    pub(crate) local_monitor_index: usize,
    /// Oneshot receiver that signals when the channel is closed.
    pub(crate) is_closed: oneshot::Receiver<()>,
    /// Oneshot receiver that detects shutdown signals for graceful termination.
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
    /// Tracks the last checked instance of the transmitter to detect changes.
    pub(crate) last_checked_tx_instance: u32,
    /// Atomic counter shared with the transmitter to monitor its version.
    pub(crate) tx_version: Arc<AtomicU32>,
    /// Atomic counter for the receiver's version, used for synchronization.
    pub(crate) rx_version: Arc<AtomicU32>,
    /// Timestamp of the last error sent, used to rate-limit error logging.
    pub(crate) last_error_send: Instant,
    /// Atomic counter incremented on each message take, for bad message detection.
    pub(crate) take_count: AtomicU32,
    /// Cached take count to detect repeated peeks without takes.
    pub(crate) cached_take_count: AtomicU32,
    /// Atomic counter tracking repeated peeks of the same message.
    pub(crate) peek_repeats: AtomicUsize,
    /// Atomic counter for detecting iterator usage drift in telemetry.
    pub(crate) iterator_count_drift: Arc<AtomicIsize>,
}

impl<T> Debug for Rx<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rx") // TODO: Add more details for debugging purposes.
    }
}

impl<T> Rx<T> {
    /// Attempts to peek at a slice of messages from the channel without removing them.
    ///
    /// This method fills the provided mutable slice with as many messages as possible from the channel,
    /// up to the slice's length, without consuming them. It updates internal counters for bad message
    /// detection and returns the number of messages copied. If no messages are available, it resets the
    /// peek repeat counter.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice where peeked messages are stored.
    ///
    /// # Returns
    /// The number of messages copied into the slice.
    ///
    /// # Requirements
    /// The message type `T` must implement the `Copy` trait.
    #[inline]
    pub(crate) fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
    where
        T: Copy,
    {
        let count: usize = self.rx.peek_slice(elems);

        if count > 0 {
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
        count
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// This method checks if a message is available and returns a reference to it if present. It updates
    /// internal counters to monitor repeated peeks, which helps identify messages that might need special
    /// handling, such as moving to a dead letter queue. If no message is available, it resets the peek
    /// repeat counter.
    ///
    /// # Returns
    /// `Some(&T)` if a message is available, `None` if the channel is empty.
    #[inline]
    pub(crate) fn shared_try_peek(&self) -> Option<&T> {
        let result = self.rx.try_peek();
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

    /// Checks if the same message is being peeked multiple times, indicating a potential bad message.
    ///
    /// This method determines if the number of repeated peeks exceeds a specified threshold, suggesting
    /// that the message might be problematic and should be handled differently, such as logging or dropping it.
    /// It relies on the peek repeat counter and only evaluates if a message is present.
    ///
    /// # Parameters
    /// - `threshold`: The number of repeated peeks after which the message is considered bad.
    ///
    /// # Returns
    /// `true` if the peek count exceeds the threshold, `false` otherwise.
    pub fn is_showstopper(&self, threshold: usize) -> bool {
        assert_ne!(threshold, 0, "Threshold must be greater than zero.");
        assert_ne!(threshold, 1, "Threshold of one is not meaningful for detection.");

        if self.rx.try_peek().is_some() {
            self.peek_repeats.load(Ordering::Relaxed) >= threshold
        } else {
            false
        }
    }

    /// Attempts to take a single message from the channel.
    ///
    /// This convenience method removes and returns the next available message from the channel. It wraps
    /// a core take operation and is useful for straightforward message consumption scenarios.
    ///
    /// # Returns
    /// `Some(T)` if a message is available, `None` if the channel is empty.
    pub fn try_take(&mut self) -> Option<T> {
        if let Some((_done, msg)) = self.shared_try_take() {
            Some(msg)
        } else {
            None
        }
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// This method provides a public interface to inspect the next message without consuming it, useful
    /// for decision-making or conditional processing before taking the message.
    ///
    /// # Returns
    /// `Some(&T)` if a message is available, `None` if the channel is empty.
    pub fn try_peek(&self) -> Option<&T> {
        self.shared_try_peek()
    }

    /// Attempts to peek at a slice of messages from the channel without removing them.
    ///
    /// This method allows inspecting multiple messages at once by filling a provided mutable slice.
    /// It is non-blocking and useful for batch inspection or processing decisions without consumption.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice to store the peeked messages, with its length setting the maximum number of messages to peek.
    ///
    /// # Returns
    /// The number of messages copied into the slice.
    ///
    /// # Requirements
    /// The message type `T` must implement the `Copy` trait.
    pub fn try_peek_slice(&self, elems: &mut [T]) -> usize
    where
        T: Copy,
    {
        self.shared_try_peek_slice(elems)
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// This method provides an iterator for inspecting all available messages in the channel, enabling
    /// examination or filtering before deciding to consume them.
    ///
    /// # Returns
    /// An iterator yielding references to the messages in the channel.
    pub fn try_peek_iter(&self) -> impl Iterator<Item = &T> {
        self.shared_try_peek_iter()
    }
}

impl<T> Rx<T> {
    /// Returns the unique identifier of the channel.
    ///
    /// Each channel has a unique ID, which is useful for logging, debugging, or associating telemetry
    /// data with specific channels.
    ///
    /// # Returns
    /// The channel’s unique ID as a `usize`.
    pub fn id(&self) -> usize {
        self.channel_meta_data.meta_data.id
    }

    /// Checks if the transmitter instance has changed since the last check.
    ///
    /// This method detects if the transmitter has been updated or replaced, which might indicate a
    /// configuration change or restart. It updates the last checked instance if a change is detected.
    ///
    /// # Returns
    /// `true` if the transmitter instance has changed, `false` otherwise.
    pub fn tx_instance_changed(&mut self) -> bool {
        let id = self.tx_version.load(Ordering::SeqCst);
        if id == self.last_checked_tx_instance {
            false
        } else {
            self.last_checked_tx_instance = id;
            true
        }
    }

    /// Resets the last checked transmitter instance to the current instance.
    ///
    /// This method updates the receiver’s internal state to reflect the current transmitter instance,
    /// typically called after handling a detected change.
    pub fn tx_instance_reset(&mut self) {
        let id = self.tx_version.load(Ordering::SeqCst);
        self.last_checked_tx_instance = id;
    }

    /// Checks if the channel is both closed and empty.
    ///
    /// This method verifies if the channel is closed (no more messages will be sent) and has no remaining
    /// messages to process. It is essential for shutdown procedures to ensure all data is handled before termination.
    ///
    /// # Returns
    /// `true` if the channel is closed and empty, `false` otherwise.
    pub fn is_closed_and_empty(&mut self) -> bool {
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

    /// Checks if the channel is closed.
    ///
    /// This method determines if the channel has been closed, meaning no further messages will be sent,
    /// without checking for remaining messages.
    ///
    /// # Returns
    /// `true` if the channel is closed, `false` otherwise.
    pub fn is_closed(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            true
        } else {
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            self.is_closed.poll_unpin(&mut context).is_ready()
        }
    }

    /// Checks if the channel is currently empty.
    ///
    /// This method indicates whether there are any messages available in the channel, helping decide
    /// whether to wait for messages or proceed with other tasks.
    ///
    /// # Returns
    /// `true` if the channel is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.shared_is_empty()
    }

    /// Returns the total capacity of the channel.
    ///
    /// This method provides the maximum number of messages the channel can hold, aiding in configuration
    /// analysis and bottleneck identification.
    ///
    /// # Returns
    /// The channel’s capacity as a `usize`.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// This method indicates how many messages are ready to be consumed, assisting in load management
    /// and task prioritization.
    ///
    /// # Returns
    /// The number of available messages as a `usize`.
    pub fn avail_units(&mut self) -> usize {
        self.shared_avail_units()
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// This method waits until a message is available or a shutdown is requested. It is designed for
    /// asynchronous workflows where blocking is undesirable.
    ///
    /// # Returns
    /// `Some(T)` if a message is available, `None` if the channel is closed and empty.
    pub(crate) async fn shared_take_async(&mut self) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            select! { _ = one_down => self.rx.try_pop(), p = operation => p }
        } else {
            self.rx.try_pop()
        };
        if result.is_some() {
            self.take_count.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Asynchronously retrieves and removes a single message from the channel with an optional timeout.
    ///
    /// This method waits for a message up to a specified duration, returning early if the timeout expires
    /// or a shutdown occurs.
    ///
    /// # Parameters
    /// - `timeout`: An optional duration to wait for a message.
    ///
    /// # Returns
    /// `Some(T)` if a message is available within the timeout, `None` if the timeout expires or the channel is closed and empty.
    pub(crate) async fn shared_take_async_timeout(&mut self, timeout: Option<Duration>) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! {
                    _ = one_down => self.rx.try_pop(),
                    p = operation => p,
                    _ = timeout => self.rx.try_pop()
                }
            } else {
                select! {
                    _ = one_down => self.rx.try_pop(),
                    p = operation => p
                }
            }
        } else {
            self.rx.try_pop()
        };
        if result.is_some() {
            self.take_count.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Provides an iterator that takes messages from the channel.
    ///
    /// This method returns an iterator that removes and yields messages until the channel is empty,
    /// suitable for batch processing of all available messages.
    ///
    /// # Returns
    /// An iterator yielding the messages in the channel.
    pub(crate) fn shared_take_into_iter(&mut self) -> impl Iterator<Item = T> + '_ {
        CountingIterator::new(self.rx.pop_iter(), &self.take_count)
    }

    /// Provides an iterator over the messages currently in the channel without removing them.
    ///
    /// This method allows inspecting all available messages without consumption, mirroring the behavior
    /// of `try_peek_iter` but used internally.
    ///
    /// # Returns
    /// An iterator yielding references to the messages in the channel.
    pub(crate) fn shared_try_peek_iter(&self) -> impl Iterator<Item = &T> {
        self.rx.iter()
    }

    /// Asynchronously waits for a specified number of messages to be available, then returns an iterator over them.
    ///
    /// This method waits until the channel has at least the specified number of messages or a shutdown
    /// occurs, then provides an iterator for peeking at the available messages.
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for.
    /// - `timeout`: An optional duration to wait before timing out.
    ///
    /// # Returns
    /// An iterator yielding references to the messages in the channel.
    pub(crate) async fn _shared_peek_async_iter_timeout(
        &mut self,
        wait_for_count: usize,
        timeout: Option<Duration>,
    ) -> impl Iterator<Item = &T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! {
                    _ = one_down => {},
                    _ = operation => {},
                    _ = timeout => {}
                };
            } else {
                select! {
                    _ = one_down => {},
                    _ = operation => {}
                };
            }
        }
        self.rx.iter()
    }

    /// Takes a slice of messages from the channel (deprecated).
    ///
    /// This method removes multiple messages into a provided slice and updates the take counter. It is
    /// marked as deprecated and should be avoided in new code.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice to store the taken messages.
    ///
    /// # Returns
    /// The number of messages taken.
    ///
    /// # Requirements
    /// The message type `T` must implement the `Copy` trait.
    pub(crate) fn deprecated_shared_take_slice(&mut self, elems: &mut [T]) -> usize
    where
        T: Copy,
    {
        let count = self.rx.pop_slice(elems);
        self.take_count.fetch_add(count as u32, Ordering::Relaxed);
        count
    }
}

/// An iterator that counts the number of items taken from the channel.
///
/// This struct wraps an existing iterator and increments a counter each time an item is yielded,
/// aiding in telemetry or logging of message processing.
pub(crate) struct CountingIterator<'a, I> {
    iter: I,
    counter: &'a AtomicU32,
}

impl<'a, I> CountingIterator<'a, I> {
    /// Creates a new counting iterator with the given iterator and counter.
    ///
    /// This constructor initializes the iterator with the provided components, linking it to the take
    /// counter for tracking.
    ///
    /// # Parameters
    /// - `iter`: The iterator to wrap.
    /// - `count`: The atomic counter to increment.
    ///
    /// # Returns
    /// A new `CountingIterator` instance.
    pub fn new(iter: I, count: &'a AtomicU32) -> Self {
        CountingIterator { iter, counter: count }
    }
}

impl<I> Iterator for CountingIterator<'_, I>
where
    I: Iterator,
{
    type Item = I::Item;

    /// Yields the next item from the underlying iterator and increments the counter.
    ///
    /// Each time an item is retrieved, the counter is updated atomically to reflect the number of
    /// messages processed.
    ///
    /// # Returns
    /// `Some(I::Item)` if an item is available, `None` if the iterator is exhausted.
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next();
        if item.is_some() {
            self.counter.fetch_add(1, Ordering::Relaxed);
        }
        item
    }
}

/// Trait defining the required methods for providing receiver metadata.
pub trait RxMetaDataProvider: Debug {
    /// Retrieves metadata associated with the receiver.
    ///
    /// This method provides access to the channel’s metadata, such as its ID and name, for use in
    /// logging, debugging, and telemetry.
    ///
    /// # Returns
    /// A shared reference to the channel’s metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl<T: Send + Sync> RxMetaDataProvider for Mutex<Rx<T>> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            std::thread::yield_now();
            error!("got stuck");
        }
    }
}

impl<T: Send + Sync> RxMetaDataProvider for Arc<Mutex<Rx<T>>> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            std::thread::yield_now();
            error!("got stuck");
        }
    }
}

/// Trait defining the required methods for a steady receiver bundle.
pub trait SteadyRxBundleTrait<T, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// This method prepares all receivers for synchronized access, returning a future that resolves
    /// when all locks are acquired.
    ///
    /// # Returns
    /// A future resolving to a collection of mutex guards for the receivers.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// This method collects metadata from each receiver, enabling comprehensive channel information access.
    ///
    /// # Returns
    /// An array of references to metadata providers, one for each receiver.
    fn meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];

    /// Asynchronously waits for a specified number of units to be available in the bundle.
    ///
    /// This method waits until a certain number of receivers have at least the specified number of
    /// messages available, coordinating multi-channel operations.
    ///
    /// # Parameters
    /// - `avail_count`: The number of messages to wait for in each channel.
    /// - `ready_channels`: The number of channels that must meet the availability condition.
    ///
    /// # Returns
    /// A future that resolves when the conditions are met.
    fn wait_avail_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()>;
}

impl<T: Send + Sync, const GIRTH: usize> SteadyRxBundleTrait<T, GIRTH> for SteadyRxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn RxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Wrong number of elements")
    }

    async fn wait_avail_units(&self, avail_count: usize, ready_channels: usize) {
        let futures = self.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.shared_wait_closed_or_avail_units(avail_count).await;
            }
                .boxed()
        });

        let mut count_down = ready_channels.min(GIRTH);
        let mut futures: Vec<_> = futures.collect();
        while !futures.is_empty() {
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if count_down == 0 {
                break;
            }
        }
    }
}

/// Trait defining the required methods for a receiver bundle.
pub trait RxBundleTrait {
    /// Checks if all receivers in the bundle are closed and empty.
    ///
    /// This method is critical for shutdown checks, ensuring all channels have completed processing.
    ///
    /// # Returns
    /// `true` if all receivers are closed and empty, `false` otherwise.
    fn is_closed_and_empty(&mut self) -> bool;

    /// Checks if all receivers in the bundle are closed.
    ///
    /// This method verifies if all channels have ceased accepting new messages.
    ///
    /// # Returns
    /// `true` if all receivers are closed, `false` otherwise.
    fn is_closed(&mut self) -> bool;

    /// Checks if all receivers in the bundle are empty.
    ///
    /// This method indicates if no messages remain in any channel, though it is less reliable for
    /// shutdown checks than `is_closed_and_empty`.
    ///
    /// # Returns
    /// `true` if all receivers are empty, `false` otherwise.
    fn is_empty(&mut self) -> bool;

    /// Checks if the transmitter instance has changed for any receiver in the bundle.
    ///
    /// This method detects updates or replacements in any transmitter, triggering resets if necessary.
    ///
    /// # Returns
    /// `true` if any transmitter instance has changed, `false` otherwise.
    fn tx_instance_changed(&mut self) -> bool;

    /// Resets the transmitter instance for all receivers in the bundle.
    ///
    /// This method updates the internal state of all receivers to reflect current transmitter instances,
    /// typically after a change is handled.
    fn tx_instance_reset(&mut self);
}

impl<T> RxBundleTrait for RxBundle<'_, T> {
    fn is_closed_and_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed_and_empty())
    }

    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }

    fn is_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_empty())
    }

    fn tx_instance_changed(&mut self) -> bool {
        if self.iter_mut().any(|f| f.tx_instance_changed()) {
            self.iter_mut().for_each(|f| f.tx_instance_reset());
            true
        } else {
            false
        }
    }

    fn tx_instance_reset(&mut self) {
        self.iter_mut().for_each(|f| f.tx_instance_reset());
    }
}

/// Represents the outcome of a receive operation in a steady-state system.
///
/// This enum distinguishes between normal and stream-based receive operations, providing item and
/// byte counts for telemetry or logging purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxDone {
    /// Indicates a standard receive operation with the number of items received.
    Normal(usize),
    /// Indicates a stream receive operation with the number of items and bytes received.
    Stream(usize, usize),
}

impl RxDone {
    /// Returns the number of items received.
    ///
    /// This method extracts the item count from either variant, providing a consistent way to access
    /// this information.
    ///
    /// # Returns
    /// The number of items as a `usize`.
    pub fn item_count(&self) -> usize {
        match *self {
            RxDone::Normal(count) => count,
            RxDone::Stream(first, _) => first,
        }
    }

    /// Returns the number of payload bytes received, if applicable.
    ///
    /// This method retrieves the byte count for stream operations, returning `None` for normal operations.
    ///
    /// # Returns
    /// `Some(usize)` for stream operations, `None` for normal operations.
    pub fn payload_count(&self) -> Option<usize> {
        match *self {
            RxDone::Normal(_) => None,
            RxDone::Stream(_, second) => Some(second),
        }
    }
}

impl Deref for RxDone {
    type Target = usize;

    /// Allows dereferencing to the item count.
    ///
    /// This enables treating `RxDone` as its item count in contexts where a `usize` is expected.
    fn deref(&self) -> &usize {
        match self {
            RxDone::Normal(count) => count,
            RxDone::Stream(first, _) => first,
        }
    }
}

#[cfg(test)]
mod rx_tests {
    use crate::core_rx::RxCore;
    use crate::core_tx::TxCore;
    use super::*;
    use crate::*;

    #[test]
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
}

#[cfg(test)]
mod steady_rx_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;

    #[test]
    fn test_peek_slice_and_iter() {
        let builder = ChannelBuilder::default().with_capacity(3);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(vec![5, 6, 7], false);

        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        let mut buf = [0; 3];
        let n = ste_rx.shared_try_peek_slice(&mut buf);
        assert_eq!(n, 3);
        assert_eq!(buf, [5, 6, 7]);
        assert!(ste_rx.shared_try_peek().is_some());

        let collected: Vec<_> = ste_rx.try_peek_iter().cloned().collect();
        assert_eq!(collected, vec![5, 6, 7]);
    }
}