
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
use num_traits::Zero;
// For pin_mut!


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
pub trait LazySteadyStreamTxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamTxBundle<T, GIRTH>;
}
pub trait LazySteadyStreamRxBundleClone<T: StreamControlItem, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyStreamRxBundle<T, GIRTH>;
}


impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamTxBundleClone<T, GIRTH> for LazySteadyStreamTxBundle<T, GIRTH> {
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
impl<T: StreamControlItem, const GIRTH: usize> LazySteadyStreamRxBundleClone<T, GIRTH> for LazySteadyStreamRxBundle<T, GIRTH> {
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


pub trait SteadyStreamRxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
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

impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamRxBundleTrait<T, GIRTH> for SteadyStreamRxBundle<T, GIRTH> {

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

pub trait SteadyStreamTxBundleTrait<T: StreamControlItem, const GIRTH: usize> {
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

impl<T: StreamControlItem, const GIRTH: usize> SteadyStreamTxBundleTrait<T, GIRTH> for SteadyStreamTxBundle<T, GIRTH> {

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

    fn is_empty(&mut self) -> bool;

}

impl<T: StreamControlItem> StreamTxBundleTrait for StreamTxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        if self.is_empty() {
            trace!("bundle has no streams, nothing found to be closed");
            return true; //true we did close nothing
        }
        //NOTE: must be all or nothing it never returns early
        self.iter_mut().for_each(|f| {let _ = f.mark_closed();});
        true  // always returns true, close request is never rejected by this method.
    }


}

impl<T: StreamControlItem> StreamRxBundleTrait for StreamRxBundle<'_,T> {
    fn is_closed_and_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed_and_empty()
        )
    }
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed())
    }
    fn is_empty(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_empty()
        )
    }

}


//////////////////////////////


/// A trait to unify items that can be streamed.
///
/// These controllers of ingress and egress never have any refs and are copy
pub trait StreamControlItem: Copy + Send + Sync + 'static {
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
pub struct StreamIngress {
    pub length: i32,
    pub session_id: IdType,
    pub arrival: Instant,
    pub finished: Instant,
}
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

impl StreamIngress {
    /// Creates a new `StreamFragment`.
    ///
    /// # Panics
    /// - Panics if `length < 0`.
    pub fn new(length: i32, session_id: i32, arrival: Instant, finished: Instant) -> Self {
        assert!(length >= 0, "Fragment length cannot be negative");
        StreamIngress {
            length,
            session_id,
            arrival,
            finished,
        }
    }

    pub fn by_box(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, Box<[u8]>)
    {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0.into())
    }
    pub fn by_ref(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8])
    {
        (StreamIngress::new(p0.len() as i32, session_id, arrival, finished), p0)
    }
    pub fn build(session_id: i32, arrival: Instant, finished: Instant, p0: &[u8]) -> (StreamIngress, &[u8])
    {
        StreamIngress::by_ref(session_id, arrival, finished, p0)
    }
}

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

/// A simple stream message, typically for outgoing single-part messages.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamEgress {
    pub(crate) length: i32,
}

impl StreamEgress {
    pub fn build(p0: &[u8]) -> (StreamEgress, Box<[u8]>) {
        StreamEgress::by_box(p0)
    }
    pub fn by_box(p0: &[u8]) -> (StreamEgress, Box<[u8]>)
    {
        (StreamEgress::new(p0.len() as i32), p0.into())
    }
    pub fn by_ref(p0: &[u8]) -> (StreamEgress, &[u8])
    {
        (StreamEgress::new(p0.len() as i32), p0)
    }

}

impl StreamEgress {
    /// Creates a new `StreamMessage` from a `Length` wrapper.
    ///
    /// # Panics
    /// - Panics if `length.0 < 0` (should never happen due to `Length` checks).
    pub fn new(length: i32) -> Self {
        assert!(length >= 0, "Message length cannot be negative");
        StreamEgress { length }
    }
}

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

pub const RATE_COLLECTOR_MASK:usize = 31;
pub const RATE_COLLECTOR_LEN:usize = 32;


