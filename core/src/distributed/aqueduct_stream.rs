//! Steady stream module for managing lazy-initialized Tx and Rx channels.
//! We use one stream per channel. We can have N streams as a const array
//! going into `aeron_publish` and N streams as a const array coming from
//! `aeron_subscribe`.

use crate::core_tx::TxCore;
use crate::{channel_builder::ChannelBuilder, Rx, SteadyActor, Tx};
use ahash::AHashMap;
use async_ringbuf::wrap::AsyncWrap;
use async_ringbuf::AsyncRb;
use futures_util::lock::{Mutex, MutexGuard, MutexLockFuture};
use ringbuf::consumer::Consumer;
use ringbuf::producer::Producer;
use ringbuf::storage::Heap;
use ringbuf::traits::{Observer, Split};
use std::collections::VecDeque;
use std::fmt::{Debug, Formatter};
use std::num::NonZero;
use std::ops::Mul;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;
use futures_util::select;
use crate::core_rx::RxCore;
use crate::monitor::ChannelMetaData;
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;
use crate::core_exec;
use futures::future::FutureExt; // For .fuse()
use futures::pin_mut;
use log::{error, trace};
// For pin_mut!

/// Type alias for the identifier used in Aeron, typically a 32-bit integer for stream or session IDs.
pub type IdType = i32;

/// Type alias for an array of fixed size (GIRTH) containing thread-safe transmitters (Tx) for lazy-initialized streams.
pub type LazySteadyStreamTxBundle<T, const GIRTH: usize> = [LazyStreamTx<T>; GIRTH];

/// Type alias for an array of fixed size (GIRTH) containing thread-safe receivers (Rx) for lazy-initialized streams.
pub type LazySteadyStreamRxBundle<T, const GIRTH: usize> = [LazyStreamRx<T>; GIRTH];

/// Trait for cloning a bundle of lazy-initialized transmitter streams, triggering channel initialization if needed.
pub trait LazySteadyStreamTxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Creates a new bundle of thread-safe transmitters by cloning the lazy-initialized channels and initializing them if not already done.
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH>;
}

/// Trait for cloning a bundle of lazy-initialized receiver streams, triggering channel initialization if needed.
pub trait LazySteadyStreamRxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Creates a new bundle of thread-safe receivers by cloning the lazy-initialized channels and initializing them if not already done.
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH>;
}

/// Implementation of cloning for a bundle of lazy-initialized transmitter streams.
impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamTxBundleClone<T, GIRTH> for LazySteadyStreamTxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH> {
        let tx_clones: Vec<SteadyStreamTx<T>> = self.iter().map(|l| l.clone()).collect();
        match tx_clones.try_into() {
            Ok(array) => Arc::new(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }
}

/// Implementation of cloning for a bundle of lazy-initialized receiver streams.
impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamRxBundleClone<T, GIRTH> for LazySteadyStreamRxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH> {
        let rx_clones: Vec<SteadyStreamRx<T>> = self.iter().map(|l| l.clone()).collect();
        match rx_clones.try_into() {
            Ok(array) => Arc::new(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }
}

/// Type alias for a thread-safe, fixed-size array of receiver streams wrapped in an Arc.
pub type SteadyStreamRxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamRx<T>; GIRTH]>;

/// Type alias for a thread-safe, fixed-size array of transmitter streams wrapped in an Arc.
pub type SteadyStreamTxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamTx<T>; GIRTH]>;

/// Trait providing methods for interacting with a bundle of receiver streams.
pub trait SteadyStreamRxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
    /// Acquires locks for all receivers in the bundle, returning a future that resolves when all locks are obtained.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>>;

    /// Retrieves metadata for the control channels of all receivers in the bundle.
    fn control_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];

    /// Retrieves metadata for the payload channels of all receivers in the bundle.
    fn payload_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];
}

/// Implementation of receiver bundle operations for a thread-safe array of receiver streams.
impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| steady_stream as &dyn RxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| {
                {
                    steady_stream.try_lock().expect("Internal error").spotlight_control = false;
                }
                steady_stream as &dyn RxMetaDataProvider
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }
}

/// Trait providing methods for interacting with a bundle of transmitter streams.
pub trait SteadyStreamTxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
    /// Acquires locks for all transmitters in the bundle, returning a future that resolves when all locks are obtained.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>>;

    /// Retrieves metadata for the control channels of all transmitters in the bundle.
    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];

    /// Retrieves metadata for the payload channels of all transmitters in the bundle.
    fn payload_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];
}

/// Implementation of transmitter bundle operations for a thread-safe array of transmitter streams.
impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| steady_stream as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| {
                {
                    steady_stream.try_lock().expect("Internal error").spotlight_control = false;
                }
                steady_stream as &dyn TxMetaDataProvider
            })
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }
}

//////////////////////////////////////////////
/////   run loop functions
/////////////////////////////////

/// Type alias for a vector of locked transmitter stream guards.
pub type StreamTxBundle<'a, T> = Vec<MutexGuard<'a, StreamTx<T>>>;

/// Type alias for a vector of locked receiver stream guards.
pub type StreamRxBundle<'a, T> = Vec<MutexGuard<'a, StreamRx<T>>>;

/// Trait for managing a bundle of transmitter channels during runtime.
pub trait StreamTxBundleTrait {
    /// Marks all channels in the bundle as closed, signaling that no further data will be sent.
    fn mark_closed(&mut self) -> bool;
}

/// Trait for inspecting the state of a bundle of receiver channels during runtime.
pub trait StreamRxBundleTrait {
    /// Checks if all channels in the bundle are closed and have no remaining data.
    fn is_closed_and_empty(&mut self) -> bool;

    /// Checks if all channels in the bundle are closed.
    fn is_closed(&mut self) -> bool;

