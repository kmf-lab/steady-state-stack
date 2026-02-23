use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use futures_util::lock::Mutex;
use log::warn;
use ringbuf::producer::Producer;
use crate::{SteadyRx, SteadyRxBundle, SteadyTx, SteadyTxBundle};
use crate::channel_builder::ChannelBuilder;
use crate::core_exec;

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
        use crate::core_tx::TxCore;

        let tx = self.clone();
        let mut tx = tx.try_lock().expect("internal error");

        if data.len() >= tx.shared_vacant_units() {
            let existing = tx.capacity() - tx.shared_vacant_units();
            warn!("test data is larger than target chanel, this requires the actor/graph consumer to be started before we call testing_send_all OR you must lengthen the target channel to at least {}"
                 , data.len()+existing);
        }

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
            Ok(array) => Arc::new(array),
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
            Ok(array) => Arc::new(array),
            Err(_) => panic!("Internal error, bad length"),
        }
    }
}

// Smoke tests for LazySteadyTx and LazySteadyRx to verify basic functionality.
#[cfg(test)]
mod steady_lazy_tests {
    use super::*;
    use crate::channel_builder::ChannelBuilder;
    use crate::*;

    /// Tests the basic flow of sending and receiving messages through lazy transmitter and receiver channels.
    #[test]
    fn test_lazy_flow() {
        let builder = ChannelBuilder::default().with_capacity(2);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();

        // Clones the transmitter lazily and sends messages.
        tx_lazy.testing_send_all(vec![1, 2], false);

        // Locks and inspects the transmitter's capacity.
        let tx = tx_lazy.clone();
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 2);
        drop(ste_tx);

        // Locks and peeks at the receiver's next message.
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        assert_eq!(ste_rx.try_peek(), Some(&1));
        drop(ste_rx);

        // Locks and retrieves messages from the receiver.
        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        assert_eq!(ste_rx.try_take(), Some(1));
        assert_eq!(ste_rx.try_take(), Some(2));
        assert_eq!(ste_rx.try_take(), None);
    }

    /// Tests lazy channel initialization - verifies that channels are properly created on first clone.
    #[test]
    fn test_lazy_channel_initialization() {
        let builder = ChannelBuilder::default().with_capacity(10);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();
        
        // First clone triggers initialization on tx side
        let tx = tx_lazy.clone();
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 10);
        drop(ste_tx);
        
        // First clone on rx triggers initialization  
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        // Should be empty but initialized
        assert!(ste_rx.shared_is_empty());
        // Verify capacity is correct
        assert_eq!(ste_rx.shared_capacity(), 10);
        drop(ste_rx);
    }

    /// Tests testing_send_all with close=true to verify channel closure.
    #[test]
    fn test_testing_send_all_with_close() {
        let builder = ChannelBuilder::default().with_capacity(5);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();
        
        // Send data with close = true
        tx_lazy.testing_send_all(vec![1, 2, 3], true);
        
        // Verify transmitter is properly configured
        let tx = tx_lazy.clone();
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 5);
        
        // Verify we can still receive the data (channel not fully closed at this level)
        drop(ste_tx);
        
        let rx = rx_lazy.clone();
        let mut ste_rx = core_exec::block_on(rx.lock());
        assert_eq!(ste_rx.try_take(), Some(1));
        assert_eq!(ste_rx.try_take(), Some(2));
        assert_eq!(ste_rx.try_take(), Some(3));
    }

    /// Tests LazySteadyRx::testing_take_all() to verify it retrieves all available items.
    #[test]
    fn test_testing_take_all() {
        let builder = ChannelBuilder::default().with_capacity(5);
        let (tx_lazy, rx_lazy) = builder.build_channel::<u8>();
        
        // Send some data via the lazy tx
        tx_lazy.testing_send_all(vec![10, 20, 30], false);
        
        // Take all via lazy receiver's testing method
        let items = rx_lazy.testing_take_all();
        assert_eq!(items, vec![10, 20, 30]);
        
        // Verify channel is now empty
        let rx = rx_lazy.clone();
        let ste_rx = core_exec::block_on(rx.lock());
        assert!(ste_rx.shared_is_empty());
    }

    /// Tests LazySteadyTxBundleClone trait implementation.
    /// This test verifies that cloning a lazy bundle triggers lazy initialization 
    /// and returns a SteadyTxBundle that can be used with the lock() method.
    #[test]
    fn test_lazy_tx_bundle_clone() {
        use crate::channel_builder_lazy::LazySteadyTxBundleClone;
        
        let builder = ChannelBuilder::default().with_capacity(3);
        let (tx_bundle, _rx_bundle) = builder.build_channel_bundle::<u8, 4>();
        
        // Clone via trait - this triggers lazy initialization on all lanes
        // The clone() method on LazySteadyTxBundle returns a SteadyTxBundle
        let cloned: SteadyTxBundle<u8, 4> = tx_bundle.clone();
        
        // Use cloned bundle - all lanes should be initialized
        // SteadyTxBundle has the lock() method from SteadyTxBundleTrait
        let mut guards = core_exec::block_on(cloned.lock());
        for tx in guards.iter_mut() {
            let result = tx.shared_try_send(42);
            assert!(result.is_ok());
        }
    }

    /// Tests LazySteadyRxBundleClone trait implementation.
    /// This test verifies that cloning a lazy receiver bundle triggers lazy initialization
    /// and returns a SteadyRxBundle that can be used with the lock() method.
    #[test]
    fn test_lazy_rx_bundle_clone() {
        use crate::channel_builder_lazy::LazySteadyRxBundleClone;
        
        let builder = ChannelBuilder::default().with_capacity(3);
        let (tx_bundle, rx_bundle) = builder.build_channel_bundle::<u8, 4>();
        
        // First send some data on each lane
        {
            let cloned_tx: SteadyTxBundle<u8, 4> = tx_bundle.clone();
            let mut guards = core_exec::block_on(cloned_tx.lock());
            for (i, tx) in guards.iter_mut().enumerate() {
                let _ = tx.shared_try_send(i as u8);
            }
        }
        
        // Clone receiver bundle via trait - triggers lazy initialization
        let cloned_rx: SteadyRxBundle<u8, 4> = rx_bundle.clone();
        
        // Verify we can receive on all lanes
        let mut guards = core_exec::block_on(cloned_rx.lock());
        for (i, rx) in guards.iter_mut().enumerate() {
            let val = rx.shared_try_take();
            assert_eq!(val.map(|(_, v)| v), Some(i as u8));
        }
    }
}
