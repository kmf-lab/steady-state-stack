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
use crate::distributed::aqueduct_stream::{TxChannelMetaDataWrapper};
use crate::monitor::{ChannelMetaData};

/// Represents a transmission channel for sending messages of type `T`.
///
/// This struct encapsulates the core functionality for transmitting messages within a steady-state actor system. It manages an internal ring buffer for queuing messages, tracks channel metadata for logging and telemetry, and provides mechanisms for lifecycle management, such as closing the channel and detecting shutdown signals. It is designed to support both synchronous and asynchronous workflows, ensuring reliable and efficient message delivery.
pub struct Tx<T> {
    /// The internal sender that manages the ring buffer for message transmission.
    pub(crate) tx: InternalSender<T>,
    /// A wrapper around metadata for the channel, utilized for logging and telemetry purposes.
    pub(crate) channel_meta_data: TxChannelMetaDataWrapper,
    /// An index assigned for local monitoring, initialized upon first use of the channel.
    pub(crate) local_monitor_index: usize,
    /// The timestamp of the last error transmission, used to enforce rate-limiting on error logs.
    pub(crate) last_error_send: Instant,
    /// An optional sender for signaling that the channel has been closed, facilitating lifecycle control.
    pub(crate) make_closed: Option<oneshot::Sender<()>>,
    /// A receiver for detecting shutdown signals, enabling graceful termination of the channel.
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
}

impl<T> Debug for Tx<T> {
    /// Formats the `Tx` struct for debugging purposes.
    ///
    /// This implementation provides a basic representation of the struct. Future enhancements may include additional details for improved debugging visibility.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tx") // TODO: Enhance with more detailed information in the future.
    }
}

// Satisfy the trait for the raw struct
impl<T> TxMetaDataProvider for Tx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.channel_meta_data.meta_data)
    }
}

impl<T> Tx<T> {

    /// Retrieves metadata for this transmitter in a single-element array.
    ///
    /// This provides consistency with bundle interfaces, allowing a single
    /// transmitter to be treated as a collection of metadata providers.
    pub fn meta_data(&self) -> [&dyn TxMetaDataProvider; 1] {
        [self as &dyn TxMetaDataProvider]
    }

    /// Retrieves the unique identifier assigned to this transmission channel.
    ///
    /// Each channel is assigned a distinct ID, which is useful for tracking, debugging, or associating telemetry data with specific channels.
    ///
    /// # Returns
    /// A `usize` representing the channel’s unique identifier.
    pub fn id(&self) -> usize {
        self.channel_meta_data.meta_data.id
    }

    /// Signals that the channel is closed and will no longer accept new messages.
    ///
    /// This method marks the channel as closed in an idempotent manner, meaning multiple calls have the same effect as a single call. Once closed, new messages cannot be sent, though any queued messages may still be processed by the receiver.
    ///
    /// # Returns
    /// A `bool` indicating `true` if the channel was successfully marked as closed, `false` otherwise.
    pub fn mark_closed(&mut self) -> bool {
        self.shared_mark_closed()
    }