    /// Checks if all channels in the bundle have no remaining data.
    fn is_empty(&mut self) -> bool;
}

/// Implementation of transmitter bundle operations for a vector of locked transmitter streams.
impl<T: StreamControlItem> StreamTxBundleTrait for StreamTxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        if self.is_empty() {
            trace!("bundle has no streams, nothing found to be closed");
            return true; // true we did close nothing
        }
        // NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {
            let _ = f.mark_closed();
        });
        true // always returns true, close request is never rejected by this method.
    }
}

/// Implementation of receiver bundle operations for a vector of locked receiver streams.
impl<T: StreamControlItem> StreamRxBundleTrait for StreamRxBundle<'_, T> {
    fn is_closed_and_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed_and_empty())
    }

    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }

    fn is_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_empty())
    }
}

//////////////////////////////

/// Trait for items that can be transmitted or received over a stream, providing metadata and construction methods.
pub trait StreamControlItem: Copy + Send + Sync + 'static {
    /// Creates a new instance for testing purposes with the specified length.
    fn testing_new(length: i32) -> Self;

    /// Returns the length of the item in bytes.
    fn length(&self) -> i32;

    /// Constructs a new stream item from a defragmentation entry.
    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self;
}

/// Represents an incoming stream fragment, typically part of a multi-part message, with metadata for session and timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamIngress {
    /// Length of the fragment in bytes.
    pub length: i32,
    /// Session identifier for the stream.
    pub session_id: IdType,
    /// Time when the fragment was received.
    pub arrival: Instant,
    /// Time when the fragment was fully processed.
    pub finished: Instant,
}

/// Implementation of default values for incoming stream fragments.
impl Default for StreamIngress {
    fn default() -> Self {
        let now = Instant::now();
        StreamIngress {
            length: 0,
            session_id: 0,
            arrival: now,
            finished: now,
        }
    }
}

/// Methods for creating and manipulating incoming stream fragments.
impl StreamIngress {
    /// Creates a new incoming stream fragment with the specified parameters.
    ///
    /// Panics if the length is negative.
    pub fn new(length: i32, session_id: i32, arrival: Instant, finished: Instant) -> Self {
        assert!(length >= 0, "Fragment length cannot be negative");
        StreamIngress {
            length,
            session_id,
            arrival,
            finished,
        }
    }

    /// Creates a new fragment and returns it with an owned byte buffer.
    pub fn by_box(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, Box<[u8]>) {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0.into())
    }

    /// Creates a new fragment and returns it with a reference to the input byte slice.
    pub fn by_ref(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8]) {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0)
    }

    /// Alias for `by_ref`, creating a new fragment with a reference to the input byte slice.
    pub fn build(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8]) {
        StreamIngress::by_ref(session_id, arrival, finished, p0)
    }
}

/// Implementation of stream control item functionality for incoming fragments.
impl StreamControlItem for StreamIngress {
    fn testing_new(length: i32) -> Self {
        StreamIngress {
            length,
            session_id: 0,
            arrival: Instant::now(),
            finished: Instant::now(),
        }
    }

    fn length(&self) -> i32 {
        self.length
    }

    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self {
        StreamIngress {
            length: defrag_entry.running_length as i32,
            session_id: defrag_entry.session_id,
            arrival: defrag_entry.arrival.expect("defrag must have needed Instant"),
            finished: defrag_entry.finish.expect("defrag must have needed Instant"),
        }
    }
}

/// Represents an outgoing stream message, typically a single-part message with length metadata.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamEgress {
    /// Length of the message in bytes.
    pub(crate) length: i32,
}

/// Methods for creating and manipulating outgoing stream messages.
impl StreamEgress {
    /// Creates a new outgoing stream message and returns it with an owned byte buffer.
    pub fn build(p0: &[u8]) -> (StreamEgress, Box<[u8]>) {
        StreamEgress::by_box(p0)
    }

    /// Creates a new outgoing stream message and returns it with an owned byte buffer.
    pub fn by_box(p0: &[u8]) -> (StreamEgress, Box<[u8]>) {
        (StreamEgress::new(p0.len() as i32), p0.into())
    }

    /// Creates a new outgoing stream message and returns it with a reference to the input byte slice.
    pub fn by_ref(p0: &[u8]) -> (StreamEgress, &[u8]) {
        (StreamEgress::new(p0.len() as i32), p0)
    }

    /// Creates a new outgoing stream message with the specified length.
    ///
    /// Panics if the length is negative.
    pub fn new(length: i32) -> Self {
        assert!(length >= 0, "Message length cannot be negative");
        StreamEgress { length }
    }
}

/// Implementation of stream control item functionality for outgoing messages.
impl StreamControlItem for StreamEgress {
    fn testing_new(length: i32) -> Self {
        StreamEgress { length }
    }

    fn length(&self) -> i32 {
        self.length
    }

    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self {
        StreamEgress::new(defrag_entry.running_length as i32)
    }
}

/// Metadata for receiver stream channels, providing introspection for control and payload channels.
pub struct StreamRxMetaData {
    /// Metadata for the control channel.
    pub control: RxChannelMetaDataWrapper,
    /// Metadata for the payload channel.
    pub payload: RxChannelMetaDataWrapper,
}

/// Wrapper for receiver channel metadata, providing access to channel information.
#[derive(Debug)]
pub struct RxChannelMetaDataWrapper {
    /// The underlying channel metadata, wrapped in an Arc for thread-safe sharing.
    pub(crate) meta_data: Arc<ChannelMetaData>,
}

/// Implementation of metadata provider for receiver channel wrappers.
impl RxMetaDataProvider for RxChannelMetaDataWrapper {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}

/// Wrapper for transmitter channel metadata, providing access to channel information.
#[derive(Debug)]
pub struct TxChannelMetaDataWrapper {
    /// The underlying channel metadata, wrapped in an Arc for thread-safe sharing.
    pub(crate) meta_data: Arc<ChannelMetaData>,
}

