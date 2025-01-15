
//! Steady stream module for managing lazy-initialized Tx and Rx channels.
//! We use one stream per channel. We can have N streams as a const array
//! going into `aeron_publish` and N streams as a const array coming from
//! `aeron_subscribe`.

use std::sync::Arc;
use std::time::Instant;
use std::ops::BitOr;
use futures_util::lock::{Mutex, MutexGuard, MutexLockFuture};

use crate::{monitor::{RxMetaData, TxMetaData}, channel_builder::ChannelBuilder, Rx, Tx};

/// Type alias for ID used in Aeron. Aeron commonly uses `i32` for stream/session IDs.
pub type IdType = i32;

/// Type alias for an array of thread-safe transmitters (Tx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of transmitters that can be shared across multiple threads.
pub type LazySteadyStreamTxBundle<T, const GIRTH: usize> = [LazyStreamTx<T>; GIRTH];
pub type LazySteadyStreamRxBundle<T, const GIRTH: usize> = [LazyStreamRx<T>; GIRTH];


/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
/// This one is special because its clone will lazy create teh channels.
///
pub trait LazySteadyStreamTxBundleClone<T: SteadyStreamItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH>;
}
pub trait LazySteadyStreamRxBundleClone<T: SteadyStreamItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH>;
}


impl<T: SteadyStreamItem, const GIRTH: usize> LazySteadyStreamTxBundleClone<T, GIRTH> for LazySteadyStreamTxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH> {
        let tx_clones:Vec<SteadyStreamTx<T>> = self.iter().map(|l|l.clone()).collect();
        match tx_clones.try_into() {
            Ok(array) => Arc::new(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }
}
impl<T: SteadyStreamItem, const GIRTH: usize> LazySteadyStreamRxBundleClone<T, GIRTH> for LazySteadyStreamRxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH> {
        let rx_clones:Vec<SteadyStreamRx<T>> = self.iter().map(|l|l.clone()).collect();
        match rx_clones.try_into() {
            Ok(array) => Arc::new(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }
}

pub type SteadyStreamRxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamRx<T>; GIRTH]>;
pub type SteadyStreamTxBundle<T, const GIRTH: usize> = Arc<[SteadyStreamTx<T>; GIRTH]>;


pub trait SteadyStreamRxBundleTrait<T: SteadyStreamItem, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn control_meta_data(&self) -> [RxMetaData; GIRTH];
    fn payload_meta_data(&self) -> [RxMetaData; GIRTH];


}

impl<T: SteadyStreamItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {

    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(&self) -> [RxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data().control)
            .collect::<Vec<RxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [RxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data().payload)
            .collect::<Vec<RxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

}

pub trait SteadyStreamTxBundleTrait<T: SteadyStreamItem, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn control_meta_data(&self) -> [TxMetaData; GIRTH];
    fn payload_meta_data(&self) -> [TxMetaData; GIRTH];
}

impl<T: SteadyStreamItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {

    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(&self) -> [TxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data().control)
            .collect::<Vec<TxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [TxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data().payload)
            .collect::<Vec<TxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }

}

//////////////////////////////////////////////
/////   run loop functions
/////////////////////////////////

pub type StreamTxBundle<'a, T> = Vec<MutexGuard<'a, StreamTx<T>>>;
pub type StreamRxBundle<'a, T> = Vec<MutexGuard<'a, StreamRx<T>>>;

/// A trait for handling transmission channel bundles.
pub trait StreamTxBundleTrait {
    /// Marks all channels in the bundle as closed.
    fn mark_closed(&mut self) -> bool;
}

pub trait StreamRxBundleTrait {
    fn is_closed_and_empty(&mut self) -> bool;
}

impl<T: SteadyStreamItem> StreamTxBundleTrait for StreamTxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }

}

impl<T: SteadyStreamItem> StreamRxBundleTrait for StreamRxBundle<'_,T> {
    fn is_closed_and_empty(&mut self) -> bool {
       self.iter_mut().all(|f| f.is_closed_and_empty())
    }
}


