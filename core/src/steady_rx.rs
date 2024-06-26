use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicIsize, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use futures_util::{FutureExt, select, task};
use std::sync::Arc;
use futures_util::lock::{MutexLockFuture};
use std::time::{Duration, Instant};
use log::error;
use futures::channel::oneshot;
use std::cell::RefCell;
use std::collections::HashSet;
use num_traits::Zero;
use futures_util::future::{BoxFuture, FusedFuture, select_all};
use async_ringbuf::consumer::AsyncConsumer;
use ringbuf::consumer::Consumer;
use ringbuf::traits::Observer;
use std::fmt::Debug;
use std::ops::{Deref, DerefMut};
use crate::channel_builder::InternalReceiver;
use crate::monitor::{ChannelMetaData, RxMetaData};
use crate::{RxBundle, SteadyRx, SteadyRxBundle};

/// Represents a receiver that consumes messages from a channel.
///
/// # Type Parameters
/// - `T`: The type of messages in the channel.
pub struct Rx<T> {
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize, // Set on first usage
    pub(crate) is_closed: oneshot::Receiver<()>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
    pub(crate) last_checked_tx_instance: u32,
    pub(crate) tx_version: Arc<AtomicU32>,
    pub(crate) rx_version: Arc<AtomicU32>,
    pub(crate) internal_warn_dedupe_set: RefCell<HashSet<String>>, // could be removed someday

    pub(crate) take_count: AtomicU32, // inc upon every take, For bad message detection
    pub(crate) cached_take_count: AtomicU32, // to find repeats, For bad message detection
    pub(crate) peek_repeats: AtomicUsize, // count of repeats, For bad message detection

    pub(crate) iterator_count_drift: Arc<AtomicIsize>, //for RX iterator drift detection to keep telemetry right
}

impl <T> Rx<T> {

    /// Checks if the same item is being peeked multiple times, indicating it should be moved to dead letters.
    ///
    /// # Parameters
    /// - `threshold`: The number of repeats to consider the message as bad.
    ///
    /// # Returns
    /// `true` if the message has been peeked more than the threshold, otherwise `false`.
    pub fn possible_bad_message(&self, threshold: usize) -> bool {
        assert_ne!(threshold, 0); // Never checked
        assert_ne!(threshold, 1); // We have the first unique item

        // If we have a lot of repeats, then we have a problem
        self.peek_repeats.load(Ordering::Relaxed) >= threshold
    }


    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Can be used to inspect the next message without consuming it, useful in decision-making scenarios.
    pub fn try_peek(&self) -> Option<&T> {
        self.shared_try_peek()
    }

    /// Attempts to peek at a slice of messages from the channel without removing them.
    /// This operation is non-blocking and allows multiple messages to be inspected simultaneously.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `elems`: A mutable slice to store the peeked messages. Its length determines the maximum number of messages to peek.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Ideal for scenarios where processing or decision making is based on a batch of incoming messages without consuming them.
    pub fn try_peek_slice(&self, elems: &mut [T]) -> usize
        where T: Copy {
        self.shared_try_peek_slice(elems)
    }

    /// Asynchronously waits to peek at a slice of messages from the channel without removing them.
    /// Waits until the specified number of messages is available or the channel is closed.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Suitable for use cases requiring a specific number of messages to be available for batch processing.
    pub async fn peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
        where T: Copy {
        self.shared_peek_async_slice(wait_for_count, elems).await
    }

    /// Asynchronously peeks at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Example Usage
    /// Useful for async scenarios where inspecting the next message without consuming it is required.
    pub async fn peek_async(&mut self) -> Option<&T> {
        self.shared_peek_async().await
    }

    /// Asynchronously returns an iterator over the messages in the channel, waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Ideal for async batch processing where a specific number of messages are needed for processing.
    pub async fn peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &T> {
        self.shared_peek_async_iter(wait_for_count).await
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Enables iterating over messages for inspection or conditional processing without consuming them.
    pub fn try_peek_iter(&self) -> impl Iterator<Item = &T> {
        self.shared_try_peek_iter()
    }

    #[inline]
    pub(crate) async fn shared_peek_async(&mut self) -> Option<&T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(1);
            select! { _ = one_down => {}, _ = operation => {}, };
        }
        let result = self.rx.first();