/// Implementation of metadata provider for transmitter channel wrappers.
impl TxMetaDataProvider for TxChannelMetaDataWrapper {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}

/// Constant defining the bitmask for rate collector indexing.
pub const RATE_COLLECTOR_MASK: usize = 31;

/// Constant defining the length of the rate collector array.
pub const RATE_COLLECTOR_LEN: usize = 32;

/// Represents a transmitter for a steady stream, managing control and payload channels with defragmentation support.
pub struct StreamTx<T: StreamControlItem> {
    /// The control channel for sending stream metadata.
    pub(crate) control_channel: Tx<T>,
    /// The payload channel for sending raw data bytes.
    pub(crate) payload_channel: Tx<u8>,
    /// A map of session IDs to defragmentation entries for reassembling fragmented messages.
    defrag: AHashMap<i32, Defrag<T>>,
    /// A queue of session IDs with ready messages for processing.
    pub(crate) ready_msg_session: VecDeque<i32>,
    /// The timestamp of the last input data received.
    pub(crate) last_input_instant: Instant,
    /// The timestamp of the last output data sent.
    pub(crate) last_output_instant: Instant,
    /// The current index for the input rate collector.
    pub(crate) input_rate_index: usize,
    /// An array collecting input rate statistics (duration, messages, bytes).
    pub(crate) input_rate_collector: [(Duration, u32, u32); RATE_COLLECTOR_LEN],
    /// The maximum latency allowed for polling operations.
    pub(crate) max_poll_latency: Duration,
    /// The current index for the output rate collector.
    pub(crate) output_rate_index: usize,
    /// An array collecting output rate statistics (duration, messages, bytes).
    pub(crate) output_rate_collector: [(Duration, u32, u32); RATE_COLLECTOR_LEN],
    /// Cached values for available message and byte capacities.
    pub(crate) stored_vacant_values: (i32, i32),
    /// Flag indicating whether to focus on control channel metadata.
    pub(crate) spotlight_control: bool,
}

/// Implementation of debug formatting for transmitter streams.
impl<T: StreamControlItem> Debug for StreamTx<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamTx")
            .field("item_channel", &"Tx<T>")
            .field("payload_channel", &"Tx<u8>")
            .field("defrag_keys", &self.defrag.keys().collect::<Vec<_>>())
            .field("ready", &self.ready_msg_session)
            .finish()
    }
}

/// Represents a defragmentation entry for reassembling stream messages.
pub struct Defrag<T: StreamControlItem> {
    /// The time when the first fragment was received, if available.
    pub(crate) arrival: Option<Instant>,
    /// The time when the last fragment was received, if available.
    pub(crate) finish: Option<Instant>,
    /// The session identifier for the defragmentation entry.
    pub(crate) session_id: i32,
    /// The cumulative length of data in the defragmentation buffer.
    pub(crate) running_length: usize,
    /// Ring buffers for storing stream control items (producer and consumer).
    #[allow(clippy::type_complexity)]
    pub(crate) ringbuffer_items: (
        AsyncWrap<Arc<AsyncRb<Heap<T>>>, true, false>,
        AsyncWrap<Arc<AsyncRb<Heap<T>>>, false, true>,
    ),
    /// Ring buffers for storing raw byte data (producer and consumer).
    pub(crate) ringbuffer_bytes: (
        AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>,
        AsyncWrap<Arc<AsyncRb<Heap<u8>>>, false, true>,
    ),
}

/// Methods for managing defragmentation entries.
impl<T: StreamControlItem> Defrag<T> {
    /// Creates a new defragmentation entry with the specified session ID and buffer capacities.
    pub fn new(session_id: i32, items: usize, bytes: usize) -> Self {
        Defrag {
            arrival: None,
            finish: None,
            session_id,
            running_length: 0,
            ringbuffer_items: AsyncRb::<Heap<T>>::new(items).split(),
            ringbuffer_bytes: AsyncRb::<Heap<u8>>::new(bytes).split(),
        }
    }

    /// Ensures the defragmentation buffers have sufficient capacity for additional items and bytes.
    pub fn ensure_additional_capacity(&mut self, items: usize, bytes: usize) {
        // Handle ringbuffer_bytes
        let bytes_vacant = self.ringbuffer_bytes.0.vacant_len();
        if bytes_vacant < bytes {
            // Calculate new capacity: at least occupied + required, or double current capacity
            let current_capacity = self.ringbuffer_bytes.0.capacity();
            let occupied = self.ringbuffer_bytes.1.occupied_len();
            let required_capacity = occupied + bytes;
            let new_capacity = current_capacity.max(NonZero::try_from(required_capacity).expect("internal"));

            // Create new ring buffer and split it
            let new_rb = AsyncRb::<Heap<u8>>::new(usize::from(new_capacity));
            let (mut new_producer, new_consumer) = new_rb.split();

            // Transfer existing data from old consumer to new producer
            let mut buf = vec![0u8; 1024]; // Temporary buffer for slicing
            let count = self.ringbuffer_bytes.1.pop_slice(&mut buf);
            loop {
                if count == 0 {
                    break;
                }
                let pushed = new_producer.push_slice(&buf[0..count]);
                debug_assert_eq!(pushed, count, "Pushed bytes should match popped count");
                let _ = self.ringbuffer_bytes.1.pop_slice(&mut buf);
            }

            // Replace the old ringbuffer_bytes with the new one
            self.ringbuffer_bytes = (new_producer, new_consumer);
        }

        // Handle ringbuffer_items
        let items_vacant = self.ringbuffer_items.0.vacant_len();
        if items_vacant < items {
            // Calculate new capacity: at least occupied + required, or double current capacity
            let current_capacity = self.ringbuffer_items.0.capacity();
            let occupied = self.ringbuffer_items.1.occupied_len();
            let required_capacity = occupied + items;
            let new_capacity = current_capacity.max(NonZero::try_from(required_capacity).expect("internal"));

            // Create new ring buffer and split it
            let new_rb = AsyncRb::<Heap<T>>::new(usize::from(new_capacity));
            let (mut new_producer, new_consumer) = new_rb.split();

            // Transfer existing data from old consumer to new producer
            while let Some(item) = self.ringbuffer_items.1.try_pop() {
                let ok = new_producer.try_push(item).is_ok();
                debug_assert!(ok, "Pushed bytes should match popped count");
            }

            // Replace the old ringbuffer_items with the new one
            self.ringbuffer_items = (new_producer, new_consumer);
        }
    }
}

