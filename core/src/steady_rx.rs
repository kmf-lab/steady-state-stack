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

/// Represents a receiver that consumes messages from a channel.
///
/// # Type Parameters
/// - `T`: The type of messages in the channel.
pub struct Rx<T> {
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: RxChannelMetaDataWrapper,
    pub(crate) local_monitor_index: usize,  // Set on first usage
    pub(crate) is_closed: oneshot::Receiver<()>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
    pub(crate) last_checked_tx_instance: u32,
    pub(crate) tx_version: Arc<AtomicU32>,
    pub(crate) rx_version: Arc<AtomicU32>,
    pub(crate) last_error_send: Instant,
    pub(crate) take_count: AtomicU32, // inc upon every take, For bad message detection
    pub(crate) cached_take_count: AtomicU32, // to find repeats, For bad message detection
    pub(crate) peek_repeats: AtomicUsize, // count of repeats, For bad message detection

    pub(crate) iterator_count_drift: Arc<AtomicIsize>, //for RX iterator drift detection to keep telemetry right
}
impl<T> Debug for Rx<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rx") //TODO: add more details
    }
}

impl <T> Rx<T> {

    #[inline]
    pub(crate) fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
    where T: Copy {

        let count:usize = self.rx.peek_slice(elems);

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
    pub(crate) fn shared_try_peek(&self) -> Option<&T> {

        let result = self.rx.try_peek();
        if result.is_some() { ///TODO: this totally wrong !! we needed to check the positon!!!
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

    /// Checks if the same item is being peeked multiple times, indicating it should be moved to dead letters.
    /// DLQ implementation is not provided here, you may want take and log this or choose to drop it.
    ///
    /// # Parameters
    /// - `threshold`: The number of repeats to consider the message as bad.
    ///
    /// # Returns
    /// `true` if the message has been peeked more than the threshold, otherwise `false`.
    pub fn is_showstopper(&self, threshold: usize) -> bool { //for DLQ
        assert_ne!(threshold, 0); // Never checked
        assert_ne!(threshold, 1); // We have the first unique item

        let result = self.rx.try_peek();
        if result.is_some() {

            // If we have a lot of repeats, then we have a problem
            self.peek_repeats.load(Ordering::Relaxed) >= threshold
        } else {
            false
        }
    }


    //TODO: this is a problem used by test macro yet pub!!!
    pub fn try_take(&mut self) -> Option<T> {
        if let Some((_done, msg)) = self.shared_try_take() {
            Some(msg)
        } else {
            None
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




    // #[inline]
    // pub(crate) async fn shared_peek_async_slice_timeout(&mut self, wait_for_count: usize, elems: &mut [T], timeout: Option<Duration>) -> usize
    // where T: Copy
    // {
    //     let mut one_down = &mut self.oneshot_shutdown;
    //     if !one_down.is_terminated() {
    //         let mut operation = &mut self.rx.wait_occupied(wait_for_count);
    //         if let Some(timeout) = timeout {
    //             let mut timeout = Delay::new(timeout).fuse();
    //             select! { _ = one_down => {}
    //                     , _ = operation => {}
    //                     , _ = timeout => {}
    //             }
    //         } else {
    //             select! { _ = one_down => {}
    //                     , _ = operation => {}
    //             }
    //         }
    //     }
    //     self.shared_try_peek_slice(elems)
    // }


}

impl<T> Rx<T> {

    /// Returns the unique identifier of the channel.
    ///
    /// # Returns
    /// A `usize` representing the channel's unique ID.
    pub fn id(&self) -> usize {
        self.channel_meta_data.meta_data.id
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


    /// Checks if the channel is closed.
    ///
    /// # Returns
    /// `true` if the channel is closed, otherwise `false`.
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





    //TODO: confirm these are moved to RxCore??


    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` when a message becomes available.
    /// None is ONLY returned if there is no data AND a shutdown was requested!
    ///
    /// # Asynchronous
    pub(crate) async fn shared_take_async(&mut self) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            select! { _ = one_down => self.rx.try_pop(),  //TODO: this seems very wrong..
                     p = operation => p }
        } else {
            self.rx.try_pop()
        };
        if result.is_some() { //mut be done on every possible take method
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
        }
        result
    }

    pub(crate) async fn shared_take_async_timeout(&mut self, timeout: Option<Duration> ) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let result = if !one_down.is_terminated() {
            let mut operation = &mut self.rx.pop();
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down  => self.rx.try_pop()
                        , p = operation => p
                        , _ = timeout   => self.rx.try_pop()
                }
            } else {
                select! { _ = one_down  => self.rx.try_pop()
                        , p = operation => p
                }
            }
        } else {
            self.rx.try_pop()
        };
        if result.is_some() { //mut be done on every possible take method
            self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
        }
        result
    }



    //  difficult to move because we have dual iterators
    pub(crate) fn shared_take_into_iter(&mut self) -> impl Iterator<Item = T> + '_ {
        CountingIterator::new(self.rx.pop_iter(), &self.take_count)
        // self.rx.pop_iter()
        //TODO: this needs help to do for an iterator.
        // if result.is_some() { //mut be done on every possible take method
        //     self.take_count.fetch_add(1,Ordering::Relaxed); //wraps on overflow
        // }
    }


    //  difficult to move because we have dual iterators and peek
    pub(crate) fn shared_try_peek_iter(&self) -> impl Iterator<Item = &T> {
        self.rx.iter()
    }

    //  difficult to move because we have dual iterators and peek
    pub(crate) async fn _shared_peek_async_iter_timeout(&mut self, wait_for_count: usize, timeout: Option<Duration>) -> impl Iterator<Item = &T> {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.rx.wait_occupied(wait_for_count);
            if let Some(timeout) = timeout {
                let mut timeout = Delay::new(timeout).fuse();
                select! { _ = one_down => {}
                        , _ = operation => {}
                        , _ = timeout => {}
                };
            } else {
                select! { _ = one_down => {}
                        , _ = operation => {}                        
                };
            }
        }
        self.rx.iter()
    }

    //TODO: delete this used by telemtry
    pub(crate) fn deprecated_shared_take_slice(&mut self, elems: &mut [T]) -> usize
    where T: Copy {
        let count = self.rx.pop_slice(elems);
        self.take_count.fetch_add(count as u32, Ordering::Relaxed); //wraps on overflow
        count
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

impl<I> Iterator for CountingIterator<'_,I>
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
pub trait RxMetaDataProvider: Debug {
    /// Retrieves metadata associated with the receiver.
    ///
    /// # Returns
    /// An `RxMetaData` object containing the metadata.
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
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];

    /// Asynchronously waits for a specified number of units to be available in the bundle.
    ///
    /// # Parameters
    /// - `avail_count`: The number of units to wait for.
    /// - `ready_channels`: The number of channels to wait for readiness.
    ///
    /// # Returns
    /// A future that resolves when the specified conditions are met.
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
            .try_into().expect("Wrong number of elements")
    }

    async fn wait_avail_units(&self, avail_count: usize, ready_channels: usize) {
        let futures = self.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.shared_wait_closed_or_avail_units(avail_count).await;
            }.boxed()
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
    /// This is the preferred method for shutdown checks in the is_running method
    ///
    /// # Returns
    /// `true` if all receivers are closed, otherwise `false`.
    fn is_closed_and_empty(&mut self) -> bool;

    /// Checks if all receivers in the bundle are closed.
    ///
    /// # Returns
    /// `true` if all receivers are closed, otherwise `false`.
    fn is_closed(&mut self) -> bool;

    /// Checks if all receivers in the bundle are empty. (avoid)
    /// This may not work since we just attempted to re-define is_empty on a vector of receivers
    /// To solve this please use the preferred is_closed_and_empty method for clarity
    ///
    /// # Returns
    /// `true` if all receivers are empty, otherwise `false`.
    fn is_empty(&mut self) -> bool;
    /// Checks if the Tx instance has changed for any receiver in the bundle.
    ///
    /// # Returns
    /// `true` if the Tx instance has changed for any receiver, otherwise `false`.
    fn tx_instance_changed(&mut self) -> bool;

    /// Resets the Tx instance for all receivers in the bundle.
    fn tx_instance_reset(&mut self);
}

impl<T> RxBundleTrait for RxBundle<'_, T> {

    fn is_closed_and_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed_and_empty())
    }

    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }

    //probably not something you can normally reach
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

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum RxDone {
    Normal(usize),
    Stream(usize,usize)
}
impl RxDone {
    pub fn item_count(&self) -> usize {
        match *self {
            RxDone::Normal(count) => count,
            RxDone::Stream(first, _) => first,
        }
    }
    pub fn payload_count(&self) -> Option<usize> {
        match *self {
            RxDone::Normal(_count) => None,
            RxDone::Stream(_, second) => Some(second),
        }
    }
}
impl Deref for RxDone {
    type Target = usize;

    fn deref(&self) -> &usize {
        match self {
            RxDone::Normal(count) => count,
            RxDone::Stream(first, _) => first,
        }
    }
}
/////////////////////////////////////////////////////////////////


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
        let (lazy_tx_bundle, lazy_rx_bundle) = channel_builder.build_channel_bundle::<String,3>();
        let (steady_tx_bundle0, steady_rx_bundle0) = (lazy_tx_bundle[0].clone(), lazy_rx_bundle[0].clone());
        let (steady_tx_bundle1, steady_rx_bundle1) = (lazy_tx_bundle[1].clone(), lazy_rx_bundle[1].clone());
        let (steady_tx_bundle2, steady_rx_bundle2) = (lazy_tx_bundle[2].clone(), lazy_rx_bundle[2].clone());

        let (steady_tx_bundle, steady_rx_bundle) = (  SteadyTxBundle::new([steady_tx_bundle0, steady_tx_bundle1, steady_tx_bundle2])
                                                     , SteadyRxBundle::new([steady_rx_bundle0, steady_rx_bundle1, steady_rx_bundle2]));

        let array_tx_meta_data = steady_tx_bundle.meta_data();
        let array_rx_meta_data = steady_rx_bundle.meta_data();
        assert_eq!(array_rx_meta_data[0].meta_data().id,array_tx_meta_data[0].meta_data().id);
        assert_eq!(array_rx_meta_data[1].meta_data().id,array_tx_meta_data[1].meta_data().id);
        assert_eq!(array_rx_meta_data[2].meta_data().id,array_tx_meta_data[2].meta_data().id);

        let mut vec_tx_bundle = core_exec::block_on(steady_tx_bundle.lock());
        assert!(vec_tx_bundle[0].shared_try_send("0".to_string()).is_ok());
        assert!(vec_tx_bundle[1].shared_try_send("1".to_string()).is_ok());
        assert!(vec_tx_bundle[2].shared_try_send("2".to_string()).is_ok());

        //if above 3 are not written this will hang
        let mut vec_rx_bundle = core_exec::block_on( async {
            steady_rx_bundle.wait_avail_units(1, 3).await;
            steady_rx_bundle.lock().await
        });
        assert_eq!(3,vec_rx_bundle.len());
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

// Unit tests for Rx<T> convenience methods
#[cfg(test)]
mod steady_rx_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;

    #[test]
    fn test_peek_slice_and_iter() {
        let builder = ChannelBuilder::default().with_capacity(3);
        let (tx_lazy, rx_lazy) = builder.build_channel::<i32>();
        tx_lazy.testing_send_all(vec![5,6,7], false);

        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        let mut buf = [0; 3];
        let n = ste_rx.shared_try_peek_slice(&mut buf);
        assert_eq!(n, 3);
        assert_eq!(buf, [5,6,7]);
        assert!(ste_rx.shared_try_peek().is_some());

        // try_peek_iter
        let collected: Vec<_> = ste_rx.try_peek_iter().cloned().collect();
        assert_eq!(collected, vec![5,6,7]);
    }
}