        if let Some(r) = result {

            let take_count = self.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
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

    #[inline]
    pub(crate) fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
        where T: Copy {
        let mut last_index = 0;
        for (i, e) in self.rx.iter().enumerate() {
            if i < elems.len() {
                elems[i] = *e; // Assuming e is a reference and needs dereferencing
                last_index = i;
            } else {
                break;
            }
        }
        let count = last_index + 1;

        if count > 0 {
            let take_count = self.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
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

    #[inline]
    pub(crate) async fn shared_peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
        where T: Copy {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            select! { _ = one_down => {}, _ = operation => {}, };
        }
        self.shared_try_peek_slice(elems)
    }

    #[inline]
    pub(crate) fn shared_try_peek(&self) -> Option<&T> {
        let result = self.rx.first();
        if let Some(r) = result {
            let take_count = self.take_count.load(Ordering::Relaxed);
            let cached_take_count = self.cached_take_count.load(Ordering::Relaxed);
            if !cached_take_count == take_count {
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
}

impl<T> Rx<T> {

    /// Returns the unique identifier of the channel.
    ///
    /// # Returns
    /// A `usize` representing the channel's unique ID.
    pub fn id(&self) -> usize {
        self.channel_meta_data.id
    }

    /// Checks if the Tx instance has changed since the last check.
    ///
    /// # Returns
    /// `true` if the Tx instance has changed, otherwise `false`.
    pub fn tx_instance_changed(&mut self) -> bool {
        let id = self.tx_version.load(Ordering::SeqCst);
        if id == self.last_checked_tx_instance {
            false
        } else {
            self.last_checked_tx_instance = id;
            true
        }
    }

    /// Resets the last checked Tx instance to the current Tx instance.
    pub fn tx_instance_reset(&mut self) {
        let id = self.tx_version.load(Ordering::SeqCst);
        self.last_checked_tx_instance = id;
    }

    /// Only for use in unit tests.
    pub fn block_until_not_empty(&self, duration: Duration) {
        assert!(cfg!(debug_assertions), "This function is only for testing");
        let start = Instant::now();
        loop {
            if !self.is_empty() {
                break;
            }
            if start.elapsed() > duration {
                error!("timeout waiting for channel to not be empty");
                break;
            }
            std::thread::yield_now();
        }
    }

    /// Checks if the channel is closed.
    ///
    /// # Returns
    /// `true` if the channel is closed, otherwise `false`.
    pub fn is_closed(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            true
        } else {
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            self.is_closed.poll_unpin(&mut context).is_ready()
        }
    }

    /// Returns the total capacity of the channel.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// Useful for initial configuration and monitoring of channel capacity to ensure it aligns with expected load.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Retrieves and removes a slice of messages from the channel.
    /// This operation is blocking and will remove the messages from the channel.
    /// Note: May take fewer messages than the slice length if the channel is not full.
    /// This method requires `T` to implement the `Copy` trait, ensuring efficient handling of message data.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This constraint ensures that messages can be efficiently copied out of the channel without ownership issues.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `elems`.
    ///
    /// # Example Usage
    /// Useful for batch processing where multiple messages are consumed at once for efficiency. Particularly effective in scenarios where processing overhead needs to be minimized.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Performance Optimization**: Facilitates quick, lightweight operations by leveraging the ability to copy data directly, without the overhead of more complex memory management.
    /// - **Simplicity in Message Handling**: Reduces the complexity of managing message lifecycles and ownership, streamlining channel operations.
    /// - **Reliability**: Ensures that the operation does not inadvertently alter or lose message data during transfer, maintaining message integrity.
    ///
    /// ## Cons:
    /// - **Type Limitation**: Limits the use of the channel to data types that implement `Copy`, potentially excluding more complex or resource-managed types that might require more nuanced handling.
    /// - **Overhead for Larger Types**: While `Copy` is intended for lightweight types, using it with larger, though still `Copy`, data types could introduce unnecessary copying overhead.
    /// - **Design Rigidity**: Imposes a strict design requirement on the types that can be used with the channel, which might not align with all application designs or data models.
    ///
    /// Incorporating the `T: Copy` requirement, `take_slice` is optimized for use cases prioritizing efficiency and simplicity in handling a series of lightweight messages, making it a key method for high-performance message processing tasks.
    pub(crate) fn take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_slice(elems)
    }

    /// Retrieves and removes messages from the channel into an iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Useful for scenarios where messages need to be processed in a streaming manner.
    pub(crate) fn take_into_iter(&mut self) -> impl Iterator<Item = T> + '_ {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_into_iter()
    }

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Ideal for non-blocking single message consumption, allowing for immediate feedback on message availability.
    pub(crate) fn try_take(&mut self) -> Option<T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_take()
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Returns
    /// A `Result<T, String>`, where `Ok(T)` is the message if available, and `Err(String)` contains an error message if the retrieval fails.
    ///
    /// # Example Usage
    /// Suitable for async processing where messages are consumed one at a time as they become available.
    pub(crate) async fn take_async(&mut self) -> Option<T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_async().await
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    ///
    /// # Example Usage
    /// Useful for determining if the channel is empty before attempting to consume messages.
    pub fn is_empty(&self) -> bool {
        self.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    ///
    /// # Example Usage
    /// Enables monitoring of the current load or backlog of messages in the channel for adaptive processing strategies.
    pub fn avail_units(&mut self) -> usize {
        self.shared_avail_units()
    }

    /// Asynchronously waits for a specified number of messages to be available in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of messages to wait for.
    ///
    /// # Example Usage
    /// Suitable for scenarios requiring batch processing where a certain number of messages are needed before processing begins.
    #[inline]
    pub async fn wait_avail_units(&mut self, count: usize) -> bool {
        self.shared_wait_avail_units(count).await;
        false
    }

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            crate::write_warning_to_console(&mut self.internal_warn_dedupe_set.borrow_mut());
        }
    }

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    #[inline]
    pub(crate) fn shared_take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        let count = self.rx.pop_slice(elems);
        self.take_count.fetch_add(count as u32, Ordering::Relaxed); //wraps on overflow
        count
    }