/// A transmitter for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
pub struct StreamTx<T: StreamControlItem> {
    pub(crate) control_channel: Tx<T>,
    pub(crate) payload_channel: Tx<u8>,
    defrag: AHashMap<i32, Defrag<T>>,
    pub(crate) ready_msg_session: VecDeque<i32>,
    pub(crate) last_input_instant: Instant,
    pub(crate) last_output_instant: Instant,
   /// pub(crate) vacant_fragments: usize, //cache for polling
    pub(crate) input_rate_index: usize,
    pub(crate) input_rate_collector: [(Duration,u32,u32); RATE_COLLECTOR_LEN],
    pub(crate) max_poll_latency: Duration, //for polling systems must check at least this often
    pub(crate) output_rate_index: usize,
    pub(crate) output_rate_collector: [(Duration,u32,u32); RATE_COLLECTOR_LEN],

    pub(crate) stored_vacant_values: (i32,i32)

}
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


pub struct Defrag<T: StreamControlItem> {
    pub(crate) arrival: Option<Instant>,
    pub(crate) finish: Option<Instant>,
    pub(crate) session_id: i32,
    pub(crate) running_length: usize,
    pub(crate) ringbuffer_items: (AsyncWrap<Arc<AsyncRb<Heap<T>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<T>>>,false, true>),
    pub(crate) ringbuffer_bytes: (AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>, AsyncWrap<Arc<AsyncRb<Heap<u8>>>,false, true>)
}

impl <T: StreamControlItem> Defrag<T> {

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

impl<T: StreamControlItem> StreamTx<T> {
    /// Creates a new `StreamTx` wrapping the given channels and `stream_id`.
    pub fn new(control_channel: Tx<T>, payload_channel: Tx<u8>) -> Self {
        StreamTx {
            max_poll_latency: Duration::from_millis(1000), //TODO: expose?
         //   vacant_fragments: item_channel.capacity() as usize,
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
        }
    }

    pub fn set_stored_vacant_values(&mut self, messages: i32, total_bytes_for_messages: i32) {
       self.stored_vacant_values  = (messages,total_bytes_for_messages);
    }

    pub fn get_stored_vacant_values(&mut self) -> (i32, i32) {
        self.stored_vacant_values
    }

