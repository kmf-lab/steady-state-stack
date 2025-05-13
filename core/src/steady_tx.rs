use std::sync::Arc;
use log::{error, warn};
use futures_util::{FutureExt};
use std::any::type_name;
use std::backtrace::Backtrace;
use std::time::{Duration, Instant};
use futures::channel::oneshot;
use futures_util::lock::{Mutex, MutexLockFuture};
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use futures_util::future::{select_all};
use std::fmt::Debug;
use std::ops::Deref;
use std::thread;
use std::thread::sleep;
use crate::{steady_config, ActorIdentity, SteadyTxBundle, TxBundle};
use crate::channel_builder::InternalSender;
use crate::core_tx::TxCore;
use crate::distributed::distributed_stream::{TxChannelMetaDataWrapper};
use crate::monitor::{ChannelMetaData};

/// The `Tx` struct represents a transmission channel for messages of type `T`.
/// It provides methods to send messages to the channel, check the channel's state, and handle the transmission lifecycle.
pub struct Tx<T> {
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: TxChannelMetaDataWrapper,
    pub(crate) local_index: usize, //set on first usage
    pub(crate) last_error_send: Instant,
    pub(crate) make_closed: Option<oneshot::Sender<()>>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
}

impl<T> Debug for Tx<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tx") //TODO: add more details
    }
}


impl<T> Tx<T> {

    /// Returns the unique identifier of the transmission channel.
    ///
    /// # Returns
    /// A `usize` representing the channel's unique ID.
    pub fn id(&self) -> usize {
        self.channel_meta_data.meta_data.id
    }

    /// Marks the channel as closed, indicating no more messages are expected.
    ///
    /// # Returns
    /// `true` if the channel was successfully marked as closed, otherwise `false`.
    pub fn mark_closed(&mut self) -> bool {
        self.shared_mark_closed()
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


    pub(crate) fn report_tx_full_warning(&mut self, ident: ActorIdentity) {
        if self.last_error_send.elapsed().as_secs() > steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            let type_name = type_name::<T>().split("::").last();
            warn!("{:?} tx full channel #{} {:?} cap:{:?} type:{:?} ",
                  ident, self.channel_meta_data.meta_data.id, self.channel_meta_data.meta_data.labels,
                  self.tx.capacity(), type_name);
            self.last_error_send = Instant::now();
        }
    }

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

    // TODO: move this once we figure copy.
    pub(crate) fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
     where T: Copy
    {
        debug_assert!(self.make_closed.is_some(),"Send called after channel marked closed");
        self.tx.push_slice(slice)
    }


}

/// A trait representing a definition for a transmission channel.
pub trait TxMetaDataProvider: Debug {
    /// Retrieves the metadata for the transmission channel.
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl<T: Send + Sync> TxMetaDataProvider for Mutex<Tx<T>> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            thread::yield_now();
            count += 1;

            //only print once we have tried for a while
            if 10000 == count {
                let backtrace = Backtrace::capture();
                error!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}
impl<T: Send + Sync> TxMetaDataProvider for Arc<Mutex<Tx<T>>> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            sleep(Duration::from_millis(5));
            count += 1;

            //only print once we have tried for a while
            if 100_000 == count {
                let backtrace = Backtrace::capture();
                warn!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}

/// A trait for handling bundles of transmission channels.
pub trait SteadyTxBundleTrait<T, const GIRTH: usize> {
    /// Locks all channels in the bundle, returning a future that resolves when all locks are acquired.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>>;

    /// Retrieves the metadata for all transmission channels in the bundle.
    fn meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];

    /// Waits until a specified number of units are vacant in the channels.
    ///
    /// # Parameters
    /// - `avail_count`: The number of vacant units to wait for.
    /// - `ready_channels`: The number of channels that should have the vacant units.
    fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()>;
}

impl<T: Sync + Send, const GIRTH: usize> SteadyTxBundleTrait<T, GIRTH> for SteadyTxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
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


}

impl<T> TxBundleTrait for TxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }

}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum TxDone {
    Normal(usize),
    Stream(usize,usize)
}

// LazySteadyTx / LazySteadyRx smoke tests
#[cfg(test)]
mod steady_lazy_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;

    #[test]
    fn test_lazy_flow() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();

        // lazy-clone and send
        tx_lazy.testing_send_all(vec![1, 2], false);
        // lock & inspect
        let tx = tx_lazy.clone();
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 2);
        drop(ste_tx);

        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        assert_eq!(ste_rx.try_peek(), Some(&1));
        drop(ste_rx);

        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        assert_eq!(ste_rx.try_take(), Some(1));
        assert_eq!(ste_rx.try_take(), Some(2));
        assert_eq!(ste_rx.try_take(), None);
    }
}




