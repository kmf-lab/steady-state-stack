
//! Steady stream module for managing lazy-initialized Tx and Rx channels.
//! We use one stream per channel. We can have N streams as a const array
//! going into `aeron_publish` and N streams as a const array coming from
//! `aeron_subscribe`.

use crate::core_tx::TxCore;
use crate::{channel_builder::ChannelBuilder, Rx, SteadyCommander, Tx};
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
use std::sync::Arc;
use std::time::Instant;
use crate::core_rx::RxCore;
use crate::monitor::ChannelMetaData;
use crate::steady_rx::RxMetaDataProvider;
use crate::steady_tx::TxMetaDataProvider;
use crate::core_exec;

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
pub trait LazySteadyStreamTxBundleClone<T: StreamItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH>;
}
pub trait LazySteadyStreamRxBundleClone<T: StreamItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH>;
}


impl<T: StreamItem, const GIRTH: usize> LazySteadyStreamTxBundleClone<T, GIRTH> for LazySteadyStreamTxBundle<T, GIRTH> {
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
impl<T: StreamItem, const GIRTH: usize> LazySteadyStreamRxBundleClone<T, GIRTH> for LazySteadyStreamRxBundle<T, GIRTH> {
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


pub trait SteadyStreamRxBundleTrait<T: StreamItem, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn control_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];
    fn payload_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH];

}

impl<T: StreamItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {

    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamRx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(& self) -> [& dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn RxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [&dyn RxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn RxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

}

pub trait SteadyStreamTxBundleTrait<T: StreamItem, const GIRTH: usize> {
    /// Locks all receivers in the bundle.
    ///
    /// # Returns
    /// A `JoinAll` future that resolves when all receivers are locked.
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>>;

    /// Retrieves metadata for all receivers in the bundle.
    ///
    /// # Returns
    /// An array of `RxMetaData` objects containing metadata for each receiver.
    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];
    fn payload_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH];


}

impl<T: StreamItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {

    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, StreamTx<T>>> {
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn control_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
            .try_into()
            .expect("Internal Error")
    }

    fn payload_meta_data(&self) -> [&dyn TxMetaDataProvider; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxMetaDataProvider)
            .collect::<Vec<_>>()
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
    fn is_closed(&mut self) -> bool;

}

impl<T: StreamItem> StreamTxBundleTrait for StreamTxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }


}

impl<T: StreamItem> StreamRxBundleTrait for StreamRxBundle<'_,T> {
    fn is_closed_and_empty(&mut self) -> bool {
       self.iter_mut().all(|f| f.is_closed_and_empty())
    }
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }

}


//////////////////////////////


/// A trait to unify items that can be streamed.
///
/// This allows both `StreamFragment` and `StreamMessage` to share
/// functionality in testing and real code.
pub trait StreamItem: Copy + Send + Sync {
    /// Creates a new instance with the given length, used for testing.
    fn testing_new(length: i32) -> Self;

    /// Returns the length of this item (in bytes).
    fn length(&self) -> i32;

    // Return new stream item
    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self;
}


/// A stream fragment, typically for incoming multi-part messages.
///
/// `StreamFragment` includes metadata about the session and arrival time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamSessionMessage {
    pub length: i32,
    pub session_id: IdType,
    pub arrival: Instant,
    pub finished: Instant,

}

impl StreamSessionMessage {
    /// Creates a new `StreamFragment`.
    ///
    /// # Panics
    /// - Panics if `length < 0`.
    pub fn new(length: i32, session_id: i32, arrival: Instant, finished: Instant) -> Self {
        assert!(length >= 0, "Fragment length cannot be negative");
        StreamSessionMessage {
            length,
            session_id,
            arrival,
            finished,
        }
    }
}

impl StreamItem for StreamSessionMessage {
    fn testing_new(length: i32) -> Self {
        StreamSessionMessage {
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
        StreamSessionMessage{
            length: defrag_entry.running_length as i32,
            session_id: defrag_entry.session_id,
            arrival: defrag_entry.arrival.expect("defrag must have needed Instant"),
            finished: defrag_entry.finish.expect("defrag must have needed Instant"),
        }
    }
}

/// A simple stream message, typically for outgoing single-part messages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamSimpleMessage {
    pub(crate) length: i32,
}

