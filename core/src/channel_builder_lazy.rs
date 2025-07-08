use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use futures_util::lock::Mutex;
use ringbuf::producer::Producer;
use crate::abstract_executor_async_std::core_exec;
use crate::{SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle};
use crate::channel_builder::ChannelBuilder;

/**
 * Lazy wrapper for `SteadyTx<T>`, deferring channel allocation until first use.
 *
 * Ensures resources are allocated near the point of use, improving efficiency in actor systems.
 */
#[derive(Debug)]
pub struct LazySteadyTx<T> {
    lazy_channel: Arc<LazyChannel<T>>,
}

impl <T> LazySteadyTx<T> {
    /**
     * Creates a new `LazySteadyTx` instance.
     *
     * # Arguments
     *
     * - `lazy_channel`: Shared reference to the lazy channel configuration.
     *
     * # Returns
     *
     * A new `LazySteadyTx<T>` instance.
     */
    pub(crate) fn new(lazy_channel: Arc<LazyChannel<T>>) -> Self {
        LazySteadyTx {
            lazy_channel,
        }
    }

    /**
     * Clones the transmitter, initializing the channel on first call.
     *
     * Returns a cached `SteadyTx<T>` instance for subsequent calls, ensuring allocation near the actor.
     *
     * # Returns
     *
     * A `SteadyTx<T>` instance for sending data.
     */
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyTx<T> {
        core_exec::block_on(self.lazy_channel.get_tx_clone())
    }

    /**
     * Sends all items in a vector to the channel for testing purposes.
     *
     * Blocks until all items are sent, retrying if the channel is full; optionally closes the channel.
     *
     * # Arguments
     *
     * - `data`: Vector of items to send.
     * - `close`: Whether to close the channel after sending.
     *
     * # Panics
     *
     * Panics with "internal error" if the transmitter lock cannot be acquired.
     */
    pub fn testing_send_all(&self, data: Vec<T>, close: bool) {
        let tx = self.clone();
        let mut tx = tx.try_lock().expect("internal error");

        for d in data.into_iter() {
            let mut temp = d;
            while let Err(r) = tx.tx.try_push(temp) {
                temp = r;
                sleep(Duration::from_millis(2));
            };
        }
        if close {
            tx.mark_closed(); // for clean shutdown we tell the actor we have no more data
        }
    }

    /**
     * Closes the channel for testing clean shutdowns.
     *
     * Marks the channel as closed, indicating no further data will be sent.
     *
     * # Panics
     *
     * Panics with "internal error" if the transmitter lock cannot be acquired.
     */
    pub fn testing_close(&self) {
        let tx = self.clone();
        let mut tx = tx.try_lock().expect("internal error");
        tx.mark_closed();
    }
}

/**
 * Lazy wrapper for `SteadyRx<T>`, deferring channel allocation until first use.
 *
 * Similar to `LazySteadyTx`, it ensures resource allocation occurs near the point of use.
 */
#[derive(Debug)]
pub struct LazySteadyRx<T> {
    lazy_channel: Arc<LazyChannel<T>>,
}

impl <T> LazySteadyRx<T> {
    /**
     * Creates a new `LazySteadyRx` instance.
     *
     * # Arguments
     *
     * - `lazy_channel`: Shared reference to the lazy channel configuration.
     *
     * # Returns
     *
     * A new `LazySteadyRx<T>` instance.
     */
    pub(crate) fn new(lazy_channel: Arc<LazyChannel<T>>) -> Self {
        LazySteadyRx {
            lazy_channel,
        }
    }

    /**
     * Clones the receiver, initializing the channel on first call.
     *
     * Returns a cached `SteadyRx<T>` instance for subsequent calls.
     *
     * # Returns
     *
     * A `SteadyRx<T>` instance for receiving data.
     */
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyRx<T> {
        core_exec::block_on(self.lazy_channel.get_rx_clone())
    }

    /**
     * Takes all available items from the channel for testing.
     *
     * Blocks until all items are retrieved, returning them in a vector.
     *
     * # Returns
     *
     * A vector containing all available items from the channel.
     */
    pub fn testing_take_all(&self) -> Vec<T> {
        core_exec::block_on(async {
            let rx = self.lazy_channel.get_rx_clone().await;
            let mut rx = rx.lock().await;
            let mut result = Vec::new();

            while let Some(item) = rx.try_take() {
                result.push(item);
            }

            result
        })
    }
}

