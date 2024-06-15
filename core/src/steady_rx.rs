use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use ahash::AHasher;
use futures_util::{FutureExt, select, task};
use std::sync::Arc;
use futures_util::lock::{Mutex, MutexLockFuture};
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

pub struct Rx<T> {
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize, //set on first usage
    pub(crate) is_closed: oneshot::Receiver<()>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
    pub(crate) last_checked_tx_instance: u32,
    pub(crate) tx_version: Arc<AtomicU32>,
    pub(crate) rx_version: Arc<AtomicU32>,
    pub(crate) dedupeset: RefCell<HashSet<String>>,
    pub(crate) peek_hash: AtomicU64, //for bad message detection
    pub(crate) peek_hash_repeats: AtomicUsize,  //for bad message detection
}

impl<T: Hash> Rx<T> {

    /// check if we are peeking the same item multiple times so we know that it should be moved to dead letters
    pub fn possible_bad_message(&self, threshold: usize) -> bool {
        assert_ne!(threshold, 0); //never checked
        assert_ne!(threshold, 1); //we have the first unique item

        //if we have a lot of repeats then we have a problem
        //this is only tracked for the use of 'peek' methods
        self.peek_hash_repeats.load(Ordering::Relaxed) >= threshold
    }

    fn store_item_hash(&self, hasher: AHasher) {
        let hash: u64 = hasher.finish();
        let new_hash = if !hash.is_zero() { hash } else { 1 };
        let old_hash = self.peek_hash.load(Ordering::Relaxed);
        if new_hash != old_hash {
            self.peek_hash.store(new_hash, Ordering::Relaxed);
            self.peek_hash_repeats.store(1, Ordering::Relaxed);
        } else {
            //it already has this hash but we should count occurrences
            self.peek_hash_repeats.fetch_add(1, Ordering::Relaxed);
        }
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
        //may return less that desired if we are shutting down
        self.shared_peek_async_slice(wait_for_count, elems).await
    }


    /// Asynchronously peeks at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Example Usage
    /// Useful for async scenarios where inspecting the next message without consuming it is required.
    pub async fn peek_async(& mut self) -> Option<&T> {
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
    pub async fn peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        self.shared_peek_async_iter(wait_for_count).await

    }
    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Enables iterating over messages for inspection or conditional processing without consuming them.
    pub fn try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        self.shared_try_peek_iter()
    }


    #[inline]
    pub(crate) async fn shared_peek_async(& mut self) -> Option<&T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(1);
            select! { _ = one_down => {}, _ = operation => {}, };
        };
        //we need to hash this in case of failure.
        let result = self.rx.first();

        if let Some(r) = result {
            let mut hasher = AHasher::default();
            r.hash(&mut hasher);
            self.store_item_hash(hasher);
        } else {
            self.clear_item_hash();
        }

        result
    }


    #[inline]
    pub(crate) fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
        where T: Copy {

        //self.rx.occupied_slices()
        // TODO: rewrite when the new version is out

        let mut last_index = 0;
        for (i, e) in self.rx.iter().enumerate() {
            if i < elems.len() {
                elems[i] = *e; // Assuming e is a reference and needs dereferencing
                last_index = i;
            } else {
                break;
            }
        }
        // Return the count of elements written, adjusted for 0-based indexing
        let count = last_index + 1;

        if count>0 {
            let mut hasher = AHasher::default();
            elems[0].hash(&mut hasher);
            self.store_item_hash(hasher);
        } else {
            self.clear_item_hash();
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
        };
        self.shared_try_peek_slice(elems)
    }

    #[inline]
    pub(crate) fn shared_try_peek(&self) -> Option<&T> {
        let result = self.rx.first();

        if let Some(r) = result {
            let mut hasher = AHasher::default();
            r.hash(&mut hasher);
            self.store_item_hash(hasher);
        } else {
            self.clear_item_hash();
        }

        result
    }

}


impl<T> Rx<T> {

    fn clear_item_hash(&self) {
        self.peek_hash.store(0, Ordering::Relaxed);
    }

    pub fn id(&self) -> usize {
        self.channel_meta_data.id
    }

    pub fn tx_instance_changed(&mut self) -> bool {
        let id = self.tx_version.load(Ordering::SeqCst);
        if id == self.last_checked_tx_instance {
            false
        } else {
            //after restart
            //you only get one chance to act on this once detected
            self.last_checked_tx_instance = id;
            true
        }
    }
    pub fn tx_instance_reset(&mut self) {
        let id = self.tx_version.load(Ordering::SeqCst);
        self.last_checked_tx_instance = id;
    }


    /// only for use in unit tests
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