impl StreamSimpleMessage {
    pub fn wrap(p0: &[u8]) -> (StreamSimpleMessage, &[u8]) {
        (StreamSimpleMessage::new(p0.len() as i32), p0)
    }
}

impl StreamSimpleMessage {
    /// Creates a new `StreamMessage` from a `Length` wrapper.
    ///
    /// # Panics
    /// - Panics if `length.0 < 0` (should never happen due to `Length` checks).
    pub fn new(length: i32) -> Self {
        assert!(length >= 0, "Message length cannot be negative");
        StreamSimpleMessage { length }
    }
}

impl StreamItem for StreamSimpleMessage {
    fn testing_new(length: i32) -> Self {
        StreamSimpleMessage { length }
    }

    fn length(&self) -> i32 {
        self.length
    }
    
    fn from_defrag(defrag_entry: &Defrag<Self>) -> Self {
        StreamSimpleMessage::new(defrag_entry.running_length as i32)
    }
}


/// Metadata about control and payload channels for a receiver, providing
/// introspection or debugging information.
pub struct StreamRxMetaData {
    /// Metadata about the control channel.
    pub control: RxChannelMetaDataWrapper,
    /// Metadata about the payload channel.
    pub payload: RxChannelMetaDataWrapper,
}

#[derive(Debug)]
pub struct RxChannelMetaDataWrapper {
    pub(crate) meta_data: Arc<ChannelMetaData>
}
impl RxMetaDataProvider for RxChannelMetaDataWrapper {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}

/// Metadata about control and payload channels for a transmitter,
/// often used for diagnostics or debugging.
pub struct StreamTxMetaData {
    /// Metadata about the control channel.
    pub(crate) control: TxChannelMetaDataWrapper,
    /// Metadata about the payload channel.
    pub(crate) payload: TxChannelMetaDataWrapper,
}

#[derive(Debug)]
pub struct TxChannelMetaDataWrapper {
    pub(crate) meta_data: Arc<ChannelMetaData>
}
impl TxMetaDataProvider for TxChannelMetaDataWrapper {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        Arc::clone(&self.meta_data)
    }
}


/// A transmitter for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
pub struct StreamTx<T: StreamItem> {
    pub(crate) item_channel: Tx<T>,
    pub(crate) payload_channel: Tx<u8>,
    defrag: AHashMap<i32, Defrag<T>>,
    pub(crate) ready: VecDeque<i32>
}
impl<T: StreamItem> Debug for StreamTx<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamTx")
            .field("item_channel", &"Tx<T>")
            .field("payload_channel", &"Tx<u8>")
            .field("defrag_keys", &self.defrag.keys().collect::<Vec<_>>())
            .field("ready", &self.ready)
            .finish()
    }
}


pub struct Defrag<T: StreamItem> {
    pub(crate) arrival: Option<Instant>,
    pub(crate) finish: Option<Instant>,
    pub(crate) session_id: i32,
    pub(crate) running_length: usize,
    pub(crate) ringbuffer_items: (AsyncWrap<Arc<AsyncRb<Heap<T>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<T>>>,false, true>),
    pub(crate) ringbuffer_bytes: (AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<u8>>>,false, true>)
}

impl <T: StreamItem> Defrag<T> {

    pub fn new(session_id: i32, items: usize, bytes: usize) -> Self {
        Defrag {
            arrival: None,
            finish: None,
            session_id,
            running_length: 0,
            ringbuffer_items: AsyncRb::<Heap<T>>::new(items).split(),
            ringbuffer_bytes: AsyncRb::<Heap<u8>>::new(bytes).split()
        }
    }
}

impl<T: StreamItem> StreamTx<T> {
    /// Creates a new `StreamTx` wrapping the given channels and `stream_id`.
    pub fn new(item_channel: Tx<T>, payload_channel: Tx<u8>) -> Self {
        StreamTx {
            item_channel,
            payload_channel,
            defrag: Default::default(),
            ready: VecDeque::with_capacity(4)
        }
    }

    /// Marks both control and payload channels as closed. Returns `true` for convenience.
    pub fn mark_closed(&mut self) -> bool {
        self.item_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }

    pub fn capacity(&mut self) -> usize {
        self.item_channel.capacity()
    }

