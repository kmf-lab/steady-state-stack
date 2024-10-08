use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use log::{error, trace, warn};
use futures_util::{FutureExt, select};
use std::any::type_name;
use std::backtrace::Backtrace;
use std::time::Instant;
use futures::channel::oneshot;
use futures_util::lock::{MutexLockFuture};
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use futures_util::future::{FusedFuture, select_all};
use async_ringbuf::producer::AsyncProducer;
use std::fmt::Debug;
use std::ops::Deref;
use std::thread;
use crate::{ActorIdentity, SendSaturation, SteadyTx, SteadyTxBundle, TxBundle};
use crate::channel_builder::InternalSender;
use crate::monitor::{ChannelMetaData, TxMetaData};

/// The `Tx` struct represents a transmission channel for messages of type `T`.
/// It provides methods to send messages to the channel, check the channel's state, and handle the transmission lifecycle.
pub struct Tx<T> {
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize, //set on first usage
    pub(crate) last_error_send: Instant,
    pub(crate) make_closed: Option<oneshot::Sender<()>>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
    pub(crate) last_checked_rx_instance: u32,
    pub(crate) tx_version: Arc<AtomicU32>,
    pub(crate) rx_version: Arc<AtomicU32>,
}

impl<T> Tx<T> {

    /// Returns the unique identifier of the transmission channel.
    ///
    /// # Returns
    /// A `usize` representing the channel's unique ID.
    pub fn id(&self) -> usize {
        self.channel_meta_data.id
    }

    /// Checks if the receiver instance has changed.
    ///
    /// # Returns
    /// `true` if the receiver instance has changed, otherwise `false`.
    pub fn rx_instance_changed(&mut self) -> bool {
        let id = self.rx_version.load(Ordering::SeqCst);
        if id == self.last_checked_rx_instance {
            false
        } else {
            // After restart, you only get one chance to act on this once detected
            self.last_checked_rx_instance = id;
            true
        }
    }

    /// Resets the receiver instance tracking to the current state.
    pub fn rx_instance_reset(&mut self) {
        let id = self.rx_version.load(Ordering::SeqCst);
        self.last_checked_rx_instance = id;
    }

    /// Marks the channel as closed, indicating no more messages are expected.
    ///
    /// # Returns
    /// `true` if the channel was successfully marked as closed, otherwise `false`.
    pub fn mark_closed(&mut self) -> bool {
        if let Some(c) = self.make_closed.take() {
            let result = c.send(());
            if result.is_err() {
                //not a serious issue, may happen with bundles
                trace!("close called but the receiver already dropped");
            }
        } 
        true // always returns true, close request is never rejected by this method.        
    }

    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// This method is useful for understanding the size constraints of the channel
    /// and for configuring buffer sizes or for performance tuning.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /*
    /// Attempts to send a message to the channel without blocking.
    /// If the channel is full, the send operation will fail and return the message.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    ///
    /// # Example Usage
    /// Use this method for non-blocking send operations where immediate feedback on send success is required.
    /// Not suitable for scenarios where ensuring message delivery is critical without additional handling for failed sends.
    pub(crate) fn try_send(&mut self, msg: T) -> Result<(), T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_send(msg)
    }

    /// Sends messages from an iterator to the channel until the channel is full.
    /// Each message is sent in a non-blocking manner.
    ///
    /// # Parameters
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Ideal for batch sending operations where partial success is acceptable.
    /// Less suitable when all messages must be sent without loss, as this method does not guarantee all messages are sent.
    pub(crate) fn send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_iter_until_full(iter)
    }

    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `ident`: The identity of the actor sending the message. Get this from the SteadyContext.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    pub(crate) async fn send_async(&mut self, ident: ActorIdentity, a: T, saturation: SendSaturation) -> Result<(), T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_async(a, ident, saturation).await
    }

    /// Sends messages from a slice to the channel until the channel is full.
    /// Operates in a non-blocking manner, similar to `send_iter_until_full`, but specifically for slices.
    /// This method requires `T` to implement the `Copy` trait, ensuring that messages can be copied into the channel without taking ownership.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This allows the method to efficiently handle the messages without needing to manage ownership or deal with borrowing complexities.
    ///
    /// # Parameters
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Useful for efficiently sending multiple messages when partial delivery is sufficient.
    /// May not be appropriate for use cases requiring guaranteed delivery of all messages.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Efficiency**: Allows for the fast and straightforward copying of messages into the channel, leveraging the stack when possible.
    /// - **Simplicity**: Simplifies message handling by avoiding the complexities of ownership and borrowing rules inherent in more complex types.
    /// - **Predictability**: Ensures that messages remain unchanged after being sent, providing clear semantics for message passing.
    ///
    /// ## Cons:
    /// - **Flexibility**: Restricts the types of messages that can be sent through the channel to those that implement `Copy`, potentially limiting the use of the channel with more complex data structures.
    /// - **Resource Use**: For types where `Copy` might involve deep copying (though typically `Copy` is for inexpensive-to-copy types), it could lead to unintended resource use.
    /// - **Design Constraints**: Imposes a design constraint on the types used with the channel, which might not always align with the broader application architecture or data handling strategies.
    ///
    /// By requiring `T: Copy`, this method optimizes for scenarios where messages are lightweight and can be copied without significant overhead, making it an excellent choice for high-throughput, low-latency applications.
    pub(crate) fn send_slice_until_full(&mut self, slice: &[T]) -> usize
        where T: Copy {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_slice_until_full(slice)
    }