/// Methods for managing transmitter streams.
impl<T: StreamControlItem> StreamTx<T> {
    /// Creates a new transmitter stream with the specified control and payload channels.
    pub fn new(control_channel: Tx<T>, payload_channel: Tx<u8>) -> Self {
        StreamTx {
            max_poll_latency: Duration::from_millis(1000),
            stored_vacant_values: (control_channel.capacity() as i32, payload_channel.capacity() as i32),
            control_channel,
            payload_channel,
            defrag: Default::default(),
            ready_msg_session: VecDeque::with_capacity(4),
            last_input_instant: Instant::now(),
            input_rate_index: RATE_COLLECTOR_MASK,
            input_rate_collector: Default::default(),
            last_output_instant: Instant::now(),
            output_rate_index: RATE_COLLECTOR_MASK,
            output_rate_collector: Default::default(),
            spotlight_control: false,
        }
    }

    /// Sets the cached values for available message and byte capacities.
    pub(crate) fn set_stored_vacant_values(&mut self, messages: i32, total_bytes_for_messages: i32) {
        self.stored_vacant_values = (messages, total_bytes_for_messages);
    }

    /// Retrieves the cached values for available message and byte capacities.
    pub(crate) fn get_stored_vacant_values(&mut self) -> (i32, i32) {
        self.stored_vacant_values
    }

    /// Records input data rate statistics, including duration, message count, and byte count.
    pub fn store_input_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.input_rate_index += 1;
        self.input_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration, messages, total_bytes_for_messages);
    }

    /// Records output data rate statistics, including duration, message count, and byte count.
    pub fn store_output_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.output_rate_index += 1;
        self.output_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration, messages, total_bytes_for_messages);
    }

    /// Estimates the minimum and maximum durations for processing pending data based on available capacity and historical rates.
    pub fn next_poll_bounds(&self) -> (Duration, Duration) {
        if let Some(d) = self.fastest_byte_processing_duration() {
            let waiting_bytes = self.payload_channel.capacity() - self.payload_channel.shared_vacant_units();
            if waiting_bytes < 2 {
                (Duration::ZERO, self.max_poll_latency.min(d))
            } else {
                (
                    self.max_poll_latency.min(d.mul((waiting_bytes >> 1) as u32)),
                    self.max_poll_latency.min(d.mul((waiting_bytes - 1) as u32)),
                )
            }
        } else {
            (Duration::ZERO, self.max_poll_latency)
        }
    }

    /// Calculates the fastest byte processing duration based on historical output rate data.
    pub fn fastest_byte_processing_duration(&self) -> Option<Duration> {
        // Iterate over output_rate_collector to find the highest rate (bytes per second)
        let max_rate = self
            .output_rate_collector
            .iter()
            .filter_map(|&(duration, _, bytes)| {
                let duration_secs = duration.as_secs_f64();
                if duration_secs > 0.0 && bytes > 0 {
                    Some(bytes as f64 / duration_secs) // Bytes per second
                } else {
                    None
                }
            })
            .fold(None, |acc: Option<f64>, rate| match acc {
                None => Some(rate),
                Some(max) => Some(max.max(rate)),
            });

        // Convert max rate to duration per byte (seconds per byte)
        max_rate.map(|rate| {
            let seconds_per_byte = 1.0 / rate; // Seconds per byte
            Duration::from_secs_f64(seconds_per_byte)
        })
    }

    /// Estimates the mean and standard deviation of the duration between message arrivals based on input rate data.
    pub fn guess_duration_between_arrivals(&self) -> (Duration, Duration) {
        let mut sum: f64 = 0.0; // Sum of average times (in seconds)
        let mut sum_sq: f64 = 0.0; // Sum of squared average times
        let mut count: usize = 0; // Number of entries with m > 0

        // Single pass over the collector
        for &(d, m, _) in &self.input_rate_collector {
            if m > 0 {
                // Compute average time per message in seconds
                let avg_time = d.as_nanos() as f64 / m as f64 / 1_000_000_000.0; // Convert ns to s
                sum += avg_time;
                sum_sq += avg_time * avg_time;
                count += 1;
            }
        }

        // Handle edge cases
        if count == 0 {
            return (Duration::from_millis(1), Duration::from_millis(0));
        } else if count == 1 {
            return (Duration::from_secs_f64(sum), Duration::from_millis(0));
        }

        // Compute mean and sample standard deviation
        let mean = sum / count as f64;
        let variance = (sum_sq - sum * sum / count as f64) / (count as f64 - 1.0);
        let stddev = variance.sqrt();

        (Duration::from_secs_f64(mean), Duration::from_secs_f64(stddev))
    }

    /// Marks both control and payload channels as closed, signaling no further data will be sent.
    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }

    /// Returns the capacities of the control and payload channels.
    pub fn capacity(&self) -> (usize, usize) {
        (self.control_channel.capacity(), self.payload_channel.capacity())
    }

    /// Flushes ready defragmented messages to the control and payload channels, returning the number of messages and bytes processed.
    pub(crate) fn fragment_flush_ready<C: SteadyActor>(&mut self, actor: &mut C) -> (u32, u32) {
        let mut total_messages = 0;
        let mut total_bytes = 0;
        let mut to_consume = self.ready_msg_session.len();
        while let Some(session_id) = self.ready_msg_session.pop_front() {
            // Changed to pop_front directly
            to_consume -= 1;
            if let Some(defrag_entry) = self.defrag.get_mut(&session_id) {
                // how do we know how much we wrote??
                if let (msgs, bytes, Some(needs_more_work_for_session_id)) = actor.flush_defrag_messages(
                    &mut self.control_channel,
                    &mut self.payload_channel,
                    defrag_entry,
                ) {
                    total_messages += msgs;
                    total_bytes += bytes;
                    self.ready_msg_session.push_back(needs_more_work_for_session_id);
                }
            } else {
                error!("internal error, session reported without any defrag");
            }
            if to_consume == 0 {
                break;
            }
        }
        (total_messages, total_bytes)
    }

    /// Calculates the minimum available capacity for defragmentation across all sessions.
    pub(crate) fn defrag_has_room_for(&mut self) -> usize {
        let items: u128 = self.control_channel.capacity() as u128;
        let bytes: u128 = self.payload_channel.capacity() as u128;

        self.defrag
            .values()
            .map(|d| {
                // empty item count
                d.ringbuffer_items.0.vacant_len()
                    // actual items expected to fit in the available bytes
                    .min(((d.ringbuffer_bytes.0.vacant_len() as u128 * items) / bytes) as usize)
            })
            .min() // out of all sessions take the smallest in case that is what we get next.
            .unwrap_or(self.control_channel.capacity())
    }

    /// Consumes a fragment of data, storing it in the defragmentation buffer for the specified session.
    pub(crate) fn fragment_consume(&mut self, session_id: i32, slice: &[u8], is_begin: bool, is_end: bool, now: Instant) {
        debug_assert!(
            slice.len() <= self.payload_channel.capacity(),
            "Internal error, slice is too large"
        );
        // Get or create the Defrag entry for the session ID
        let defrag_entry: &mut Defrag<T> = self.defrag.entry(session_id).or_insert_with(|| {
            Defrag::new(session_id, self.control_channel.capacity(), self.payload_channel.capacity()) // Adjust capacity as needed
        });

        debug_assert!(slice.len() <= defrag_entry.ringbuffer_bytes.0.vacant_len());

        // If this is the beginning of a fragment, assert the ringbuffer is empty
        let some_now = Some(now);
        if is_begin {
            defrag_entry.arrival = some_now; // Set the arrival time to now
            if is_end {
                defrag_entry.finish = some_now;
            }
        } else if is_end {
            defrag_entry.finish = some_now;
        }

        // Append the slice to the ringbuffer (first half of the split)
        let slice_len = slice.len();
        debug_assert!(slice_len > 0);

        let count = defrag_entry.ringbuffer_bytes.0.push_slice(slice);
        debug_assert_eq!(
            count,
            slice_len,
            "internal buffer should have had room, check the channel definition to ensure it has enough bytes per item"
        );

        defrag_entry.running_length += count;

        // If this is the end of a fragment send
        if is_end {
            let result = defrag_entry.ringbuffer_items.0.try_push(T::from_defrag(defrag_entry));
            debug_assert!(result.is_ok());

            if !self.ready_msg_session.contains(&defrag_entry.session_id) {
                self.ready_msg_session.push_back(defrag_entry.session_id);
            };

            defrag_entry.running_length = 0;
            defrag_entry.arrival = None;
            defrag_entry.finish = None;
        }
    }
}

