//! Steady stream module for managing lazy-initialized Tx and Rx channels.
//! We use one stream per channel. We can have N streams as a const array
//! going into `aeron_publish` and N streams as a const array coming from
//! `aeron_subscribe`.

// ss[related distributed.aqueduct-stream]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::{channel_builder::ChannelBuilder, Rx, SteadyActor, Tx};
// ss[related philosophy.structural-hierarchy]
use ahash::AHashMap;
// ss[related distributed.aqueduct-stream]
use async_ringbuf::wrap::AsyncWrap;
// ss[related philosophy.structural-hierarchy]
use async_ringbuf::AsyncRb;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::{Mutex, MutexGuard, MutexLockFuture};
// ss[related distributed.aqueduct-stream]
use ringbuf::consumer::Consumer;
// ss[related philosophy.structural-hierarchy]
use ringbuf::producer::Producer;
// ss[related philosophy.structural-hierarchy]
use ringbuf::storage::Heap;
// ss[related distributed.aqueduct-stream]
use ringbuf::traits::{Observer, Split};
// ss[related philosophy.structural-hierarchy]
use std::collections::VecDeque;
// ss[related philosophy.structural-hierarchy]
use std::fmt::{Debug, Formatter};
// ss[related distributed.aqueduct-stream]
use std::num::NonZero;
// ss[related philosophy.structural-hierarchy]
use std::ops::Mul;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related distributed.aqueduct-stream]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use futures_timer::Delay;
// ss[related philosophy.structural-hierarchy]
use futures_util::select;
// ss[related distributed.aqueduct-stream]
use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
use crate::monitor::ChannelMetaData;
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::RxMetaDataProvider;
// ss[related distributed.aqueduct-stream]
use crate::steady_tx::TxMetaDataProvider;
// ss[related philosophy.structural-hierarchy]
use crate::core_exec;
// ss[related philosophy.structural-hierarchy]
use futures::future::FutureExt; // For .fuse()
// ss[related distributed.aqueduct-stream]
use futures::pin_mut;
// ss[related philosophy.structural-hierarchy]
use log::{error, trace};
// For pin_mut!

/// Type alias for the identifier used in Aeron, typically a 32-bit integer for stream or session IDs.
// ss[related distributed.aqueduct-stream]
pub type IdType = i32;

/// Type alias for an array of fixed size (GIRTH) containing thread-safe transmitters (Tx) for lazy-initialized streams.
// ss[related distributed.aqueduct-stream]
pub type LazySteadyStreamTxBundle<T, const GIRTH: usize> = [LazyStreamTx<T>; GIRTH];

/// Type alias for an array of fixed size (GIRTH) containing thread-safe receivers (Rx) for lazy-initialized streams.
// ss[related distributed.aqueduct-stream]
pub type LazySteadyStreamRxBundle<T, const GIRTH: usize> = [LazyStreamRx<T>; GIRTH];

/// Trait for cloning a bundle of lazy-initialized transmitter streams, triggering channel initialization if needed.
// ss[related distributed.aqueduct-stream]
pub trait LazySteadyStreamTxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Creates a new bundle of thread-safe transmitters by cloning the lazy-initialized channels and initializing them if not already done.
    // ss[related philosophy.structural-hierarchy]
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH>;
}

/// Trait for cloning a bundle of lazy-initialized receiver streams, triggering channel initialization if needed.
// ss[related distributed.aqueduct-stream]
pub trait LazySteadyStreamRxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Creates a new bundle of thread-safe receivers by cloning the lazy-initialized channels and initializing them if not already done.
    // ss[related philosophy.structural-hierarchy]
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH>;
}

/// Implementation of cloning for a bundle of lazy-initialized transmitter streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamTxBundleClone<T, GIRTH> for LazySteadyStreamTxBundle<T, GIRTH> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamRxBundleClone<T, GIRTH> for LazySteadyStreamRxBundle<T, GIRTH> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
pub type SteadyStreamRxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamRx<T>; GIRTH]>;

/// Type alias for a thread-safe, fixed-size array of transmitter streams wrapped in an Arc.
// ss[related distributed.aqueduct-stream]
pub type SteadyStreamTxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamTx<T>; GIRTH]>;