    pub fn store_input_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.input_rate_index += 1;
        self.input_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration,messages,total_bytes_for_messages);
    }

    pub fn store_output_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.output_rate_index += 1;
        self.output_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration,messages,total_bytes_for_messages);
    }

    /// compute the fastest we expect this to possibly be done
    pub fn next_poll_bounds(&self) -> (Duration,Duration) {
        if let Some(d) = self.fastest_byte_processing_duration() {
            let waiting_bytes= self.payload_channel.capacity()-self.payload_channel.shared_vacant_units();
            if waiting_bytes<2 {
                (Duration::ZERO,self.max_poll_latency.min(d))
            } else {
                (  self.max_poll_latency.min(d.mul((waiting_bytes >> 1) as u32))
                 , self.max_poll_latency.min(d.mul((waiting_bytes - 1) as u32)))
            }
        } else {
            (Duration::ZERO, self.max_poll_latency)
        }
    }

    pub fn fastest_byte_processing_duration(&self) -> Option<Duration> {
        // Iterate over output_rate_collector to find the highest rate (bytes per second)
        let max_rate = self.output_rate_collector.iter()
            .filter_map(|&(duration, _, bytes)| {
                let duration_secs = duration.as_secs_f64();
                if duration_secs > 0.0 && bytes > 0 {
                    Some(bytes as f64 / duration_secs) // Bytes per second
                } else {
                    None
                }
            })
            .fold(None, |acc: Option<f64>, rate| {
                match acc {
                    None => Some(rate),
                    Some(max) => Some(max.max(rate)),
                }
            });

        // Convert max rate to duration per byte (seconds per byte)
        max_rate.map(|rate| {
            let seconds_per_byte = 1.0 / rate; // Seconds per byte
            Duration::from_secs_f64(seconds_per_byte)
        })
    }

    pub fn guess_duration_between_arrivals(&self) -> (Duration, Duration) {
        let mut sum: f64 = 0.0;      // Sum of average times (in seconds)
        let mut sum_sq: f64 = 0.0;   // Sum of squared average times
        let mut count: usize = 0;    // Number of entries with m > 0

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

    /// Marks both control and payload channels as closed. Returns `true` for convenience.
    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }

    pub fn capacity(&self) -> usize {
        self.control_channel.capacity()
    }

    //call this when we have no data in from poll to clear out anything waiting.
    // pub(crate) fn fragment_flush_all<C: SteadyCommander>(&mut self, actor: &mut C) {
    //     self.ready_msg_session.clear();
    //     for de in self.defrag.values_mut().filter(|f| !f.ringbuffer_items.0.is_empty()) {
    //                  if let (msgs,bytes,Some(more_session_id)) = actor.flush_defrag_messages( &mut self.item_channel
    //                                                             , &mut self.payload_channel
    //                                                             , de) {
    //                      self.ready_msg_session.push_back(more_session_id);
    //                  }
    //     }
    // }

    pub(crate) fn fragment_flush_ready<C: SteadyActor>(&mut self, actor: &mut C) -> (u32, u32) {
        let mut total_messages = 0;
        let mut total_bytes = 0;
        let mut to_consume = self.ready_msg_session.len();
        while let Some(session_id) = self.ready_msg_session.pop_front() { // Changed to pop_front directly
            to_consume -= 1;
            if let Some(defrag_entry) = self.defrag.get_mut(&session_id) {
                //how do we know how much we wrote??
                if let (msgs,bytes,Some(needs_more_work_for_session_id)) = actor.flush_defrag_messages(&mut self.control_channel,
                                                                                                       &mut self.payload_channel,
                                                                                                       defrag_entry) {
                    total_messages += msgs;
                    total_bytes += bytes;
                    self.ready_msg_session.push_back(needs_more_work_for_session_id);
                }
            } else {
                error!("internal error, session reported without any defrag");
            }
            if to_consume.is_zero() {
                break;
            }
        }
        (total_messages,total_bytes)
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
            Defrag::new(session_id, self.control_channel.capacity(), self.payload_channel.capacity()) // Adjust capacity as needed
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
            
            if!self.ready_msg_session.contains(&defrag_entry.session_id) {
                self.ready_msg_session.push_back(defrag_entry.session_id);
            };

            defrag_entry.running_length = 0;
            defrag_entry.arrival = None;
            defrag_entry.finish = None;
        }
    }
}


/// A receiver for a steady stream. Holds two channels:
/// one for control (`control_channel`) and one for payload (`payload_channel`).
#[derive(Debug)]
pub struct StreamRx<T: StreamControlItem> {
    pub(crate) control_channel: Rx<T>,
    pub(crate) payload_channel: Rx<u8>
}

impl<T: StreamControlItem> StreamRx<T> {
    /// Creates a new `StreamRx` wrapping the given channels and `stream_id`.
    pub(crate) fn new(control_channel: Rx<T>, payload_channel: Rx<u8>) -> Self {
        StreamRx {
            control_channel,
            payload_channel
        }
    }


    pub fn try_take(&mut self) -> Option<(T,Box<[u8]>)> {
        if let Some((_done, msg)) = self.shared_try_take() {
            Some(msg)
        } else {
            None
        }
    }

    // pub async fn shared_wait_closed_or_avail_messages(&mut self, full_messages: usize) -> bool {
    //     self.item_channel.shared_wait_closed_or_avail_units(full_messages).await
    // }
    pub fn capacity(&mut self) -> usize {
        self.control_channel.capacity()
    }

    pub fn avail_units(&mut self) -> usize {
        self.shared_avail_units()
    }

    /// Checks if both channels are closed and empty.
    // pub fn is_closed_and_empty(&mut self) -> bool {
    //     //debug!("closed_empty {} {}", self.item_channel.is_closed_and_empty(), self.payload_channel.is_closed_and_empty());
    //     self.item_channel.is_closed_and_empty()
    //         && self.payload_channel.is_closed_and_empty()
    // }
    /// Checks if both channels are closed and empty.
    pub fn is_closed(&mut self) -> bool {
        //debug!("closed {} {}", self.item_channel.is_closed(), self.payload_channel.is_closed());
        self.control_channel.is_closed() && self.payload_channel.is_closed()
    }