/// Represents a receiver for a steady stream, managing control and payload channels.
#[derive(Debug)]
pub struct StreamRx<T: StreamControlItem> {
    /// The control channel for receiving stream metadata.
    pub(crate) control_channel: Rx<T>,
    /// The payload channel for receiving raw data bytes.
    pub(crate) payload_channel: Rx<u8>,
    /// Flag indicating whether to focus on control channel metadata.
    pub(crate) spotlight_control: bool,
}

/// Methods for managing receiver streams.
impl<T: StreamControlItem> StreamRx<T> {
    /// Creates a new receiver stream with the specified control and payload channels.
    pub(crate) fn new(control_channel: Rx<T>, payload_channel: Rx<u8>) -> Self {
        StreamRx {
            control_channel,
            payload_channel,
            spotlight_control: false,
        }
    }

    /// Attempts to take a single message and its associated payload from the receiver.
    pub fn try_take(&mut self) -> Option<(T, Box<[u8]>)> {
        if let Some((_done, msg)) = self.shared_try_take() {
            Some(msg)
        } else {
            None
        }
    }

    /// Returns the capacity of the control channel.
    pub fn capacity(&mut self) -> usize {
        self.control_channel.capacity()
    }

    /// Returns the number of available units in the control and payload channels.
    pub fn avail_units(&mut self) -> (usize, usize) {
        self.shared_avail_units()
    }

    /// Checks if both control and payload channels are closed.
    pub fn is_closed(&mut self) -> bool {
        self.control_channel.is_closed() && self.payload_channel.is_closed()
    }

    /// Checks if both control and payload channels are empty.
    pub fn is_empty(&mut self) -> bool {
        self.control_channel.is_empty() && self.payload_channel.is_empty()
    }