/// Trait providing methods for interacting with a bundle of receiver streams.
// ss[related distributed.aqueduct-stream]
pub trait SteadyStreamRxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
    /// Acquires the guards for all receivers in the bundle, returning a future that resolves when every guard is bound.
    // ss[related philosophy.structural-hierarchy]
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>>;

    /// Guard-first alias for [`SteadyStreamRxBundleTrait::lock`] — the preferred spelling.
    ///
    /// Identical future and semantics; only the vocabulary changes. Hold the guards for the
    /// life of the actor; this is not a mutex critical section.
    // ss[related distributed.aqueduct-stream]
    fn acquire_guard(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>> {
        self.lock()
    }

    /// Retrieves metadata for the control channels of all receivers in the bundle.
    // ss[related distributed.aqueduct-stream]
    fn control_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];

    /// Retrieves metadata for the payload channels of all receivers in the bundle.
    // ss[related distributed.aqueduct-stream]
    fn payload_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];
}

/// Implementation of receiver bundle operations for a thread-safe array of receiver streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {
    // ss[related philosophy.structural-hierarchy]
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    // ss[related distributed.aqueduct-stream]
    fn control_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| steady_stream as &dyn RxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    // ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
pub trait SteadyStreamTxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
    /// Acquires the guards for all transmitters in the bundle, returning a future that resolves when every guard is bound.
    // ss[related philosophy.structural-hierarchy]
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>>;

    /// Guard-first alias for [`SteadyStreamTxBundleTrait::lock`] — the preferred spelling.
    ///
    /// Identical future and semantics; only the vocabulary changes. Hold the guards for the
    /// life of the actor; this is not a mutex critical section.
    // ss[related distributed.aqueduct-stream]
    fn acquire_guard(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>> {
        self.lock()
    }

    /// Retrieves metadata for the control channels of all transmitters in the bundle.
    // ss[related distributed.aqueduct-stream]
    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];

    /// Retrieves metadata for the payload channels of all transmitters in the bundle.
    // ss[related distributed.aqueduct-stream]
    fn payload_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];
}

/// Implementation of transmitter bundle operations for a thread-safe array of transmitter streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {
    // ss[related philosophy.structural-hierarchy]
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    // ss[related distributed.aqueduct-stream]
    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|steady_stream| steady_stream as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    // ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
pub type StreamTxBundle<'a, T> = Vec<MutexGuard<'a, StreamTx<T>>>;

/// Type alias for a vector of locked receiver stream guards.
// ss[related distributed.aqueduct-stream]
pub type StreamRxBundle<'a, T> = Vec<MutexGuard<'a, StreamRx<T>>>;

/// Trait for managing a bundle of transmitter channels during runtime.
// ss[related distributed.aqueduct-stream]
pub trait StreamTxBundleTrait {
    /// Marks all channels in the bundle as closed, signaling that no further data will be sent.
    // ss[related philosophy.structural-hierarchy]
    fn mark_closed(&mut self) -> bool;
}

/// Trait for inspecting the state of a bundle of receiver channels during runtime.
// ss[related distributed.aqueduct-stream]
pub trait StreamRxBundleTrait {
    /// Checks if all channels in the bundle are closed and have no remaining data.
    // ss[related philosophy.structural-hierarchy]
    fn is_closed_and_empty(&mut self) -> bool;

    /// Checks if all channels in the bundle are closed.
    // ss[related distributed.aqueduct-stream]
    fn is_closed(&mut self) -> bool;

    /// Checks if all channels in the bundle have no remaining data.
    // ss[related distributed.aqueduct-stream]
    fn is_empty(&mut self) -> bool;
}

/// Implementation of transmitter bundle operations for a vector of locked transmitter streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> StreamTxBundleTrait for StreamTxBundle<'_, T> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> StreamRxBundleTrait for StreamRxBundle<'_, T> {
    // ss[related philosophy.structural-hierarchy]
    fn is_closed_and_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed_and_empty())
    }

    // ss[related distributed.aqueduct-stream]
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }

    // ss[related distributed.aqueduct-stream]
    fn is_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_empty())
    }
}

//////////////////////////////

/// Trait for items that can be transmitted or received over a stream, providing metadata and construction methods.
// ss[related distributed.aqueduct-stream]
pub trait StreamControlItem: Copy + Send + Sync + 'static {
    /// Creates a new instance for testing purposes with the specified length.
    // ss[related philosophy.structural-hierarchy]
    fn testing_new(length: i32) -> Self;

    /// Returns the length of the item in bytes.
    // ss[related distributed.aqueduct-stream]
    fn length(&self) -> i32;

    /// Constructs a new stream item from a defragmentation entry.
    // ss[related distributed.aqueduct-stream]
    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self;
}

/// Represents an incoming stream fragment, typically part of a multi-part message, with metadata for session and timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// ss[depends stream.control-payload]
// ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
impl Default for StreamIngress {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
impl StreamIngress {
    /// Creates a new incoming stream fragment with the specified parameters.
    ///
    /// Panics if the length is negative.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
    pub fn by_box(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, Box<[u8]>) {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0.into())
    }