    /// Retrieves the total capacity of the channel’s message buffer.
    ///
    /// This method returns the maximum number of messages the channel can hold at any given time, aiding in buffer size configuration and performance optimization.
    ///
    /// # Returns
    /// A `usize` representing the total capacity of the channel.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Logs a warning when the channel reaches full capacity, with rate-limiting applied.
    ///
    /// This internal method issues a warning when the channel cannot accept additional messages, but only if sufficient time has passed since the last warning to avoid overwhelming the log. It includes details such as the actor’s identity and message type for context.
    ///
    /// # Parameters
    /// - `ident`: The identity of the actor associated with this channel, providing context for the warning.
    pub(crate) fn report_tx_full_warning(&mut self, ident: ActorIdentity) {
        if self.last_error_send.elapsed().as_secs() > steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
            let type_name = type_name::<T>().split("::").last();
            warn!(
                "{:?} tx full channel #{} {:?} cap:{:?} type:{:?} ",
                ident,
                self.channel_meta_data.meta_data.id,
                self.channel_meta_data.meta_data.labels,
                self.tx.capacity(),
                type_name
            );
            self.last_error_send = Instant::now();
        }
    }

    /// Determines whether the channel is currently at full capacity.
    ///
    /// This method checks if the channel has reached its maximum message limit, which is useful for implementing backpressure or deciding when to pause sending operations.
    ///
    /// # Returns
    /// A `bool` returning `true` if the channel is full, `false` otherwise.
    pub fn is_full(&self) -> bool {
        self.shared_is_full()
    }

    /// Determines whether the channel currently contains no messages.
    ///
    /// This method is helpful for monitoring the channel’s state or ensuring it is empty before performing specific actions, such as shutdown.
    ///
    /// # Returns
    /// A `bool` returning `true` if the channel is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.shared_is_empty()
    }

    /// Calculates the number of available slots remaining in the channel.
    ///
    /// This method provides the count of messages that can still be sent before the channel becomes full, supporting dynamic flow control or backpressure strategies.
    ///
    /// # Returns
    /// A `usize` representing the number of vacant units in the channel.
    pub fn vacant_units(&self) -> usize {
        self.shared_vacant_units()
    }

    /// Asynchronously waits until the channel has at least the specified number of vacant units.
    ///
    /// This method suspends execution until the channel has enough free space to accommodate the requested number of messages, making it suitable for regulating message flow in asynchronous contexts.
    ///
    /// # Parameters
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Returns
    /// A `bool` returning `true` if the required vacant units are available, `false` if the wait is interrupted by a shutdown signal.
    pub async fn wait_vacant_units(&mut self, count: usize) -> bool {
        self.shared_wait_shutdown_or_vacant_units(count).await
    }

    /// Asynchronously waits until the channel contains no messages.
    ///
    /// This method is ideal for scenarios requiring confirmation that all messages have been processed, such as prior to system shutdown or a state transition.
    ///
    /// # Returns
    /// A `bool` returning `true` if the channel becomes empty, `false` if the wait is interrupted by a shutdown signal.
    pub async fn wait_empty(&mut self) -> bool {
        self.shared_wait_empty().await
    }

    /// Sends a slice of messages to the channel until it reaches full capacity.
    ///
    /// This internal method attempts to transmit as many messages as possible from the provided slice, stopping when the channel can no longer accept additional messages. It requires the message type to implement the `Copy` trait.
    ///
    /// # Parameters
    /// - `slice`: A slice of messages to be sent through the channel.
    ///
    /// # Returns
    /// A `usize` indicating the number of messages successfully sent.
    pub(crate) fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
    where
        T: Copy,
    {
        debug_assert!(self.make_closed.is_some(), "Send called after channel marked closed");
        self.tx.push_slice(slice)
    }
}

/// Defines an interface for accessing metadata about a transmission channel.
///
/// This trait ensures that implementing types can provide metadata, such as channel ID and labels, for purposes like logging, debugging, or telemetry.
pub trait TxMetaDataProvider: Debug {
    /// Retrieves the metadata associated with the transmission channel.
    ///
    /// This method provides a shared reference to the channel’s metadata, ensuring thread-safe access to its details.
    ///
    /// # Returns
    /// An atomically reference-counted pointer (`Arc`) to the channel’s metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData>;
}

impl<T: Send + Sync> TxMetaDataProvider for Mutex<Tx<T>> {
    /// Retrieves the metadata for a transmission channel wrapped in a `Mutex`.
    ///
    /// This implementation attempts to acquire the mutex lock to access the channel’s metadata, retrying with a yield if the lock is unavailable. It logs an error if the lock cannot be obtained after numerous attempts, indicating a potential contention issue.
    ///
    /// # Returns
    /// An `Arc` pointing to the channel’s metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            thread::yield_now();
            count += 1;

            // Logs an error if the lock cannot be acquired after many attempts, aiding in diagnosing lock contention.
            if 10000 == count {
                let backtrace = Backtrace::capture();
                error!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}

impl<T: Send + Sync> TxMetaDataProvider for Arc<Mutex<Tx<T>>> {
    /// Retrieves the metadata for a transmission channel wrapped in an `Arc<Mutex>`.
    ///
    /// This implementation repeatedly attempts to acquire the mutex lock, pausing briefly between attempts. It logs a warning and error if the lock remains unavailable after an extended period, helping to identify prolonged contention.
    ///
    /// # Returns
    /// An `Arc` pointing to the channel’s metadata.
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        let mut count = 0;
        loop {
            if let Some(guard) = self.try_lock() {
                return Arc::clone(&guard.deref().channel_meta_data.meta_data);
            }
            sleep(Duration::from_millis(5));
            count += 1;

            // Logs a warning and error if the lock cannot be acquired after many attempts, indicating potential issues.
            if 100_000 == count {
                let backtrace = Backtrace::capture();
                warn!("{:?}", backtrace);
                error!("got stuck on meta_data, unable to get lock on ChannelMetaData");
            }
        }
    }
}

/// Defines an interface for managing a fixed-size bundle of transmission channels.
///
/// This trait provides methods for synchronized operations across multiple channels, such as locking or waiting for vacant units, which is valuable in systems requiring coordinated channel management.
pub trait SteadyTxBundleTrait<T, const GIRTH: usize> {
    /// Acquires locks on all channels in the bundle.
    ///
    /// This method returns a future that resolves when all channels in the bundle are locked, enabling synchronized access across the entire set.
    ///
    /// # Returns
    /// A `JoinAll` future resolving to a collection of mutex guards for the channels.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>>;