    /// Consumes messages from the receiver, applying a provided function to process the data up to a byte limit.
    pub(crate) fn consume_messages<C: SteadyActor>(
        &mut self,
        actor: &mut C,
        byte_limit: usize,
        mut fun: impl FnMut(&mut [u8], &mut [u8]) -> bool,
    ) {
        // Obtain mutable slices from the item and payload channels
        let (item1, item2) = self.control_channel.rx.as_mut_slices();
        let (payload1, payload2) = self.payload_channel.rx.as_mut_slices();

        // Variables to track the state of the iteration
        let mut on_first = true; // Whether we are still processing the first payload slice
        let mut active_index = 0; // Current index in the active payload slice
        let mut active_items = 0; // Number of items processed
        let mut active_data: usize = 0; // Total bytes processed

        // Process items from the first slice
        for i in item1 {
            // Extract payload slices based on the current state
            let (a, b) = Self::extract_stream_payload_slices(payload1, payload2, &mut on_first, &mut active_index, i.length() as usize);

            // Apply the provided function to the payload slices
            if active_data + (i.length() as usize) > byte_limit || !fun(a, b) {
                // If the limit is reached or the function returns false, advance the read indices and exit
                let x = actor.advance_take_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
                let x = actor.advance_take_index(&mut self.control_channel, active_items);
                debug_assert_eq!(x.item_count(), active_items, "Item channel advance mismatch");
                return;
            }

            // Update the counts of processed items and data
            active_items += 1;
            active_data += i.length() as usize;
        }

        // Process items from the second slice
        for i in item2 {
            // Extract payload slices based on the current state
            let (a, b) = Self::extract_stream_payload_slices(payload1, payload2, &mut on_first, &mut active_index, i.length() as usize);

            // Apply the provided function to the payload slices
            if active_data + (i.length() as usize) > byte_limit || !fun(a, b) {
                // If the limit is reached or the function returns false, advance the read indices and exit
                let x = actor.advance_take_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
                let x = actor.advance_take_index(&mut self.control_channel, active_items);
                debug_assert_eq!(x.item_count(), active_items, "Item channel advance mismatch");
                return;
            }

            // Update the counts of processed items and data
            active_items += 1;
            active_data += i.length() as usize;
        }

        // If all items are processed successfully, advance the read indices
        let x = actor.advance_take_index(&mut self.payload_channel, active_data);
        debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
        let x = actor.advance_take_index(&mut self.control_channel, active_items);
        debug_assert_eq!(x.item_count(), active_items, "Item channel advance mismatch");
    }

    /// Extracts payload slices from the receiver's buffers for processing a message of the specified byte length.
    fn extract_stream_payload_slices<'a>(
        payload1: &'a mut [u8],
        payload2: &'a mut [u8],
        on_first: &mut bool,
        active_index: &mut usize,
        bytes: usize,
    ) -> (&'a mut [u8], &'a mut [u8]) {
        if *on_first {
            let payload_len = payload1.len();
            let p1_len = payload1.len() - *active_index;
            let len_a = p1_len.min(bytes); // Length of the slice from the first payload
            let len_b = bytes - len_a; // Length of the slice from the second payload

            // Create mutable slices from the payloads
            let a = &mut payload1[*active_index..(*active_index + len_a)];
            let b = &mut payload2[0..len_b];

            // Update the active index and check if we need to switch to the second payload
            *active_index += len_a;
            if *active_index >= payload_len || len_b > 0 {
                *on_first = false;
                *active_index = len_b; // Reset active_index for payload2
            }
            (a, b)
        } else {
            let len_b = (payload2.len() - *active_index).min(bytes); // Length of the slice from the second payload

            let a = &mut payload2[*active_index..(*active_index + len_b)];
            let b = &mut payload1[0..0]; // Empty slice from the first payload
            *active_index += len_b;

            (a, b)
        }
    }
}

/// Type alias for a thread-safe, mutex-protected receiver stream.
pub type SteadyStreamRx<T> = Arc<Mutex<StreamRx<T>>>;

/// Type alias for a thread-safe, mutex-protected transmitter stream.
pub type SteadyStreamTx<T> = Arc<Mutex<StreamTx<T>>>;

/// A lazy-initialized wrapper for stream channels, deferring construction until first use.
#[derive(Debug)]
pub(crate) struct LazyStream<T: StreamControlItem> {
    /// The builder for the control channel, stored until the channel is constructed.
    control_builder: Mutex<Option<ChannelBuilder>>,
    /// The builder for the payload channel, stored until the channel is constructed.
    payload_builder: Mutex<Option<ChannelBuilder>>,
    /// The constructed transmitter and receiver channels, if initialized.
    channel: Mutex<Option<(SteadyStreamTx<T>, SteadyStreamRx<T>)>>,
}