/**
 * Structure managing lazy initialization of a channel.
 *
 * Holds the builder configuration and channel instance, initializing the channel on demand.
 */
#[derive(Debug)]
pub(crate) struct LazyChannel<T> {
    builder: Mutex<Option<ChannelBuilder>>,
    channel: Mutex<Option<(SteadyTx<T>, SteadyRx<T>)>>,
}

impl <T> LazyChannel<T> {
    /**
     * Creates a new `LazyChannel` instance.
     *
     * # Arguments
     *
     * - `builder`: Reference to the `ChannelBuilder` to configure the channel.
     *
     * # Returns
     *
     * A new `LazyChannel<T>` instance.
     */
    pub(crate) fn new(builder: &ChannelBuilder) -> Self {
        LazyChannel {
            builder: Mutex::new(Some(builder.clone())),
            channel: Mutex::new(None),
        }
    }

    /**
     * Retrieves or initializes the transmitter, returning a clone.
     *
     * # Returns
     *
     * A `SteadyTx<T>` instance, initializing the channel if necessary.
     *
     * # Panics
     *
     * Panics with "internal error" if the builder is taken more than once or channel access fails.
     */
    pub(crate) async fn get_tx_clone(&self) -> SteadyTx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let b = self.builder.lock().await.take()
                .expect("internal error, only done once");
            *channel = Some(b.eager_build());
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    /**
     * Retrieves or initializes the receiver, returning a clone.
     *
     * # Returns
     *
     * A `SteadyRx<T>` instance, initializing the channel if necessary.
     *
     * # Panics
     *
     * Panics with "internal error" if the builder is taken more than once or channel access fails.
     */
    pub(crate) async fn get_rx_clone(&self) -> SteadyRx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let b = self.builder.lock().await.take()
                .expect("internal error, only done once");
            *channel = Some(b.eager_build());
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}

/// Type alias for an array of lazy-initialized transmitters with a fixed size.
///
/// Simplifies the usage of a bundle of transmitters that are initialized on first use.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
/// - `GIRTH`: The fixed size of the transmitter array.
pub type LazySteadyTxBundle<T, const GIRTH: usize> = [LazySteadyTx<T>; GIRTH];

/// Trait for cloning lazy transmitter bundles with initialization.
///
/// Defines the behavior for cloning a bundle of lazy transmitters, triggering initialization.
pub trait LazySteadyTxBundleClone<T, const GIRTH: usize> {
    /// Clones the bundle of transmitters, lazily initializing the channels.
    ///
    /// # Returns
    /// - `SteadyTxBundle<T, GIRTH>`: A fully initialized bundle of transmitters.
    fn clone(&self) -> SteadyTxBundle<T, GIRTH>;
}

impl<T, const GIRTH: usize> LazySteadyTxBundleClone<T, GIRTH> for LazySteadyTxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyTxBundle<T, GIRTH> {
        let tx_clones: Vec<SteadyTx<T>> = self.iter().map(|l| l.clone()).collect();
        match tx_clones.try_into() {
            Ok(array) => crate::steady_tx_bundle(array),
            Err(_) => panic!("Internal error, bad length"),
        }
    }
}

/// Type alias for an array of lazy-initialized receivers with a fixed size.
///
/// Simplifies the usage of a bundle of receivers that are initialized on first use.
///
/// # Type Parameters
/// - `T`: The type of data being received.
/// - `GIRTH`: The fixed size of the receiver array.
pub type LazySteadyRxBundle<T, const GIRTH: usize> = [LazySteadyRx<T>; GIRTH];

/// Trait for cloning lazy receiver bundles with initialization.
///
/// Defines the behavior for cloning a bundle of lazy receivers, triggering initialization.
pub trait LazySteadyRxBundleClone<T, const GIRTH: usize> {
    /// Clones the bundle of receivers, lazily initializing the channels.
    ///
    /// # Returns
    /// - `SteadyRxBundle<T, GIRTH>`: A fully initialized bundle of receivers.
    fn clone(&self) -> SteadyRxBundle<T, GIRTH>;
}

impl<T, const GIRTH: usize> LazySteadyRxBundleClone<T, GIRTH> for LazySteadyRxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyRxBundle<T, GIRTH> {
        let rx_clones: Vec<SteadyRx<T>> = self.iter().map(|l| l.clone()).collect();
        match rx_clones.try_into() {
            Ok(array) => crate::steady_rx_bundle(array),
            Err(_) => panic!("Internal error, bad length"),
        }
    }
}