    //call this when we have no data in from poll to clear out anything waiting.
    pub(crate) fn fragment_flush_all<C: SteadyCommander>(&mut self, cmd: &mut C) {
        self.ready.clear();
        for de in self.defrag.values_mut().filter(|f| !f.ringbuffer_items.0.is_empty()) {
                     if let Some(x) = cmd.flush_defrag_messages( &mut self.item_channel
                                                                , &mut self.payload_channel
                                                                , de) {
                         self.ready.push_back(x);
                     }
        }
    }

        pub(crate) fn fragment_flush_ready<C: SteadyCommander>(&mut self, cmd: &mut C) {
            while let Some(session_id) = self.ready.pop_front() {
            if let Some(defrag_entry) = self.defrag.get_mut(&session_id) {
                if let Some(x) = cmd.flush_defrag_messages(  &mut self.item_channel
                                                           , &mut self.payload_channel
                                                           , defrag_entry) {
                    self.ready.push_back(x);
                }
            }
        }
    }

    pub(crate) fn smallest_space(&mut self) -> Option<usize> {
         self.defrag
             .values()
             .map(|d| d.ringbuffer_items.0.vacant_len())
             .min()
    }

    #[inline]
    pub(crate) fn fragment_consume(&mut self
                                   , session_id:i32
                                   , slice: &[u8]
                                   , is_begin: bool
                                   , is_end: bool
                                   , now: Instant) {

        //Get or create the Defrag entry for the session ID
        let defrag_entry = self.defrag.entry(session_id).or_insert_with(|| {
            Defrag::new(session_id, self.item_channel.capacity(), self.payload_channel.capacity()) // Adjust capacity as needed
        });


        // // If this is the beginning of a fragment, assert the ringbuffer is empty
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
        debug_assert!(slice_len>0);

        let count = defrag_entry.ringbuffer_bytes.0.push_slice(slice);
        defrag_entry.running_length += count;
        debug_assert_eq!(count, slice_len, "internal buffer should have had room");

        // If this is the end of a fragment send
        if is_end {
            let result = defrag_entry.ringbuffer_items.0.try_push(T::from_defrag(defrag_entry));
            debug_assert!(result.is_ok());
            
            let cap: usize = defrag_entry.ringbuffer_items.0.capacity().into();
            if defrag_entry.ringbuffer_items.1.occupied_len() == cap && !self.ready.contains(&defrag_entry.session_id) {
                self.ready.push_back(defrag_entry.session_id);
            };

            defrag_entry.running_length = 0;
            defrag_entry.arrival = None;
            defrag_entry.finish = None;
        }
    }
}

// not sure we want this anymore.
//
// #[derive(Clone, Debug)]
// pub struct StreamData<T: StreamItem> {
//     pub item: T,
//     pub payload: Box<[u8]>,
// }
//
// impl <T: StreamItem>StreamData<T> {
//     pub fn new(item: T, payload: Box<[u8]>) -> Self {
//       StreamData {item, payload}
//     }
// }


/// A receiver for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
#[derive(Debug)]
pub struct StreamRx<T: StreamItem> {
    pub(crate) item_channel: Rx<T>,
    pub(crate) payload_channel: Rx<u8>
}



impl<T: StreamItem> StreamRx<T> {
    /// Creates a new `StreamRx` wrapping the given channels and `stream_id`.
    pub(crate) fn new(item_channel: Rx<T>, payload_channel: Rx<u8>) -> Self {
        StreamRx {
            item_channel,
            payload_channel
        }
    }

    // pub async fn shared_wait_closed_or_avail_messages(&mut self, full_messages: usize) -> bool {
    //     self.item_channel.shared_wait_closed_or_avail_units(full_messages).await
    // }
    pub fn capacity(&mut self) -> usize {
        self.item_channel.capacity()
    }