/// Methods for managing lazy-initialized stream channels.
impl<T: StreamControlItem> LazyStream<T> {
    /// Creates a new lazy stream with the specified channel builders for control and payload channels.
    pub(crate) fn new(item_builder: &ChannelBuilder, payload_builder: &ChannelBuilder) -> Self {
        LazyStream {
            control_builder: Mutex::new(Some(item_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
        }
    }

    /// Retrieves or constructs the transmitter channel, returning a thread-safe clone.
    pub(crate) async fn get_tx_clone(&self) -> SteadyStreamTx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self
                .control_builder
                .lock()
                .await
                .take()
                .expect("internal error: control_builder missing");
            let data_builder = self
                .payload_builder
                .lock()
                .await
                .take()
                .expect("internal error: payload_builder missing");

            let (meta_tx, meta_rx) = meta_builder.eager_build_internal();
            let (data_tx, data_rx) = data_builder.eager_build_internal();

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    /// Retrieves or constructs the receiver channel, returning a thread-safe clone.
    pub(crate) async fn get_rx_clone(&self) -> SteadyStreamRx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self
                .control_builder
                .lock()
                .await
                .take()
                .expect("internal error: control_builder missing");
            let data_builder = self
                .payload_builder
                .lock()
                .await
                .take()
                .expect("internal error: payload_builder missing");

            let (meta_tx, meta_rx) = meta_builder.eager_build_internal();
            let (data_tx, data_rx) = data_builder.eager_build_internal();

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}

/// A lazy-initialized wrapper for transmitter streams.
#[derive(Debug)]
pub struct LazyStreamTx<T: StreamControlItem> {
    /// The underlying lazy stream channel.
    lazy_channel: Arc<LazyStream<T>>,
}

/// Methods for managing lazy-initialized transmitter streams.
impl<T: StreamControlItem> LazyStreamTx<T> {
    /// Creates a new lazy transmitter stream from a shared lazy stream channel.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamTx { lazy_channel }
    }

    /// Retrieves or constructs the underlying transmitter stream, returning a thread-safe clone.
    pub fn clone(&self) -> SteadyStreamTx<T> {
        core_exec::block_on(self.lazy_channel.get_tx_clone())
    }

    /// Sends a test frame by transmitting a payload and its metadata.
    ///
    /// Panics if the entire payload cannot be sent or if metadata sending fails.
    pub fn testing_send_frame(&self, data: &[u8]) {
        let s = self.clone();

        let mut l = s.try_lock().expect("internal error: try_lock");

        let x = l.payload_channel.shared_send_slice_until_full(data);

        assert_eq!(x, data.len(), "Not all bytes were sent!");
        assert_ne!(x, 0);

        match l.control_channel.shared_try_send(T::testing_new(x as i32)) {
            Ok(_) => {}
            Err(_) => {
                panic!("error sending metadata");
            }
        };
    }

    /// Sends multiple test frames with their metadata and optionally closes the channels.
    ///
    /// Panics if any payload or metadata cannot be sent.
    pub fn testing_send_all(&self, data: Vec<(T, &[u8])>, close: bool) {
        let s = self.clone();
        let mut l = s.try_lock().expect("internal error: try_lock");

        for d in data.into_iter() {
            let x = l.payload_channel.shared_send_slice_until_full(d.1);
            match l.control_channel.shared_try_send(T::testing_new(x as i32)) {
                Ok(_) => {}
                Err(_) => {
                    panic!("error sending metadata, actor must be running or the channel needs to be longer");
                }
            };
            assert_eq!(x, d.1.len());
        }
        if close {
            l.mark_closed(); // for clean shutdown we tell the actor we have no more data
        }
    }

    /// Closes the underlying control and payload channels, signaling no further data will be sent.
    pub fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.try_lock().expect("internal error: try_lock");

        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }
}

/// A lazy-initialized wrapper for receiver streams.
#[derive(Debug)]
pub struct LazyStreamRx<T: StreamControlItem> {
    /// The underlying lazy stream channel.
    lazy_channel: Arc<LazyStream<T>>,
}

/// Methods for managing lazy-initialized receiver streams.
impl<T: StreamControlItem> LazyStreamRx<T> {
    /// Creates a new lazy receiver stream from a shared lazy stream channel.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamRx { lazy_channel }
    }

    /// Retrieves or constructs the underlying receiver stream, returning a thread-safe clone.
    pub fn clone(&self) -> SteadyStreamRx<T> {
        core_exec::block_on(self.lazy_channel.get_rx_clone())
    }

    /// Returns the number of available units in the receiver for testing purposes.
    pub fn testing_avail_units(&self) -> usize {
        let s = self.clone();
        let mut rx = s.try_lock().expect("internal error: try_lock");
        rx.shared_avail_units().0
    }

    /// Takes all available messages from the receiver for testing purposes.
    pub fn testing_take_all(&self) -> Vec<(T, Box<[u8]>)> {
        let s = self.clone();
        let mut rx = s.try_lock().expect("internal error: try_lock");
        let mut count = rx.capacity().min(rx.avail_units().0);
        let mut target = Vec::with_capacity(count);
        while count > 0 {
            target.push(rx.try_take().expect("internal error: try_take"));
            count -= 1;
        }
        target
    }

    /// Waits for a specified number of units to become available or for a timeout, returning whether the condition was met.
    pub fn testing_avail_wait(&self, count: usize,  timeout_duration: Duration) -> bool {
        core_exec::block_on(async {
            let s = self.clone();
            let mut l = s.lock().await;

            // Define futures and apply .fuse()
            let wait_fut = l.shared_wait_closed_or_avail_units(count).fuse();
            let timeout_fut = Delay::new(timeout_duration).fuse();

            // Pin the futures on the stack
            pin_mut!(wait_fut);
            pin_mut!(timeout_fut);

            select! {
                result = wait_fut => result, // Return the result if wait_fut completes first
                _ = timeout_fut => false,    // Return false if timeout_fut completes first
            }
        })
    }
}

/// Implementation of metadata provider for receiver streams, selecting between control and payload metadata.
impl<T: StreamControlItem> RxMetaDataProvider for SteadyStreamRx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => {
                if guard.spotlight_control {
                    guard.control_channel.channel_meta_data.meta_data()
                } else {
                    guard.payload_channel.channel_meta_data.meta_data()
                }
            }
            None => {
                let guard = core_exec::block_on(self.lock());
                if guard.spotlight_control {
                    guard.control_channel.channel_meta_data.meta_data()
                } else {
                    guard.payload_channel.channel_meta_data.meta_data()
                }
            }
        }
    }
}

/// Implementation of metadata provider for transmitter streams, selecting between control and payload metadata.
impl<T: StreamControlItem> TxMetaDataProvider for SteadyStreamTx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => {
                if guard.spotlight_control {
                    guard.control_channel.channel_meta_data.meta_data()
                } else {
                    guard.payload_channel.channel_meta_data.meta_data()
                }
            }
            None => {
                let guard = core_exec::block_on(self.lock());
                if guard.spotlight_control {
                    guard.control_channel.channel_meta_data.meta_data()
                } else {
                    guard.payload_channel.channel_meta_data.meta_data()
                }
            }
        }
    }
}