    #[inline]
    pub(crate) fn shared_take_into_iter(&mut self) -> impl Iterator<Item = T> + '_ {
        CountingIterator::new(self.rx.pop_iter(), &mut self.take_count)
    }

    #[inline]
    pub(crate) fn shared_try_take(&mut self) -> Option<T> {
        let result = self.rx.try_pop();
        if result.is_some() {
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
        }
        result
    }

    #[inline]
    pub(crate) fn shared_try_peek_iter(&self) -> impl Iterator<Item = &T> {
        self.rx.iter()
    }

    #[inline]
    pub(crate) async fn shared_take_async(&mut self) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            select! { _ = one_down => self.rx.try_pop(), p = operation => p, }
        } else {
            self.rx.try_pop()
        };
        if result.is_some() {
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
        }
        result
    }

    #[inline]
    pub(crate) async fn shared_peek_async_iter(&mut self, wait_for_count: usize) -> impl Iterator<Item = &T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            select! { _ = one_down => {}, _ = operation => {}, };
        }
        self.rx.iter()
    }

    #[inline]
    pub(crate) fn shared_is_empty(&self) -> bool {
        self.rx.is_empty()
    }

    #[inline]
    pub(crate) fn shared_avail_units(&mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    pub(crate) async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        if self.rx.occupied_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut self.rx.wait_occupied(count);
                select! { _ = one_down => false, _ = operation => true }
            } else if self.is_closed() {
                false
            } else {
                let mut closing = &mut self.is_closed;
                let mut operation = &mut self.rx.wait_occupied(1);
                select! { _ = closing => false, _ = operation => true }
            }
        }
    }
}