    pub fn is_empty(&mut self) -> bool {
        self.control_channel.is_empty()
            && self.payload_channel.is_empty()
    }

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
                let x = actor.advance_read_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
                let x = actor.advance_read_index(&mut self.control_channel, active_items);
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
                let x = actor.advance_read_index(&mut self.payload_channel, active_data);
                debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
                let x = actor.advance_read_index(&mut self.control_channel, active_items);
                debug_assert_eq!(x.item_count(), active_items, "Item channel advance mismatch");
                return;
            }

            // Update the counts of processed items and data
            active_items += 1;
            active_data += i.length() as usize;
        }

        // !("we made it to the end with {}",active_items);
        // If all items are processed successfully, advance the read indices
        let x = actor.advance_read_index(&mut self.payload_channel, active_data);
        debug_assert_eq!(x.item_count(), active_data, "Payload channel advance mismatch");
        let x = actor.advance_read_index(&mut self.control_channel, active_items);
        debug_assert_eq!(x.item_count(), active_items, "Item channel advance mismatch");

        //let avail = self.item_channel.shared_avail_units();
       //warn!("published {:?} on {:?} avail {:?}",active_items, self.item_channel.channel_meta_data.meta_data.id, avail);

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
pub(crate) struct LazyStream<T: StreamControlItem> {

    /// Builder for the control channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    control_builder: Mutex<Option<ChannelBuilder>>,

    /// Builder for the payload channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    payload_builder: Mutex<Option<ChannelBuilder>>,

    /// Stores the actual channel objects (`(SteadyStreamTx, SteadyStreamRx)`) once built.
    channel: Mutex<Option<(SteadyStreamTx<T>, SteadyStreamRx<T>)>>,

}

/// A lazily-initialized transmitter wrapper for a steady stream.
#[derive(Debug)]
pub struct LazyStreamTx<T: StreamControlItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

/// A lazily-initialized receiver wrapper for a steady stream.
#[derive(Debug)]
pub struct LazyStreamRx<T: StreamControlItem> {
    lazy_channel: Arc<LazyStream<T>>,
}

impl<T: StreamControlItem> LazyStream<T> {




