
//! Steady stream module for managing lazy-initialized Tx and Rx channels.
//! We use one stream per channel. We can have N streams as a const array
//! going into `aeron_publish` and N streams as a const array coming from
//! `aeron_subscribe`.

use std::sync::Arc;
use std::time::Instant;
use ahash::AHashMap;
use async_ringbuf::AsyncRb;
use async_ringbuf::wrap::AsyncWrap;
use futures_util::AsyncWriteExt;
use futures_util::future::FusedFuture;
use futures_util::lock::{Mutex, MutexGuard, MutexLockFuture};
use ringbuf::consumer::Consumer;
use ringbuf::producer::Producer;
use ringbuf::storage::Heap;
use ringbuf::traits::{Observer, Split};
use crate::{monitor::{RxMetaData, TxMetaData}, channel_builder::ChannelBuilder, Rx, Tx, SteadyCommander};

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
    fn control_meta_data(&self) -> [RxMetaData; GIRTH];
    fn payload_meta_data(&self) -> [RxMetaData; GIRTH];


}

impl<T: StreamItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {

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
    fn control_meta_data(&self) -> [TxMetaData; GIRTH];
    fn payload_meta_data(&self) -> [TxMetaData; GIRTH];
}

impl<T: StreamItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {

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

pub(crate) trait StreamItem: Send + Sync {
    /// Creates a new instance with the given length, used for testing.
    fn testing_new(length: i32) -> Self;

    /// Returns the length of this item (in bytes).
    fn length(&self) -> i32;

}

/// A stream fragment, typically for incoming multi-part messages.
///
/// `StreamFragment` includes metadata about the session and arrival time.
#[derive(Clone, Copy, Debug)]
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

}

/// A simple stream message, typically for outgoing single-part messages.
#[derive(Clone, Copy, Debug)]
pub struct StreamSimpleMessage {
    pub length: i32,
}

impl StreamSimpleMessage {
    /// Creates a new `StreamMessage` from a `Length` wrapper.
    ///
    /// # Panics
    /// - Panics if `length.0 < 0` (should never happen due to `Length` checks).
    pub fn new(length: i32) -> Self {
        assert!(length >= 0, "Message length cannot be negative");
        StreamSimpleMessage { length: length }
    }
}