//////////////////////////////



/// A small wrapper type around `i32` to represent lengths in our messages.
///
/// This ensures negative lengths are never stored, and conversions from `usize`
/// are checked against `i32::MAX`.
struct Length(i32);

impl From<usize> for Length {
    /// Creates a `Length` from a `usize`, asserting it doesn’t exceed `i32::MAX`.
    fn from(value: usize) -> Self {
        assert!(
            value <= i32::MAX as usize,
            "Value exceeds i32 maximum"
        );
        Length(value as i32)
    }
}

impl From<i32> for Length {
    /// Creates a `Length` from an `i32`, asserting it's non-negative.
    fn from(value: i32) -> Self {
        assert!(value >= 0, "Value must be non-negative");
        Length(value)
    }
}


/// Specifies the type of fragment in a multi-part message.
///
/// This enum is used to track the position of each fragment in a larger message
/// that has been split into multiple parts. Such fragmentation may occur
/// when the data size exceeds certain limits (e.g., network MTU) or when you
/// wish to process data in smaller chunks for other reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FragmentType {
    /// Represents the first piece of a multi-part message.
    Begin = 1,

    /// Represents a piece that is neither the first nor the final fragment.
    Middle = 0,

    /// Represents the final piece of a multi-part message.
    End = 2,

    /// Represents a complete single-fragment message.
    UnFragmented = 3,
}

impl BitOr for FragmentType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        match (self as u8) | (rhs as u8) {
            1 => FragmentType::Begin,
            2 => FragmentType::End,
            3 => FragmentType::UnFragmented,
            _ => FragmentType::Middle, // Default to `Middle` for any other case
        }
    }
}

impl FragmentType {
    /// Converts a numeric value to a `FragmentType`, returning `None` if invalid.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(FragmentType::Middle),
            1 => Some(FragmentType::Begin),
            2 => Some(FragmentType::End),
            3 => Some(FragmentType::UnFragmented),
            _ => None,
        }
    }

    /// Returns the numeric representation of this `FragmentType`.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}


/// A trait to unify items that can be streamed.
///
/// This allows both `StreamFragment` and `StreamMessage` to share
/// functionality in testing and real code.
pub(crate) trait SteadyStreamItem {
    /// Creates a new instance with the given length, used for testing.
    fn testing_new(length: i32) -> Self;

    /// Returns the length of this item (in bytes).
    fn length(&self) -> i32;
}

/// A stream fragment, typically for incoming multi-part messages.
///
/// `StreamFragment` includes metadata about the session and arrival time.
#[derive(Clone, Copy, Debug)]
pub struct StreamFragment {
    pub(crate) length: i32,
    pub(crate) session_id: IdType,
    pub(crate) arrival: Instant,
    pub(crate) fragment_type: FragmentType,
}

impl StreamFragment {
    /// Creates a new `StreamFragment`.
    ///
    /// # Panics
    /// - Panics if `length < 0`.
    pub fn new(length: i32, session_id: i32, arrival: Instant, fragment_type: FragmentType) -> Self {
        assert!(length >= 0, "Fragment length cannot be negative");
        StreamFragment {
            length,
            session_id,
            arrival,
            fragment_type,
        }
    }
}

impl SteadyStreamItem for StreamFragment {
    fn testing_new(length: i32) -> Self {
        StreamFragment {
            length,
            session_id: 0,
            arrival: Instant::now(),
            fragment_type: FragmentType::UnFragmented,
        }
    }

    fn length(&self) -> i32 {
        self.length
    }
}

/// A simple stream message, typically for outgoing single-part messages.
#[derive(Clone, Copy, Debug)]
pub struct StreamMessage {
    pub(crate) length: i32,
}

impl StreamMessage {
    /// Creates a new `StreamMessage` from a `Length` wrapper.
    ///
    /// # Panics
    /// - Panics if `length.0 < 0` (should never happen due to `Length` checks).
    pub fn new(length: Length) -> Self {
        assert!(length.0 >= 0, "Message length cannot be negative");
        StreamMessage { length: length.0 }
    }
}