//  */

    /// Checks if the channel is currently full.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    ///
    /// # Example Usage
    /// Can be used to avoid calling `try_send` on a full channel, or to implement backpressure mechanisms.
    pub fn is_full(&self) -> bool {
        self.shared_is_full()
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Returns
    /// `true` if the channel is empty
    ///    
    pub fn is_empty(&self) -> bool {
        self.shared_is_empty()
    }

    /// Returns the number of vacant units in the channel.
    /// Indicates how many more messages the channel can accept before becoming full.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    ///
    /// # Example Usage
    /// Useful for dynamically adjusting the rate of message sends or for implementing custom backpressure strategies.
    pub fn vacant_units(&self) -> usize {
        self.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Example Usage
    /// Use this method to delay message sending until there's sufficient space, suitable for scenarios where message delivery must be paced or regulated.
    pub async fn wait_vacant_units(&mut self, count: usize) -> bool {
        self.shared_wait_shutdown_or_vacant_units(count).await
    }

    /// Asynchronously waits until the channel is empty.
    /// This method can be used to ensure that all messages have been processed before performing further actions.
    ///
    /// # Example Usage
    /// Ideal for scenarios where a clean state is required before proceeding, such as before shutting down a system or transitioning to a new state.
    pub async fn wait_empty(&mut self) -> bool {
        self.shared_wait_empty().await
    }

    ////////////////////////////////////////////////////////////////
    // Shared implementations, if you need to swap out the channel it is done here
    ////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    #[inline]
    pub(crate) fn shared_try_send(&mut self, msg: T) -> Result<(), T> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        match self.tx.try_push(msg) {
            Ok(_) => Ok(()),
            Err(m) => Err(m),
        }
    }

    #[inline]
    pub(crate) fn shared_send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        self.tx.push_iter(iter)
    }

    #[inline]
    pub(crate) fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
        where T: Copy {
        if self.make_closed.is_none() {

            let backtrace = Backtrace::capture();
            eprintln!("{:?}", backtrace);

            warn!("Send called after channel marked closed");
        }
        self.tx.push_slice(slice)
    }

    #[inline]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.tx.is_empty()
    }

    #[inline]
    pub(crate) fn shared_vacant_units(&self) -> usize {
        self.tx.vacant_len()
    }

    #[inline]
    pub(crate) async fn shared_wait_shutdown_or_vacant_units(&mut self, count: usize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let mut one_down = &mut self.oneshot_shutdown;
            if !one_down.is_terminated() {
                let mut operation = &mut self.tx.wait_vacant(count);
                select! { _ = one_down => false, _ = operation => true, }
            } else {
                false
            }
        }
    }
    #[inline]
    pub(crate) async fn shared_wait_vacant_units(&mut self, count: usize) -> bool {
        if self.tx.vacant_len() >= count {
            true
        } else {
            let operation = &mut self.tx.wait_vacant(count);
            operation.await;
            true
        }
    }
    

    #[inline]
    pub(crate) async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        if !one_down.is_terminated() {
            let mut operation = &mut self.tx.wait_vacant(usize::from(self.tx.capacity()));
            select! { _ = one_down => false, _ = operation => true, }
        } else {
            false
        }
    }

    #[inline]
    pub(crate) async fn shared_send_async(&mut self, msg: T, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), T> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }

        match self.tx.try_push(msg) {
            Ok(_) => Ok(()),
            Err(msg) => {
                match saturation {
                    SendSaturation::IgnoreAndWait => {}
                    SendSaturation::IgnoreAndErr => {
                        return Err(msg);
                    }
                    SendSaturation::Warn => {
                        self.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        #[cfg(debug_assertions)]
                        {
                            self.report_tx_full_warning(ident);
                        }
                    }
                }


                //NOTE: may block here on shutdown if graph is built poorly
                match self.tx.push(msg).await {
                    Ok(_) => Ok(()),
                    Err(t) => {
                        error!("channel is closed");
                        Err(t)
                    }
                }
            }
        }
    }

    fn report_tx_full_warning(&mut self, ident: ActorIdentity) {
        if self.last_error_send.elapsed().as_secs() > 10 {
            let type_name = type_name::<T>().split("::").last();
            warn!("{:?} tx full channel #{} {:?} cap:{:?} type:{:?} ",
                  ident, self.channel_meta_data.id, self.channel_meta_data.labels,
                  self.tx.capacity(), type_name);
            self.last_error_send = Instant::now();
        }
    }
}