    /// Creates a new fragment and returns it with a reference to the input byte slice.
    // ss[related distributed.aqueduct-stream]
    pub fn by_ref(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8]) {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0)
    }

    /// Alias for `by_ref`, creating a new fragment with a reference to the input byte slice.
    // ss[related distributed.aqueduct-stream]
    pub fn build(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8]) {
        StreamIngress::by_ref(session_id, arrival, finished, p0)
    }
}

/// Implementation of stream control item functionality for incoming fragments.
// ss[related distributed.aqueduct-stream]
impl StreamControlItem for StreamIngress {
    // ss[related philosophy.structural-hierarchy]
    fn testing_new(length: i32) -> Self {
        StreamIngress {
            length,
            session_id: 0,
            arrival: Instant::now(),
            finished: Instant::now(),
        }
    }

    // ss[related distributed.aqueduct-stream]
    fn length(&self) -> i32 {
        self.length
    }

    // ss[related distributed.aqueduct-stream]
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
// ss[depends stream.control-payload]
// ss[related distributed.aqueduct-stream]
pub struct StreamEgress {
    /// Length of the message in bytes.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) length: i32,
}

/// Methods for creating and manipulating outgoing stream messages.
// ss[related distributed.aqueduct-stream]
impl StreamEgress {
    /// Creates a new outgoing stream message and returns it with an owned byte buffer.
    // ss[related philosophy.structural-hierarchy]
    pub fn build(p0: &[u8]) -> (StreamEgress, Box<[u8]>) {
        StreamEgress::by_box(p0)
    }

    /// Creates a new outgoing stream message and returns it with an owned byte buffer.
    // ss[related distributed.aqueduct-stream]
    pub fn by_box(p0: &[u8]) -> (StreamEgress, Box<[u8]>) {
        (StreamEgress::new(p0.len() as i32), p0.into())
    }

    /// Creates a new outgoing stream message and returns it with a reference to the input byte slice.
    // ss[related distributed.aqueduct-stream]
    pub fn by_ref(p0: &[u8]) -> (StreamEgress, &[u8]) {
        (StreamEgress::new(p0.len() as i32), p0)
    }

    /// Creates a new outgoing stream message with the specified length.
    ///
    /// Panics if the length is negative.
    // ss[related distributed.aqueduct-stream]
    pub fn new(length: i32) -> Self {
        assert!(length >= 0, "Message length cannot be negative");
        StreamEgress { length }
    }
}

/// Implementation of stream control item functionality for outgoing messages.
// ss[related distributed.aqueduct-stream]
impl StreamControlItem for StreamEgress {
    // ss[related philosophy.structural-hierarchy]
    fn testing_new(length: i32) -> Self {
        StreamEgress { length }
    }

    // ss[related distributed.aqueduct-stream]
    fn length(&self) -> i32 {
        self.length
    }