impl SteadyStreamItem for StreamMessage {
    fn testing_new(length: i32) -> Self {
        StreamMessage { length }
    }

    fn length(&self) -> i32 {
        self.length
    }
}


/// Metadata about control and payload channels for a receiver, providing
/// introspection or debugging information.
pub struct StreamRxMetaData {
    /// Metadata about the control channel.
    pub control: RxMetaData,
    /// Metadata about the payload channel.
    pub payload: RxMetaData,
}

/// Metadata about control and payload channels for a transmitter,
/// often used for diagnostics or debugging.
pub struct StreamTxMetaData {
    /// Metadata about the control channel.
    pub control: TxMetaData,
    /// Metadata about the payload channel.
    pub payload: TxMetaData,
}


/// A transmitter for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
pub(crate) struct StreamTx<T: SteadyStreamItem> {
    pub(crate) control_channel: Tx<T>,
    pub(crate) payload_channel: Tx<u8>,
    pub(crate) stream_id: i32,
}

impl<T: SteadyStreamItem> StreamTx<T> {
    /// Creates a new `StreamTx` wrapping the given channels and `stream_id`.
    pub fn new(control_channel: Tx<T>, payload_channel: Tx<u8>, stream_id: i32) -> Self {
        StreamTx {
            control_channel,
            payload_channel,
            stream_id,
        }
    }

    /// Marks both control and payload channels as closed. Returns `true` for convenience.
    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }
}

/// A receiver for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
pub struct StreamRx<T: SteadyStreamItem> {
    pub(crate) control_channel: Rx<T>,
    pub(crate) payload_channel: Rx<u8>,
    pub(crate) stream_id: i32,
}

impl<T: SteadyStreamItem> StreamRx<T> {
    /// Creates a new `StreamRx` wrapping the given channels and `stream_id`.
    pub(crate) fn new(control_channel: Rx<T>, payload_channel: Rx<u8>, stream_id: i32) -> Self {
        StreamRx {
            control_channel,
            payload_channel,
            stream_id,
        }
    }

    /// Checks if both channels are closed and empty.
    pub fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty()
            && self.payload_channel.is_closed_and_empty()
    }


}

/// Type alias for thread-safe, asynchronous references to `StreamRx` and `StreamTx`.
pub type SteadyStreamRx<T> = Arc<Mutex<StreamRx<T>>>;
pub type SteadyStreamTx<T> = Arc<Mutex<StreamTx<T>>>;


/// A lazy-initialized wrapper that stores channel builders and the resulting Tx/Rx pairs.
/// It only builds the channels once they are first needed.
#[derive(Debug)]
pub(crate) struct LazyStream<T: SteadyStreamItem> {
    /// Builder for the control channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    control_builder: Mutex<Option<ChannelBuilder>>,

    /// Builder for the payload channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    payload_builder: Mutex<Option<ChannelBuilder>>,

    /// Stores the actual channel objects (`(SteadyStreamTx, SteadyStreamRx)`) once built.
    channel: Mutex<Option<(SteadyStreamTx<T>, SteadyStreamRx<T>)>>,

    /// Stream ID for this stream.
    stream_id: i32,
}

/// A lazily-initialized transmitter wrapper for a steady stream.
#[derive(Debug)]
pub(crate) struct LazyStreamTx<T: SteadyStreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

/// A lazily-initialized receiver wrapper for a steady stream.
#[derive(Debug)]
pub(crate) struct LazyStreamRx<T: SteadyStreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