impl StreamItem for StreamSimpleMessage {
    fn testing_new(length: i32) -> Self {
        StreamSimpleMessage { length }
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
pub(crate) struct StreamTx<T: StreamItem> {
    pub(crate) item_channel: Tx<T>,
    pub(crate) payload_channel: Tx<u8>,
    pub(crate) stream_id: i32,
    defrag: AHashMap<i32, Defrag>,
    pub(crate) ready: Option<(i32,usize)>
}

struct Defrag {
    arrival: Option<Instant>,
    finish: Option<Instant>,
    session_id: i32,
    ringbuffer: (AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<u8>>>,false, true>)
}

impl Defrag {

    pub fn new(session_id: i32, capacity: usize) -> Defrag {
        Defrag {
            arrival: None,
            finish: None,
            session_id,
            ringbuffer: AsyncRb::<Heap<u8>>::new(capacity).split()
        }
    }
}


impl<T: StreamItem> StreamTx<T> {
    /// Creates a new `StreamTx` wrapping the given channels and `stream_id`.
    pub fn new(item_channel: Tx<T>, payload_channel: Tx<u8>, stream_id: i32) -> Self {
        StreamTx {
            item_channel,
            payload_channel,
            stream_id,
            defrag: Default::default(),
            ready: None
        }
    }

    /// Marks both control and payload channels as closed. Returns `true` for convenience.
    pub fn mark_closed(&mut self) -> bool {
        self.item_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }

}


impl StreamTx<StreamSessionMessage> {

    pub(crate) fn fragment_consume<C: SteadyCommander>(&mut self, cmd: &mut C, session_id:i32, slice: &[u8], is_begin: bool, is_end: bool) {

        //Get or create the Defrag entry for the session ID 
        let mut defrag_entry = self.defrag.entry(session_id).or_insert_with(|| {
            Defrag::new(session_id, self.payload_channel.capacity()) // Adjust capacity as needed
        });

        // // If this is the beginning of a fragment, assert the ringbuffer is empty
        if is_begin {
            // Ensure the ringbuffer is empty before beginning
            debug_assert!(defrag_entry.ringbuffer.0.is_empty(), "Ringbuffer must be empty at the beginning of a fragment");
            let now = Instant::now();
            defrag_entry.arrival = Some(now); // Set the arrival time to now
             if is_end {
                 defrag_entry.finish = Some(now);
            }
        } else {
            if is_end {
                defrag_entry.finish = Some(Instant::now());
            } 
        }

        // Append the slice to the ringbuffer (first half of the split)
        let slice_len = slice.len();
        debug_assert!(slice_len>0);
        let count = defrag_entry.ringbuffer.0.push_slice(slice);
        debug_assert_eq!(count, slice_len, "internal buffer should have had room");
         
        // If this is the end of a fragment send
        if is_end {
             let len = defrag_entry.ringbuffer.1.occupied_len();
             debug_assert!(len>0);
         
             if self.payload_channel.shared_vacant_units() >= len 
             && self.item_channel.shared_vacant_units() >= 1 {
                 let (a,b) = defrag_entry.ringbuffer.1.as_slices();
                 debug_assert_eq!(len, a.len()+b.len());
                 let item = Self::build_item(&defrag_entry,len as i32);

                  cmd.try_stream_session_message_send( &mut self.item_channel
                                                      , &mut self.payload_channel
                                                      , item
                                                      , a, b);
                
                 unsafe {
                     defrag_entry.ringbuffer.1.advance_read_index(len);
                 }
                 debug_assert!(defrag_entry.ringbuffer.0.is_empty(), "should be empty now");
                 
                 self.ready.take(); //clear ready so we can consume more.
                 defrag_entry.arrival = None;
                 defrag_entry.finish = None;
             } else {
                 self.ready = Some((session_id,len));         
             }
         }
    }


    pub(crate) fn fragment_flush<C: SteadyCommander>(&mut self, cmd: &mut C) {

        if let Some((session_id,len)) = self.ready {
            if self.payload_channel.shared_vacant_units() >= len
                && self.item_channel.shared_vacant_units() >= 1 {
                    if let Some(defrag_entry) = self.defrag.get_mut(&session_id) {
                        let (a, b) = defrag_entry.ringbuffer.1.as_slices();
                                               
                        cmd.try_stream_session_message_send( &mut self.item_channel
                                                             , &mut self.payload_channel
                                                             , Self::build_item(defrag_entry,len as i32)
                                                             , a, b);
                        
                        unsafe {
                            defrag_entry.ringbuffer.1.advance_read_index(len)
                         }
                   }
               }
           }
        }

    fn build_item(defrag: &Defrag, length: i32) -> StreamSessionMessage {
        debug_assert!(length>0, "we should never see zero payload");
        StreamSessionMessage {
            length,
            session_id: defrag.session_id,
            arrival: defrag.arrival.expect("arrival"),
            finished: defrag.finish.expect("finished"),
        }
    }
}

impl StreamTx<StreamSimpleMessage> {

    pub(crate) fn fragment_consume<C: SteadyCommander>(&mut self, cmd: &mut C, session_id:i32, slice: &[u8], is_begin: bool, is_end: bool) -> bool {

        // Get or create the Defrag entry for the session ID
        let mut defrag_entry = self.defrag.entry(session_id).or_insert_with(|| {
            Defrag::new(session_id, self.payload_channel.capacity()) // Adjust capacity as needed
        });

        // If this is the beginning of a fragment, assert the ringbuffer is empty
        if is_begin {
            // Ensure the ringbuffer is empty before beginning
            debug_assert!(defrag_entry.ringbuffer.0.is_empty(), "Ringbuffer must be empty at the beginning of a fragment");
            defrag_entry.arrival = Some(Instant::now()); // Set the arrival time to now
        }

        // Append the slice to the ringbuffer (first half of the split)
        let count = defrag_entry.ringbuffer.0.push_slice(slice);
        debug_assert_eq!(count, slice.len(), "internal buffer should have had room");

        // If this is the end of a fragment, set the finish time
        if is_end {
            defrag_entry.finish = Some(Instant::now());
            let len = defrag_entry.ringbuffer.1.occupied_len();
            debug_assert!(len>0);
            self.ready = Some((session_id,len));

            if self.payload_channel.shared_vacant_units() >= len
                && self.item_channel.shared_vacant_units() >= 1 {
                    let (a,b) = defrag_entry.ringbuffer.1.as_slices();
                    debug_assert_eq!(len, a.len()+b.len());
                
                    let item = Self::build_item(len as i32);

                      cmd.try_stream_simple_message_send( &mut self.item_channel
                                                     , &mut self.payload_channel
                                                     , item
                                                     , a, b);
                    unsafe {
                        defrag_entry.ringbuffer.1.advance_read_index(len)
                    }
                false
            } else {
                true // when true, we could not send so we must try again.
            }
        } else {
            false
        }
    }

    pub(crate) fn fragment_flush<C: SteadyCommander>(&mut self, cmd: &mut C) {

        if let Some((session_id,len)) = self.ready {
            if self.payload_channel.shared_vacant_units() >= len
                && self.item_channel.shared_vacant_units() >= 1 {
                if let Some(defrag_entry) = self.defrag.get_mut(&session_id) {
                    let (a, b) = defrag_entry.ringbuffer.1.as_slices();
                    debug_assert_eq!(len, a.len()+b.len());
                    let item = Self::build_item(len as i32);

                    cmd.try_stream_simple_message_send( &mut self.item_channel
                                                        , &mut self.payload_channel
                                                        , item
                                                        , a, b);
                    unsafe {
                        defrag_entry.ringbuffer.1.advance_read_index(len)
                    }
                }
            }
        }
    }

    fn build_item(length: i32) -> StreamSimpleMessage {
        debug_assert!(length>0, "we should never see zero payload");
        StreamSimpleMessage {
            length
        }
    }
}


pub struct StreamData<T: StreamItem> {
    pub item: T,
    pub payload: Box<[u8]>,
}

impl <T: StreamItem>StreamData<T> {
    pub fn new(item: T, payload: Box<[u8]>) -> Self {
      StreamData {item, payload}
    }
}


/// A receiver for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
pub struct StreamRx<T: StreamItem> {
    pub(crate) item_channel: Rx<T>,
    pub(crate) payload_channel: Rx<u8>,
    pub(crate) stream_id: i32
}



impl<T: StreamItem> StreamRx<T> {
    /// Creates a new `StreamRx` wrapping the given channels and `stream_id`.
    pub(crate) fn new(item_channel: Rx<T>, payload_channel: Rx<u8>, stream_id: i32) -> Self {
        StreamRx {
            item_channel,
            payload_channel,
            stream_id
        }
    }

    pub async fn shared_wait_closed_or_avail_messages(&mut self, full_messages: usize) -> bool {
        self.item_channel.shared_wait_closed_or_avail_units(full_messages).await
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

    /// Stream ID for this stream.
    stream_id: i32,
}

/// A lazily-initialized transmitter wrapper for a steady stream.
#[derive(Debug)]
pub(crate) struct LazyStreamTx<T: StreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

/// A lazily-initialized receiver wrapper for a steady stream.
#[derive(Debug)]
pub(crate) struct LazyStreamRx<T: StreamItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

impl<T: StreamItem> LazyStream<T> {
    /// Creates a new `LazyStream` that defers channel construction until first use.
    pub(crate) fn new(
        item_builder: &ChannelBuilder,
        payload_builder: &ChannelBuilder,
        stream_id: i32,
    ) -> Self {
        assert!(stream_id >= 0, "Stream ID must zero or positive");
        LazyStream {
            item_builder: Mutex::new(Some(item_builder.clone())),
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

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx, self.stream_id)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx, self.stream_id)));
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
    pub async fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.lock().await;
        l.payload_channel.mark_closed();
        l.item_channel.mark_closed();
    }
}