    /// Creates a new `LazyStream` that defers channel construction until first use.
    pub(crate) fn new(
        item_builder: &ChannelBuilder,
        payload_builder: &ChannelBuilder,
    ) -> Self {
        LazyStream {
            control_builder: Mutex::new(Some(item_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
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

            //TODO: create shared state object  Arc<Mutex<StreamState>>
            //              holds what we are monitoring on both ends for agrreement.
            //              upon wait/usage/into_spotlight we store pub and sub actor idents for query later
            //              ensures only same ident can relock!!
            // also add to Tx and Rx construction to fix the payload_metadata match issue.

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

            //TODO: create shared state object  Arc<Mutex<StreamState>>
            //              holds what we are monitoring on both ends for agrreement.
            //              upon wait/usage we store pub and sub actor idents for query later
            //              ensures only same ident can relock!!

            let tx = Arc::new(Mutex::new(StreamTx::new(meta_tx, data_tx)));
            let rx = Arc::new(Mutex::new(StreamRx::new(meta_rx, data_rx)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}

impl<T: StreamControlItem> LazyStreamTx<T> {
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

    pub fn testing_send_all(&self, data: Vec<(T,&[u8])>, close: bool) {

        let s = self.clone();
        let mut l = s.try_lock().expect("internal error: try_lock");

        //TODO: we might want to wait for room.
        for d in data.into_iter() {
            let x = l.payload_channel.shared_send_slice_until_full(&d.1);
            match l.control_channel.shared_try_send(T::testing_new(x as i32)) {
                Ok(_) => {}
                Err(_) => {
                    panic!("error sending metadata");
                }
            };
            assert_eq!(x,d.1.len());
        }
        if close {
            l.mark_closed(); // for clean shutdown we tell the actor we have no more data
        }
    }

    /// Closes the underlying channels by marking them as closed.
    ///
    /// This signals to any receiver that no more data will arrive.
    pub fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.try_lock().expect("internal error: try_lock");

        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }
}

impl<T: StreamControlItem> LazyStreamRx<T> {

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
    // pub async fn testing_take_frame(&self, data: &mut [u8]) -> usize {
    //     let s = self.clone();
    //     let mut l = s.lock().await;
    //
    //     if let Some(_c) = l.item_channel.shared_take_async().await {
    //         let count = l.payload_channel.shared_take_slice(data);
    //         assert_eq!(count, data.len());
    //         count
    //     } else {
    //         // Could log or handle gracefully instead of returning 0.
    //         0
    //     }
    // }



    pub fn testing_avail_wait(&self, count: usize, timeout_duration: Duration) -> bool {
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

//TODO: these two methods control streams showing items not bytes, this may need to be corrected.

impl<T: StreamControlItem> RxMetaDataProvider for SteadyStreamRx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => guard.control_channel.channel_meta_data.meta_data(),
            None => {
                let guard = core_exec::block_on(self.lock());
                guard.control_channel.channel_meta_data.meta_data()
            }
        }
    }
}

impl<T: StreamControlItem> TxMetaDataProvider for SteadyStreamTx<T> {
    fn meta_data(&self) -> Arc<ChannelMetaData> {
        match self.try_lock() {
            Some(guard) => guard.control_channel.channel_meta_data.meta_data(),
            None => {
                let guard = core_exec::block_on(self.lock());
                guard.control_channel.channel_meta_data.meta_data()
            }
        }
    }
}

#[cfg(test)]
mod extra_stream_tests {
    use super::*;
    use std::time::{Duration, Instant};
    use std::sync::Arc;
    /// Test extract_stream_payload_slices covers both on_first=true and false branches
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

    /// Test Defrag::new initializes fields correctly and buffers have correct capacity
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




    /// Test testing_new and length for StreamSessionMessage
    #[test]
    fn test_testing_new_methods() {
        let tn = StreamIngress::testing_new(9);
        assert_eq!(tn.length(), 9);
    }

    /// Test RATE_COLLECTOR constants relationship
    #[test]
    fn test_rate_collector_constants() {
        assert_eq!(RATE_COLLECTOR_LEN, 32);
        assert_eq!(RATE_COLLECTOR_MASK, 31);
        // Masking LEN yields zero
        assert_eq!(RATE_COLLECTOR_LEN & RATE_COLLECTOR_MASK, 0);
    }

    /// Test StreamTxBundleTrait.mark_closed on empty and non-empty bundles
    #[test]
    fn test_stream_tx_bundle_trait_empty() {
        type Bundle = StreamTxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.mark_closed(), "even Empty bundle should return true");
    }

    /// Test StreamRxBundleTrait on empty bundles
    #[test]
    fn test_stream_rx_bundle_trait_empty() {
        type Bundle = StreamRxBundle<'static, StreamEgress>;
        let mut bundle: Bundle = Vec::new();
        assert!(bundle.is_closed_and_empty());
        assert!(bundle.is_closed());
        assert!(bundle.is_empty());
    }


    /// Even a zero‐length LazySteadyStreamTxBundle should clone cleanly to an empty SteadyStreamTxBundle.
    #[test]
    fn test_lazy_tx_bundle_clone_empty() {
        let empty: [LazyStreamTx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamTxBundle<StreamEgress, 0> = empty.clone();
        // An Arc<[..;0]> has length 0
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// Even a zero‐length LazySteadyStreamRxBundle should clone cleanly to an empty SteadyStreamRxBundle.
    #[test]
    fn test_lazy_rx_bundle_clone_empty() {
        let empty: [LazyStreamRx<StreamEgress>; 0] = [];
        let cloned: SteadyStreamRxBundle<StreamEgress, 0> = empty.clone();
        assert_eq!(Arc::as_ref(&cloned).len(), 0);
    }

    /// SteadyStreamRxBundleTrait on empty should produce an immediate empty lock and empty metadata arrays.
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

    /// SteadyStreamTxBundleTrait on empty should produce an immediate empty lock and empty metadata arrays.
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
}