impl<T: SteadyStreamItem> LazyStream<T> {
    /// Creates a new `LazyStream` that defers channel construction until first use.
    pub(crate) fn new(
        control_builder: &ChannelBuilder,
        payload_builder: &ChannelBuilder,
        stream_id: i32,
    ) -> Self {
        assert!(stream_id >= 0, "Stream ID must zero or positive");
        LazyStream {
            control_builder: Mutex::new(Some(control_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
            stream_id,
        }
    }

    /// Retrieves (or builds) the Tx side of the channel, returning a clone of it.
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

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx, self.stream_id)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx, self.stream_id)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    /// Retrieves (or builds) the Rx side of the channel, returning a clone of it.
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

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx, self.stream_id)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx, self.stream_id)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}

impl<T: SteadyStreamItem> LazyStreamTx<T> {
    /// Creates a new `LazyStreamTx` from an existing `Arc<LazyStream<T>>`.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamTx { lazy_channel }
    }

    /// Retrieves or creates the underlying `SteadyStreamTx`, returning a clone of it.
    ///
    /// This blocks the current task if the lock is contended.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyStreamTx<T> {
        nuclei::block_on(self.lazy_channel.get_tx_clone())
    }

    /// Sends a frame (payload) and then pushes a metadata fragment (length) to the control channel.
    ///
    /// # Panics
    /// - If the entire payload can’t be sent.
    /// - If sending metadata fails.
    pub async fn testing_send_frame(&self, data: &[u8]) {
        let s = self.clone();
        let mut l = s.lock().await;

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

    /// Closes the underlying channels by marking them as closed.
    ///
    /// This signals to any receiver that no more data will arrive.
    pub async fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.lock().await;
        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }
}

impl<T: SteadyStreamItem> LazyStreamRx<T> {
    /// Creates a new `LazyStreamRx` from an existing `Arc<LazyStream<T>>`.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamRx { lazy_channel }
    }

    /// Retrieves or creates the underlying `SteadyStreamRx`, returning a clone of it.
    ///
    /// This blocks the current task if the lock is contended.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyStreamRx<T> {
        nuclei::block_on(self.lazy_channel.get_rx_clone())
    }

    /// Takes a frame of data from the payload channel, after reading a fragment from the control channel.
    ///
    /// # Panics
    /// - If the received fragment length != `data.len()`.
    /// - If an error occurs while reading.
    pub async fn testing_take_frame(&self, data: &mut [u8]) -> usize {
        let s = self.clone();
        let mut l = s.lock().await;

        if let Some(c) = l.control_channel.shared_take_async().await {
            assert_eq!(c.length() as usize, data.len());
            let count = l.payload_channel.shared_take_slice(data);
            assert_eq!(count, c.length() as usize);
            count
        } else {
            // Could log or handle gracefully instead of returning 0.
            0
        }
    }

    pub async fn testing_avail_wait(&self, count: usize) -> bool {
        let s = self.clone();
        let mut l = s.lock().await;

        l.control_channel.wait_avail_units(count).await

    }
}




/// A trait defining how to retrieve metadata from a receiver channel.
pub trait StreamRxDef {
    /// Retrieves metadata from the underlying channels, locking if necessary.
    fn meta_data(&self) -> StreamRxMetaData;
}

impl<T: SteadyStreamItem> StreamRxDef for SteadyStreamRx<T> {
    fn meta_data(&self) -> StreamRxMetaData {
        match self.try_lock() {
            Some(locked) => {
                let m1 = RxMetaData(locked.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamRxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
            None => {
                let locked = nuclei::block_on(self.lock());
                let m1 = RxMetaData(locked.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamRxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
        }
    }
}

/// A trait defining how to retrieve metadata from a transmitter channel.
pub trait StreamTxDef {
    /// Retrieves metadata from the underlying channels, locking if necessary.
    fn meta_data(&self) -> StreamTxMetaData;
}

impl<T: SteadyStreamItem> StreamTxDef for SteadyStreamTx<T> {
    fn meta_data(&self) -> StreamTxMetaData {
        match self.try_lock() {
            Some(locked) => {
                let m1 = TxMetaData(locked.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamTxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
            None => {
                let locked = nuclei::block_on(self.lock());
                let m1 = TxMetaData(locked.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamTxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
        }
    }
}