impl<T: StreamItem> LazyStreamRx<T> {
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

        if let Some(c) = l.item_channel.shared_take_async().await {
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

        l.item_channel.wait_avail_units(count).await

    }
}




/// A trait defining how to retrieve metadata from a receiver channel.
pub trait StreamRxDef {
    /// Retrieves metadata from the underlying channels, locking if necessary.
    fn meta_data(&self) -> StreamRxMetaData;
}

impl<T: StreamItem> StreamRxDef for SteadyStreamRx<T> {
    fn meta_data(&self) -> StreamRxMetaData {
        match self.try_lock() {
            Some(locked) => {
                let m1 = RxMetaData(locked.item_channel.channel_meta_data.clone());
                let d2 = RxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamRxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
            None => {
                let locked = nuclei::block_on(self.lock());
                let m1 = RxMetaData(locked.item_channel.channel_meta_data.clone());
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

impl<T: StreamItem> StreamTxDef for SteadyStreamTx<T> {
    fn meta_data(&self) -> StreamTxMetaData {
        match self.try_lock() {
            Some(locked) => {
                let m1 = TxMetaData(locked.item_channel.channel_meta_data.clone());
                let d2 = TxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamTxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
            None => {
                let locked = nuclei::block_on(self.lock());
                let m1 = TxMetaData(locked.item_channel.channel_meta_data.clone());
                let d2 = TxMetaData(locked.payload_channel.channel_meta_data.clone());
                StreamTxMetaData {
                    control: m1,
                    payload: d2,
                }
            }
        }
    }
}