    // ss[related distributed.aqueduct-stream]
    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self {
        StreamEgress::new(defrag_entry.running_length as i32)
    }
}

/// Metadata for receiver stream channels, providing introspection for control and payload channels.
// ss[related distributed.aqueduct-stream]
pub struct StreamRxMetaData {
    /// Metadata for the control channel.
    pub control: RxChannelMetaDataWrapper,
    /// Metadata for the payload channel.
    pub payload: RxChannelMetaDataWrapper,
}

/// Wrapper for receiver channel metadata, providing access to channel information.
#[derive(Debug)]
// ss[related distributed.aqueduct-stream]
pub struct RxChannelMetaDataWrapper {
    /// The underlying channel metadata, wrapped in an Arc for thread-safe sharing.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) meta_data: Arc<ChannelMetaData>,
}

/// Implementation of metadata provider for receiver channel wrappers.
// ss[related distributed.aqueduct-stream]
impl RxMetaDataProvider for RxChannelMetaDataWrapper {
    // ss[related philosophy.structural-hierarchy]
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}

/// Wrapper for transmitter channel metadata, providing access to channel information.
#[derive(Debug)]
// ss[related distributed.aqueduct-stream]
pub struct TxChannelMetaDataWrapper {
    /// The underlying channel metadata, wrapped in an Arc for thread-safe sharing.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) meta_data: Arc<ChannelMetaData>,
}

/// Implementation of metadata provider for transmitter channel wrappers.
// ss[related distributed.aqueduct-stream]
impl TxMetaDataProvider for TxChannelMetaDataWrapper {
    // ss[related philosophy.structural-hierarchy]
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}

/// Constant defining the bitmask for rate collector indexing.
// ss[related distributed.aqueduct-stream]
pub const RATE_COLLECTOR_MASK: usize = 31;

/// Constant defining the length of the rate collector array.
// ss[related distributed.aqueduct-stream]
pub const RATE_COLLECTOR_LEN: usize = 32;


// ss[related distributed.aqueduct-stream]
#[path = "aqueduct_stream_tx.rs"]
mod aqueduct_stream_tx;
#[path = "aqueduct_stream_rx.rs"]
mod aqueduct_stream_rx;
pub use aqueduct_stream_tx::{StreamTx, Defrag};
pub use aqueduct_stream_rx::StreamRx;

/// Type alias for a thread-safe, mutex-protected receiver stream.
// ss[related distributed.aqueduct-stream]
pub type SteadyStreamRx<T> = Arc<Mutex<StreamRx<T>>>;

/// Type alias for a thread-safe, mutex-protected transmitter stream.
// ss[related distributed.aqueduct-stream]
pub type SteadyStreamTx<T> = Arc<Mutex<StreamTx<T>>>;

/// A lazy-initialized wrapper for stream channels, deferring construction until first use.
#[derive(Debug)]
// ss[related distributed.aqueduct-stream]
pub(crate) struct LazyStream<T: StreamControlItem> {
    /// The builder for the control channel, stored until the channel is constructed.
    control_builder: Mutex<Option<ChannelBuilder>>,
    /// The builder for the payload channel, stored until the channel is constructed.
    payload_builder: Mutex<Option<ChannelBuilder>>,
    /// The constructed transmitter and receiver channels, if initialized.
    channel: Mutex<Option<(SteadyStreamTx<T>, SteadyStreamRx<T>)>>,
}

/// Methods for managing lazy-initialized stream channels.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> LazyStream<T> {
    /// Creates a new lazy stream with the specified channel builders for control and payload channels.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn new(item_builder: &ChannelBuilder, payload_builder: &ChannelBuilder) -> Self {
        LazyStream {
            control_builder: Mutex::new(Some(item_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
        }
    }

    /// Retrieves or constructs the transmitter channel, returning a thread-safe clone.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
pub struct LazyStreamTx<T: StreamControlItem> {
    /// The underlying lazy stream channel.
    lazy_channel: Arc<LazyStream<T>>,
}

/// Methods for managing lazy-initialized transmitter streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> LazyStreamTx<T> {
    /// Creates a new lazy transmitter stream from a shared lazy stream channel.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamTx { lazy_channel }
    }

    /// Retrieves or constructs the underlying transmitter stream, returning a thread-safe clone.
    // ss[related distributed.aqueduct-stream]
    pub fn clone(&self) -> SteadyStreamTx<T> {
        core_exec::block_on(self.lazy_channel.get_tx_clone())
    }

    /// Sends a test frame by transmitting a payload and its metadata.
    ///
    /// Panics if the entire payload cannot be sent or if metadata sending fails.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
    pub fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.try_lock().expect("internal error: try_lock");

        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }
}

/// A lazy-initialized wrapper for receiver streams.
#[derive(Debug)]
// ss[related distributed.aqueduct-stream]
pub struct LazyStreamRx<T: StreamControlItem> {
    /// The underlying lazy stream channel.
    lazy_channel: Arc<LazyStream<T>>,
}

/// Methods for managing lazy-initialized receiver streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> LazyStreamRx<T> {
    /// Creates a new lazy receiver stream from a shared lazy stream channel.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamRx { lazy_channel }
    }

    /// Retrieves or constructs the underlying receiver stream, returning a thread-safe clone.
    // ss[related distributed.aqueduct-stream]
    pub fn clone(&self) -> SteadyStreamRx<T> {
        core_exec::block_on(self.lazy_channel.get_rx_clone())
    }

    /// Returns the number of available units in the receiver for testing purposes.
    // ss[related distributed.aqueduct-stream]
    pub fn testing_avail_units(&self) -> usize {
        let s = self.clone();
        let mut rx = s.try_lock().expect("internal error: try_lock");
        rx.shared_avail_units().0
    }

    /// Takes all available messages from the receiver for testing purposes.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> RxMetaDataProvider for SteadyStreamRx<T> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> TxMetaDataProvider for SteadyStreamTx<T> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
#[path = "aqueduct_stream_tests.rs"]
mod aqueduct_stream_tests;

#[cfg(test)]
#[path = "aqueduct_stream_proptest.rs"]
// ss[related distributed.aqueduct-stream]
mod aqueduct_stream_proptest;