    /// Checks if both channels are closed and empty.
    pub fn is_closed_and_empty(&mut self) -> bool {
        //debug!("closed_empty {} {}", self.item_channel.is_closed_and_empty(), self.payload_channel.is_closed_and_empty());
        self.item_channel.is_closed_and_empty()
            && self.payload_channel.is_closed_and_empty()
    }
    /// Checks if both channels are closed and empty.
    pub fn is_closed(&mut self) -> bool {
        //debug!("closed {} {}", self.item_channel.is_closed(), self.payload_channel.is_closed());
        self.item_channel.is_closed() && self.payload_channel.is_closed()
    }
    pub(crate) fn consume_messages<C: SteadyCommander>(
        &mut self,
        cmd: &mut C,
        byte_limit: usize,
        mut fun: impl FnMut(&mut [u8], &mut [u8]) -> bool,
    ) {
        // Obtain mutable slices from the item and payload channels
        let (item1, item2) = self.item_channel.rx.as_mut_slices();
        let (payload1, payload2) = self.payload_channel.rx.as_mut_slices();

        // Variables to track the state of the iteration
        let mut on_first = true; // Whether we are still processing the first payload slice
        let mut active_index = 0; // Current index in the active payload slice
        let mut active_items = 0; // Number of items processed
        let mut active_data:usize = 0; // Total bytes processed

        // Process items from the first slice
        for i in item1 {
            // Extract payload slices based on the current state
            let (a, b) = Self::extract_stream_payload_slices(
                payload1,
                payload2,
                &mut on_first,
                &mut active_index,
                i.length() as usize,
            );

            // Apply the provided function to the payload slices
            if active_data+(i.length() as usize) > byte_limit || !fun(a, b) {
                // If the limit is reached or the function returns false, advance the read indices and exit
                let x = cmd.advance_read_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x, active_data, "Payload channel advance mismatch");
                let x = cmd.advance_read_index(&mut self.item_channel, active_items);
                debug_assert_eq!(x, active_items, "Item channel advance mismatch");
                return;
            }

            // Update the counts of processed items and data
            active_items += 1;
            active_data += i.length() as usize;
        }

        // Process items from the second slice
        for i in item2 {
            // Extract payload slices based on the current state
            let (a, b) = Self::extract_stream_payload_slices(
                payload1,
                payload2,
                &mut on_first,
                &mut active_index,
                i.length() as usize,
            );

            // Apply the provided function to the payload slices
            if active_data+(i.length() as usize) > byte_limit || !fun(a, b) {
                // If the limit is reached or the function returns false, advance the read indices and exit
                let x = cmd.advance_read_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x, active_data, "Payload channel advance mismatch");
                let x = cmd.advance_read_index(&mut self.item_channel, active_items);
                debug_assert_eq!(x, active_items, "Item channel advance mismatch");
                return;
            }

            // Update the counts of processed items and data
            active_items += 1;
            active_data += i.length() as usize;
        }

        // !("we made it to the end with {}",active_items);
        // If all items are processed successfully, advance the read indices
        let x = cmd.advance_read_index(&mut self.payload_channel, active_data);
        debug_assert_eq!(x, active_data, "Payload channel advance mismatch");
        let x = cmd.advance_read_index(&mut self.item_channel, active_items);
        debug_assert_eq!(x, active_items, "Item channel advance mismatch");
    }

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

/// Type alias for thread-safe, asynchronous references to `StreamRx` and `StreamTx`.
pub type SteadyStreamRx<T> = Arc<Mutex<StreamRx<T>>>;
pub type SteadyStreamTx<T> = Arc<Mutex<StreamTx<T>>>;


/// A lazy-initialized wrapper that stores channel builders and the resulting Tx/Rx pairs.
/// It only builds the channels once they are first needed.
#[derive(Debug)]
pub(crate) struct LazyStream<T: StreamItem> {
    /// Builder for the control channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    item_builder: Mutex<Option<ChannelBuilder>>,

    /// Builder for the payload channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    payload_builder: Mutex<Option<ChannelBuilder>>,

    /// Stores the actual channel objects (`(SteadyStreamTx, SteadyStreamRx)`) once built.
    channel: Mutex<Option<(SteadyStreamTx<T>, SteadyStreamRx<T>)>>,

}

/// A lazily-initialized transmitter wrapper for a steady stream.
#[derive(Debug)]
pub struct LazyStreamTx<T: StreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

/// A lazily-initialized receiver wrapper for a steady stream.
#[derive(Debug)]
pub struct LazyStreamRx<T: StreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

impl<T: StreamItem> LazyStream<T> {