pub(crate) struct CountingIterator<'a, I> {
    iter: I,
    counter: &'a AtomicU32,
}

impl<'a, I> CountingIterator<'a, I> {
    pub fn new(iter: I, count: &'a AtomicU32) -> Self {
        CountingIterator {
            iter,
            counter: count,
        }
    }
}

impl<'a, I> Iterator for CountingIterator<'a, I>
where
    I: Iterator,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next();
        if item.is_some() {
            self.counter.fetch_add(1,Ordering::Relaxed); // wraps on overflow
        }
        item
    }
}


/// Trait defining the required methods for a receiver definition.
pub trait RxDef: Debug + Send {
    /// Retrieves metadata associated with the receiver.
    ///
    /// # Returns
    /// An `RxMetaData` object containing the metadata.
    fn meta_data(&self) -> RxMetaData;

    /// Asynchronously waits for a specified number of units to be available in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// A `BoxFuture` that resolves to a tuple containing a boolean indicating success and an optional channel ID.
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, (bool, Option<usize>)>;
}

impl<T: Send + Sync> RxDef for SteadyRx<T> {
    fn meta_data(&self) -> RxMetaData {
        loop {
            if let Some(guard) = self.try_lock() {
                return RxMetaData(guard.deref().channel_meta_data.clone());
            }
            std::thread::yield_now();
            error!("got stuck");
        }
    }

    #[inline]
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, (bool, Option<usize>)> {
        async move {
            if let Some(mut guard) = self.try_lock() {
                let is_closed = guard.deref_mut().is_closed();
                if !is_closed {
                    let result = guard.deref_mut().shared_wait_avail_units(count).await;
                    if result {
                        (true, Some(guard.deref().id()))
                    } else {
                        (false, Some(guard.deref().id()))
                    }
                } else {
                    (false, None)
                }
            } else {
                (false, None)
            }
        }
            .boxed()
    }
}

/// Trait defining the required methods for a steady receiver bundle.
pub trait SteadyRxBundleTrait<T, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>>;

    /// Retrieves a slice of receiver definitions.
    ///
    /// # Returns
    /// A slice of receiver definitions.
    fn def_slice(&self) -> [&dyn RxDef; GIRTH];

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn meta_data(&self) -> [RxMetaData; GIRTH];

    /// Asynchronously waits for a specified number of units to be available in the bundle.
    ///
    /// # Parameters
    /// - `avail_count`: The number of units to wait for.
    /// - `ready_channels`: The number of channels to wait for readiness.
    ///
    /// # Returns
    /// A future that resolves when the specified conditions are met.
    fn wait_avail_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()> + Send;
}

impl<T: Send + Sync, const GIRTH: usize> SteadyRxBundleTrait<T, GIRTH> for SteadyRxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn def_slice(&self) -> [&dyn RxDef; GIRTH] {
        self.iter()
            .map(|x| x as &dyn RxDef)
            .collect::<Vec<&dyn RxDef>>()
            .try_into()
            .expect("Internal Error")
    }

    fn meta_data(&self) -> [RxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data())
            .collect::<Vec<RxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

    async fn wait_avail_units(&self, avail_count: usize, ready_channels: usize) {
        let futures = self.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.wait_avail_units(avail_count).await;
            }
                .boxed()
        });

        let futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);
        let mut futures = futures;

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
    /// Checks if all receivers in the bundle are closed.
    ///
    /// # Returns
    /// `true` if all receivers are closed, otherwise `false`.
    fn is_closed(&mut self) -> bool;

    /// Checks if the Tx instance has changed for any receiver in the bundle.
    ///
    /// # Returns
    /// `true` if the Tx instance has changed for any receiver, otherwise `false`.
    fn tx_instance_changed(&mut self) -> bool;

    /// Resets the Tx instance for all receivers in the bundle.
    fn tx_instance_reset(&mut self);
}

impl<T> RxBundleTrait for RxBundle<'_, T> {
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
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