/// A trait representing a definition for a transmission channel.
pub trait TxDef: Debug {
    /// Retrieves the metadata for the transmission channel.
    fn meta_data(&self) -> TxMetaData;
}

impl<T> TxDef for SteadyTx<T> {
    fn meta_data(&self) -> TxMetaData {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return TxMetaData(guard.deref().channel_meta_data.clone());
            }
            thread::yield_now();
            count += 1;

            //only print once we have tried for a while
            if 10000 == count {
                let backtrace = Backtrace::capture();
                eprintln!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}

/// A trait for handling bundles of transmission channels.
pub trait SteadyTxBundleTrait<T, const GIRTH: usize> {
    /// Locks all channels in the bundle, returning a future that resolves when all locks are acquired.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>>;

    /// Retrieves a slice of transmission channel definitions.
    fn def_slice(&self) -> [&dyn TxDef; GIRTH];

    /// Retrieves the metadata for all transmission channels in the bundle.
    fn meta_data(&self) -> [TxMetaData; GIRTH];

    /// Waits until a specified number of units are vacant in the channels.
    ///
    /// # Parameters
    /// - `avail_count`: The number of vacant units to wait for.
    /// - `ready_channels`: The number of channels that should have the vacant units.
    fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()> + Send;
}

impl<T: Sync + Send, const GIRTH: usize> SteadyTxBundleTrait<T, GIRTH> for SteadyTxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn def_slice(&self) -> [&dyn TxDef; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxDef)
            .collect::<Vec<&dyn TxDef>>()
            .try_into()
            .expect("Internal Error")
    }

    fn meta_data(&self) -> [TxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data())
            .collect::<Vec<TxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

    async fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) {
        let futures = self.iter().map(|tx| {
            let tx = tx.clone();
            async move {
                let mut tx = tx.lock().await;
                tx.wait_vacant_units(avail_count).await;
            }
                .boxed()
        });

        let mut futures: Vec<_> = futures.collect();
        let mut count_down = ready_channels.min(GIRTH);

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

/// A trait for handling transmission channel bundles.
pub trait TxBundleTrait {
    /// Marks all channels in the bundle as closed.
    fn mark_closed(&mut self) -> bool;

    /// Checks if any receiver instances have changed.
    fn rx_instance_changed(&mut self) -> bool;

    /// Resets the receiver instance tracking for all channels in the bundle.
    fn rx_instance_reset(&mut self);
}

impl<T> TxBundleTrait for TxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }

    fn rx_instance_changed(&mut self) -> bool {
        if self.iter_mut().any(|f| f.rx_instance_changed()) {
            self.iter_mut().for_each(|f| f.rx_instance_reset());
            true
        } else {
            false
        }
    }

    fn rx_instance_reset(&mut self) {
        self.iter_mut().for_each(|f| f.rx_instance_reset());
    }
}