    pub fn is_closed(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            true
        } else {
            // Temporarily create a context to poll the receiver
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            // Non-blocking check if the receiver can resolve
            self.is_closed.poll_unpin(&mut context).is_ready()
        }
    }


    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
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
    pub fn take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_slice(elems)
    }

    pub fn take_into_iter(&mut self) -> impl Iterator<Item = T> + '_ {
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
    pub fn try_take(& mut self) -> Option<T> {
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
    pub async fn take_async(& mut self) -> Option<T> {
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
        //not async and immutable so no need to check
        self.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    ///
    /// # Example Usage
    /// Enables monitoring of the current load or backlog of messages in the channel for adaptive processing strategies.
    pub fn avail_units(& mut self) -> usize {
        //not async and immutable so no need to check
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
    pub async fn wait_avail_units(& mut self, count: usize) -> bool {
        self.shared_wait_avail_units(count).await;
        false
    }
    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            crate::write_warning_to_console( &mut self.dedupeset.borrow_mut());
        }
    }

    ///////////////////////////////////////////////////////////////////
    // these are the shared internal private implementations
    // if you want to swap out the channel implementation you can do it here
    ///////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }




    #[inline]
    pub(crate) fn shared_take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        self.clear_item_hash();
        self.rx.pop_slice(elems)
    }

    #[inline]
    pub(crate) fn shared_take_into_iter(&mut self) -> impl Iterator<Item = T> + '_  {
        self.clear_item_hash();
        self.rx.pop_iter()
    }

    #[inline]
    pub(crate) fn shared_try_take(& mut self) -> Option<T> {
        self.clear_item_hash();
        self.rx.try_pop()
    }

    #[inline]
    pub(crate) fn shared_try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        self.rx.iter()
    }

    #[inline]
    pub(crate) async fn shared_take_async(& mut self) -> Option<T> {
        self.clear_item_hash();
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            select! { _ = one_down => self.rx.try_pop(), p = operation => p, }
        } else {
            self.rx.try_pop()
        }
    }


    #[inline]
    pub(crate) async fn shared_peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            select! { _ = one_down => {}, _ = operation => {}, };
        };
        self.rx.iter()
    }

    #[inline]
    pub(crate) fn shared_is_empty(&self) -> bool {
        self.rx.is_empty()
    }
    #[inline]
    pub(crate) fn shared_avail_units(& mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    pub(crate) async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        if self.rx.occupied_len() >= count {
            true //no need to wait
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut self.rx.wait_occupied(count);
                select! { _ = one_down => false, _ = operation => true }
            } else if self.is_closed() {
                    //shutdown in progress and we are closed
                    false
            } else {
                    //upstream did not mark this closed yet
                    let mut closing = &mut self.is_closed;
                    let mut operation = &mut self.rx.wait_occupied(1);
                    select! { _ = closing => false, _ = operation => true }
            }


        }
    }



}

pub trait RxDef: Debug + Send {
    fn meta_data(&self) -> RxMetaData;
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, (bool,Option<usize>)>;

    }

impl <T: Send + Sync > RxDef for SteadyRx<T>  {
    fn meta_data(&self) -> RxMetaData {
        //used on startup where we want to avoid holding the lock or using another thread
        loop {
            if let Some(guard) = self.try_lock() {
                return RxMetaData(guard.deref().channel_meta_data.clone());
            }
            std::thread::yield_now();
            error!("got stuck");

        }
    }

    ///wait for the correct units and return the true if we got that many
    /// also returns the id of the channel we are working on.
    #[inline]
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, (bool,Option<usize>)> {
        async move {
            if let Some(mut guard) = self.try_lock() {
                let is_closed = guard.deref_mut().is_closed();
                if !is_closed {
                    let result = guard.deref_mut().shared_wait_avail_units(count).await;
                    if result {
                        (true, Some(guard.deref().id()))
                    } else {
                        (false, Some(guard.deref().id())) //we are shutting down so return false
                    }
                } else {
                    (false, None)//do not return id this channel is closed, id is not valid
                }
            } else {
                (false, None)//do not return id, we have no lock.
            }
        }.boxed() // Use the `.boxed()` method to convert the future into a BoxFuture
    }

}

pub trait SteadyRxBundleTrait<T, const GIRTH: usize> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>>;
    fn def_slice(&self) -> [& dyn RxDef; GIRTH];
    fn meta_data(&self) -> [RxMetaData; GIRTH];

    fn wait_avail_units(&self
                                 , avail_count: usize
                                 , ready_channels: usize) -> impl std::future::Future<Output = ()> + Send;
}

impl<T: std::marker::Send + std::marker::Sync, const GIRTH: usize> SteadyRxBundleTrait<T, GIRTH> for SteadyRxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>> {
        //by design we always get the locks in the same order
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }
    fn def_slice(&self) -> [& dyn RxDef; GIRTH]
    {
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
    async fn wait_avail_units(&self
                                                         , avail_count: usize
                                                         , ready_channels: usize)
         {
        let futures = self.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.wait_avail_units(avail_count).await;
            }
                .boxed() // Box the future to make them the same type
        });

        let futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);
        let mut futures = futures;

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
                break;
            }
        }

    }




}


pub trait RxBundleTrait {
    fn is_closed(&mut self) -> bool;

    fn tx_instance_changed(&mut self) -> bool;
    fn tx_instance_reset(&mut self);

}

impl<T> RxBundleTrait for RxBundle<'_, T> {
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed() )
    }

    //TODO: we need docs and examples on how to restart downstream actors.
    fn tx_instance_changed(&mut self) -> bool {
        if self.iter_mut().any(|f| f.tx_instance_changed() ) {
            self.iter_mut().for_each(|f| f.tx_instance_reset() );
            true
        } else {
            false
        }
    }
    fn tx_instance_reset(&mut self) {
        self.iter_mut().for_each(|f| f.tx_instance_reset() );
    }
}