    /// Retrieves metadata for all transmission channels in the bundle.
    ///
    /// This method gathers metadata from each channel, providing a comprehensive overview of the bundle’s state.
    ///
    /// # Returns
    /// An array of references to metadata providers, one for each channel in the bundle.
    fn meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];

    /// Asynchronously waits until a specified number of channels have a certain number of vacant units.
    ///
    /// This method coordinates waiting across the bundle, ensuring that the required number of channels have sufficient free space before proceeding, which is useful for multi-channel flow control.
    ///
    /// # Parameters
    /// - `avail_count`: The number of vacant units to wait for in each channel.
    /// - `ready_channels`: The number of channels that must meet the vacancy condition.
    ///
    /// # Returns
    /// A future that resolves when the specified conditions are satisfied.
    fn wait_vacant_units(&self, avail_count: usize, ready_channels: usize) -> impl std::future::Future<Output = ()>;
}

impl<T: Sync + Send, const GIRTH: usize> SteadyTxBundleTrait<T, GIRTH> for SteadyTxBundle<T, GIRTH> {
    /// Locks all channels in the bundle concurrently.
    ///
    /// This implementation uses a join operation to acquire locks on all channels simultaneously, ensuring synchronized access.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    /// Collects metadata from all channels in the bundle.
    ///
    /// This implementation iterates over the bundle, converting each channel into a metadata provider reference and returning them as a fixed-size array.
    fn meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    /// Waits asynchronously until the specified number of channels have the required vacant units.
    ///
    /// This implementation creates a set of futures to monitor each channel, resolving when enough channels meet the vacancy condition or all channels have been checked.
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

/// Defines an interface for managing a collection of transmission channels.
///
/// This trait focuses on lifecycle operations across a group of channels, such as marking them all as closed.
pub trait TxBundleTrait {
    /// Marks all channels in the bundle as closed.
    ///
    /// This method signals that no further messages will be sent through any channel in the bundle. It operates in an all-or-nothing manner and is idempotent, ensuring consistent behavior across multiple calls.
    ///
    /// # Returns
    /// A `bool` returning `true` to indicate the request was processed successfully.
    fn mark_closed(&mut self) -> bool;
    
/// Returns true if all channels have been fully consumed
    fn is_all_empty(&mut self) -> bool;
}

impl<T> TxBundleTrait for TxBundle<'_, T> {
    /// Closes all channels in the bundle.
    ///
    /// This implementation iterates over the bundle, marking each channel as closed. It always completes the operation fully and returns `true` to confirm the request was handled.
    fn mark_closed(&mut self) -> bool {
        // Ensures all channels are closed, never stopping early.
        self.iter_mut().for_each(|f| { let _ = f.mark_closed(); });
        true // Always returns true, as the close request is never rejected by this method.
    }

    fn is_all_empty(&mut self) -> bool {
        // Ensures all channels are empty and fully consumed
        self.iter().all(|f| f.is_empty())
    }
}

/// Represents the result of a transmission operation in a steady-state system.
///
/// This enum distinguishes between standard and stream-based transmission outcomes, tracking the number of items sent and, for streams, the number of bytes transmitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxDone {
    /// Represents a standard transmission with the count of items sent.
    Normal(usize),
    /// Represents a stream transmission with counts of items and bytes sent.
    Stream(usize, usize),
}

impl TxDone {
    /// Retrieves the number of items sent during the transmission.
    ///
    /// This method provides a unified way to access the item count, applicable to both normal and stream variants.
    ///
    /// # Returns
    /// A `usize` representing the number of items sent.
    pub fn item_count(&self) -> usize {
        match *self {
            TxDone::Normal(count) => count,
            TxDone::Stream(first, _) => first,
        }
    }

    /// Retrieves the number of payload bytes sent, if applicable.
    ///
    /// This method returns the byte count for stream transmissions, or `None` for standard transmissions where byte count is not tracked.
    ///
    /// # Returns
    /// An `Option<usize>` containing the byte count for stream operations, or `None` for normal operations.
    pub fn payload_count(&self) -> Option<usize> {
        match *self {
            TxDone::Normal(_) => None,
            TxDone::Stream(_, second) => Some(second),
        }
    }
}

impl Deref for TxDone {
    type Target = usize;

    /// Allows dereferencing to the item count of the transmission.
    ///
    /// This implementation enables `TxDone` to be used as a `usize` representing the item count in contexts where such dereferencing is supported.
    fn deref(&self) -> &usize {
        match self {
            TxDone::Normal(count) => count,
            TxDone::Stream(first, _) => first,
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

        // Locks and inspects the transmitter’s capacity.
        let tx = tx_lazy.clone();
        let ste_tx = core_exec::block_on(tx.lock());
        assert_eq!(ste_tx.shared_capacity(), 2);
        drop(ste_tx);

        // Locks and peeks at the receiver’s next message.
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
}