    /// Creates a new `LazyStream` that defers channel construction until first use.
    pub(crate) fn new(
        item_builder: &ChannelBuilder,
        payload_builder: &ChannelBuilder,
    ) -> Self {
        LazyStream {
            item_builder: Mutex::new(Some(item_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
        }
    }

    /// Retrieves (or builds) the Tx side of the channel, returning a clone of it.
    pub(crate) async fn get_tx_clone(&self) -> SteadyStreamTx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self
                .item_builder
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

    /// Retrieves (or builds) the Rx side of the channel, returning a clone of it.
    pub(crate) async fn get_rx_clone(&self) -> SteadyStreamRx<T> {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self
                .item_builder
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

impl<T: StreamItem> LazyStreamTx<T> {
    /// Creates a new `LazyStreamTx` from an existing `Arc<LazyStream<T>>`.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamTx { lazy_channel }
    }

    /// Retrieves or creates the underlying `SteadyStreamTx`, returning a clone of it.
    ///
    /// This blocks the current task if the lock is contended.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyStreamTx<T> {
        core_exec::block_on(self.lazy_channel.get_tx_clone())
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

        match l.item_channel.shared_try_send(T::testing_new(x as i32)) {
            Ok(_) => {}
            Err(_) => {
                panic!("error sending metadata");
            }
        };
    }

    /// Closes the underlying channels by marking them as closed.
    ///
    /// This signals to any receiver that no more data will arrive.
    pub fn testing_close(&self) {
        core_exec::block_on( async {
            let s = self.clone();
            let mut l = s.lock().await;
            l.payload_channel.mark_closed();
            l.item_channel.mark_closed();
        });
    }
}

impl<T: StreamItem> LazyStreamRx<T> {

    /// For testing simulates taking data from the actor in a controlled manner.
    pub async fn testing_avail_units(&self) -> usize {
        let rx = self.lazy_channel.get_rx_clone().await;
        let mut rx = rx.lock().await;
        rx.shared_avail_units()
    }


    /// For testing simulates taking data from the actor in a controlled manner.
    // pub async fn testing_take(&self) -> Vec<T> {
    //     let rx = self.lazy_channel.get_rx_clone().await;
    //     let mut rx = rx.lock().await;
    //     let limit = rx.capacity();
    //
    //     rx.shared_take_into_iter().take(limit).collect()
    // }


    /// Creates a new `LazyStreamRx` from an existing `Arc<LazyStream<T>>`.
    pub(crate) fn new(lazy_channel: Arc<LazyStream<T>>) -> Self {
        LazyStreamRx { lazy_channel }
    }

    /// Retrieves or creates the underlying `SteadyStreamRx`, returning a clone of it.
    ///
    /// This blocks the current task if the lock is contended.
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyStreamRx<T> {
        core_exec::block_on(self.lazy_channel.get_rx_clone())
    }

    /// Takes a frame of data from the payload channel, after reading a fragment from the control channel.
    ///
    /// # Panics
    /// - If the received fragment length != `data.len()`.
    /// - If an error occurs while reading.
    pub async fn testing_take_frame(&self, data: &mut [u8]) -> usize {
        let s = self.clone();
        let mut l = s.lock().await;

        if let Some(_c) = l.item_channel.shared_take_async().await {
            let count = l.payload_channel.shared_take_slice(data);
            assert_eq!(count, data.len());
            count
        } else {
            // Could log or handle gracefully instead of returning 0.
            0
        }
    }

    pub async fn testing_avail_wait(&self, count: usize) -> bool {
        let s = self.clone();
        let mut l = s.lock().await;

        l.shared_wait_closed_or_avail_units(count).await

    }
}

//TODO: these two methods control streams showing items not bytes, this may need to be corrected.

impl<T: StreamItem> RxMetaDataProvider for SteadyStreamRx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => guard.item_channel.channel_meta_data.meta_data(),
            None => {
                let guard = core_exec::block_on(self.lock());
                guard.item_channel.channel_meta_data.meta_data()
            }
        }
    }
}

impl<T: StreamItem> TxMetaDataProvider for SteadyStreamTx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => guard.item_channel.channel_meta_data.meta_data(),
            None => {
                let guard = core_exec::block_on(self.lock());
                guard.item_channel.channel_meta_data.meta_data()
            }
        }
    }
}