#[cfg(test)]
mod extra_stream_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Tests the behavior of extracting payload slices from receiver buffers, covering both first and second slice cases.
    #[test]
    fn test_extract_stream_payload_slices_behavior() {
        let mut p1 = [1u8, 2, 3];
        let mut p2 = [4u8, 5, 6, 7];
        let mut on_first = true;
        let mut idx = 0;
        // First slice: take 2 bytes from p1
        let (a1, b1) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a1, &mut [1u8, 2][..]);
        assert_eq!(b1.len(), 0);
        assert!(on_first);
        assert_eq!(idx, 2);
        // Second slice: p1 has 1 left, so a from p1[2..3], b from p2[0..1]
        let (a2, b2) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a2, &mut [3u8][..]);
        assert_eq!(b2, &mut [4u8][..]);
        assert!(!on_first);
        assert_eq!(idx, 1);
        // Third slice: on_first=false, consume from p2
        let (a3, b3) = StreamRx::<StreamEgress>::extract_stream_payload_slices(&mut p1, &mut p2, &mut on_first, &mut idx, 2);
        assert_eq!(a3, &mut [5u8, 6][..]);
        assert_eq!(b3.len(), 0);
    }

    /// Tests the initialization of defragmentation entries, verifying field values and buffer capacities.
    #[test]
    fn test_defrag_new_properties() {
        let items = 3;
        let bytes = 5;
        let session_id = 42;
        let def: Defrag<StreamEgress> = Defrag::new(session_id, items, bytes);
        assert_eq!(def.session_id, session_id);
        assert_eq!(def.running_length, 0);
        assert!(def.arrival.is_none());
        assert!(def.finish.is_none());
        // Writer side vacant_len equals capacity
        assert_eq!(def.ringbuffer_items.0.vacant_len(), items);
        assert_eq!(def.ringbuffer_bytes.0.vacant_len(), bytes);
    }

    /// Tests the creation of incoming stream fragments from defragmentation entries.
    #[test]
    fn test_stream_session_message_new_and_from_defrag() {
        let arrival = Instant::now();
        let finish = arrival + Duration::from_secs(1);
        let mut def = Defrag::<StreamIngress>::new(7, 4, 4);
        def.arrival = Some(arrival);
        def.finish = Some(finish);
        def.running_length = 8;
        let msg = StreamIngress::from_defrag(&def);
        assert_eq!(msg.length(), 8);
        assert_eq!(msg.session_id, 7);
        assert_eq!(msg.arrival, arrival);
        assert_eq!(msg.finished, finish);
    }

    /// Tests the creation of incoming stream fragments for testing purposes.
    #[test]
    fn test_testing_new_methods() {
        let tn = StreamIngress::testing_new(9);
        assert_eq!(tn.length(), 9);
    }

    /// Tests the relationship between rate collector constants.
    #[test]
    fn test_rate_collector_constants() {
        assert_eq!(RATE_COLLECTOR_LEN, 32);
        assert_eq!(RATE_COLLECTOR_MASK, 31);
        // Masking LEN yields zero
        assert_eq!(RATE_COLLECTOR_LEN & RATE_COLLECTOR_MASK, 0);
    }

    /// Tests the behavior of marking an empty transmitter bundle as closed.
    #[test]
    fn test_stream_tx_bundle_trait_empty() {
        type Bundle = StreamTxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.mark_closed(), "even Empty bundle should return true");
    }

    /// Tests the state inspection methods for an empty receiver bundle.
    #[test]
    fn test_stream_rx_bundle_trait_empty() {
        type Bundle = StreamRxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.is_closed_and_empty());
        assert!(bundle.is_closed());
        assert!(bundle.is_empty());
    }

    /// Tests cloning an empty transmitter bundle.
    #[test]
    fn test_lazy_tx_bundle_clone_empty() {
        let empty: [LazyStreamTx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamTxBundle<StreamEgress, 0> = empty.clone();
        // An Arc<[..;0]> has length 0
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// Tests cloning an empty receiver bundle.
    #[test]
    fn test_lazy_rx_bundle_clone_empty() {
        let empty: [LazyStreamRx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamRxBundle<StreamEgress, 0> = empty.clone();
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// Tests the behavior of receiver bundle operations for an empty bundle.
    #[test]
    fn test_steady_rx_bundle_trait_empty() {
        let bundle: SteadyStreamRxBundle<StreamEgress, 0> = Arc::new([]);
        // lock() is a JoinAll over zero futures: completes immediately
        let guards: Vec<_> = core_exec::block_on(bundle.lock());
        assert!(guards.is_empty());

        let ctrl = bundle.control_meta_data();
        assert_eq!(ctrl.len(), 0, "control_meta_data for GIRTH=0 must be length 0");
        let payload = bundle.payload_meta_data();
        assert_eq!(payload.len(), 0, "payload_meta_data for GIRTH=0 must be length 0");
    }

    /// Tests the behavior of transmitter bundle operations for an empty bundle.
    #[test]
    fn test_steady_tx_bundle_trait_empty() {
        let bundle: SteadyStreamTxBundle<StreamEgress, 0> = Arc::new([]);
        let guards: Vec<_> = core_exec::block_on(bundle.lock());
        assert!(guards.is_empty());

        let ctrl = bundle.control_meta_data();
        assert_eq!(ctrl.len(), 0, "control_meta_data for GIRTH=0 must be length 0");
        let payload = bundle.payload_meta_data();
        assert_eq!(payload.len(), 0, "payload_meta_data for GIRTH=0 must be length 0");
    }

    #[test]
    fn test_defrag_ensure_additional_capacity() {
        let mut defrag = Defrag::<StreamEgress>::new(1, 2, 2);
        defrag.ensure_additional_capacity(5, 10);
        assert!(defrag.ringbuffer_items.0.vacant_len() >= 5);
        assert!(defrag.ringbuffer_bytes.0.vacant_len() >= 10);
    }
}
