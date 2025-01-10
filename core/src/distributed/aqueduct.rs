use std::collections::{HashMap, HashSet};
use std::iter::Peekable;
use std::ops::BitOr;
use std::sync::Arc;
use std::time::Instant;
use futures_util::lock::Mutex;
use log::warn;
use crate::channel_builder::ChannelBuilder;
use crate::{Rx, SteadyCommander, Tx};
use crate::monitor::{RxMetaData, TxMetaData};


pub struct IncomingMessageIterator<'a> {
    session_iters: Vec<(&'a IdType, std::vec::IntoIter<IncomingMessage>)>,
}

impl<'a> IncomingMessageIterator<'a> {
    pub fn new_for_stream(
        collect: &'a mut Vec<HashMap<IdType, Vec<IncomingMessage>>>,
        stream_id: i32,
        stream_first: i32,
    ) -> Self {
        let stream_index = (stream_id - stream_first) as usize;

        let session_iters: Vec<_> = collect[stream_index]
            .iter_mut()
            .map(|(session_id, messages)| {
                (
                    session_id,
                    messages.drain(..).collect::<Vec<_>>().into_iter(),
                )
            })
            .collect();

        Self { session_iters }
    }

    pub fn new_for_all_streams(
        collect: &'a mut Vec<HashMap<IdType, Vec<IncomingMessage>>>,
    ) -> Self {
        let session_iters: Vec<_> = collect
            .iter_mut()
            .flat_map(|stream| stream.iter_mut())
            .map(|(session_id, messages)| {
                (
                    session_id,
                    messages.drain(..).collect::<Vec<_>>().into_iter(),
                )
            })
            .collect();

        Self { session_iters }
    }

    pub fn new_for_stream_subset(
        collect: &'a mut Vec<HashMap<IdType, Vec<IncomingMessage>>>,
        stream_ids: &HashSet<i32>, // Use a HashSet for efficient lookups
        stream_first: i32,
    ) -> Self {
        let session_iters: Vec<_> = collect
            .iter_mut()
            .enumerate()
            .filter_map(|(stream_index, stream)| {
                let stream_id = stream_index as i32 + stream_first;
                if stream_ids.contains(&stream_id) {
                    Some(
                        stream
                            .iter_mut()
                            .map(|(session_id, messages)| {
                                (
                                    session_id,
                                    messages.drain(..).collect::<Vec<_>>().into_iter(),
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        Self { session_iters }
    }
}

impl<'a> Iterator for IncomingMessageIterator<'a> {
    type Item = IncomingMessage;

    fn next(&mut self) -> Option<Self::Item> {
        let mut smallest_session_idx: Option<usize> = None;
        let mut smallest_message: Option<IncomingMessage> = None;

        // Iterate over session iterators
        for (i, (_, iter)) in self.session_iters.iter_mut().enumerate() {
            // Peek at the next message
            if let Some(msg) = iter.next() {
                if msg.finish.is_none() {
                    // Skip the entire iterator if the first message is incomplete
                    continue;
                }

                match &smallest_message {
                    Some(current) => {
                        if msg.arrival < current.arrival {
                            smallest_session_idx = Some(i);
                            smallest_message = Some(msg);
                        }
                    }
                    None => {
                        smallest_session_idx = Some(i);
                        smallest_message = Some(msg);
                    }
                }
            }
        }

        // Remove the iterator if it's exhausted
        if let Some(idx) = smallest_session_idx {
            let (_, iter) = &mut self.session_iters[idx];
            if iter.len() == 0 {
                self.session_iters.remove(idx);
            }
            return smallest_message;
        }

        None
    }
}

pub struct CombinedFragmentIterator<I>
where
    I: Iterator<Item = AqueductFragment>,
{
    inner: Peekable<I>,
}

impl<I> CombinedFragmentIterator<I>
where
    I: Iterator<Item = AqueductFragment>,
{
    pub fn new(iter: I) -> Self {
        Self {
            inner: iter.peekable(),
        }
    }
}

impl<I> Iterator for CombinedFragmentIterator<I>
where
    I: Iterator<Item = AqueductFragment>,
{
    type Item = AqueductFragment;

    fn next(&mut self) -> Option<Self::Item> {
        let mut current = self.inner.next()?;
        while let Some(&next) = self.inner.peek() {
            // Check if fragments can be combined
            if let (
                FragmentDirection::Incoming(current_id, _current_timestamp),
                FragmentDirection::Incoming(next_id, _),
            ) = (current.direction, next.direction)
            {
                if current.fragment_type != (FragmentType::End | current.fragment_type)
                    && next.fragment_type != (FragmentType::Begin | next.fragment_type)
                    && current.stream_id == next.stream_id
                    && current_id == next_id
                {
                    // Combine fragments
                    current.length += next.length;
                    current.fragment_type = current.fragment_type | next.fragment_type;
                    //do not change current direction it stands as is
                    self.inner.next(); // Consume the peeked item
                    continue;
                }
            }
            break;
        }
        Some(current)
    }
}

impl AqueductRx {
    pub fn new(control_channel: Rx<AqueductFragment>
               , payload_channel: Rx<u8>
               , stream_first: i32
               , stream_count: i32) -> Self {

        //must have one map per stream
        let mut collect = Vec::with_capacity(stream_count as usize);
        for _ in 0..stream_count as usize {
            collect.push(HashMap::new());
        };

        AqueductRx {
            control_channel,
            payload_channel,
            stream_first,
            stream_count,
            collect
        }
    }


    pub fn take_by_stream(&mut self, stream_id: i32) -> Vec<IncomingMessage> {        
        IncomingMessageIterator::new_for_stream(&mut self.collect
                                                , stream_id
                                                , self.stream_first).collect()   
    }

    pub fn take_all_streams(&mut self) -> Vec<IncomingMessage> {
        IncomingMessageIterator::new_for_all_streams(&mut self.collect).collect()
    }



    pub fn peek_by_stream(&self, stream_id: i32) -> Vec<&IncomingMessage> {
        let mut result = Vec::new();

        let stream_index = (stream_id - self.stream_first) as usize;
        debug_assert!(stream_index < self.stream_count as usize);

        // Iterate over all sessions in the stream and collect references to finished messages
        for (_session_id, messages) in &self.collect[stream_index] {
            for msg in messages {
                if msg.finish.is_some() {
                    result.push(msg);
                }
            }
        }
        result.sort_by_key(|msg| msg.arrival);
        result
    }

    // Sort the result by arrival time
    pub fn peek_all_streams(&self) -> Vec<&IncomingMessage> {
        let mut result = Vec::new();

        // Iterate over all streams
        for stream in &self.collect {
            // Iterate over all sessions in the stream
            for (_session_id, messages) in stream {
                // Collect references to finished messages
                for msg in messages {
                    if msg.finish.is_some() {
                        result.push(msg);
                    }
                }
            }
        }

        // Sort the result by arrival time
        result.sort_by_key(|msg| msg.arrival);
        result
    }

    //do not defrag just count
    pub fn count_by_stream(&self, stream_id: i32) -> usize {
        let stream_index = (stream_id - self.stream_first) as usize;
        debug_assert!(stream_index < self.stream_count as usize);

        // Count the number of finished messages in the stream
        self.collect[stream_index]
            .values()
            .flat_map(|messages| messages.iter())
            .filter(|msg| msg.finish.is_some())
            .count()
    }

    //do not defrag just count
    pub fn count_all_streams(&self) -> usize {
        self.collect
            .iter()
            .flat_map(|stream| stream.values()) // Iterate over all session vectors in all streams
            .flat_map(|messages| messages.iter()) // Iterate over all messages in all session vectors
            .filter(|msg| msg.finish.is_some()) // Filter for finished messages
            .count()
    }


   /// errors are sent in order, unfished wait for timeout and arrive late?
    /// must always defrag before starting an iterator we cant do it in the middle! or order is lost!!
    /// non blocking call to consume available data and move it to vecs
    /// returns count frames de-fragmented
    pub fn defragment<T: SteadyCommander>(&mut self, cmd: &mut T) -> usize {
        let mut count = 0;

        //iterate each fragment we get from aeron
        let fragment_iterator = cmd.take_into_iter(&mut self.control_channel);
        //combine neighboring fragments if we get lucky and they share stream & session
        let combined_iter = CombinedFragmentIterator::new(fragment_iterator);
        //take each incoming fragment and move the payload in to vecs for consumer iterator later
        combined_iter.for_each(|frame| {
                    if let FragmentDirection::Incoming(session_id, arrival)  = frame.direction {

                        let stream_index = (frame.stream_id-self.stream_first) as usize;
                        debug_assert!(stream_index < self.stream_count as usize);

                        match frame.fragment_type {
                            FragmentType::UnFragmented | FragmentType::Begin => {
                                debug_assert!(frame.length<= self.payload_channel.capacity() as i32);
                                let mut consumer_vec = vec![0u8; frame.length as usize];
                                let count = cmd.take_slice(&mut self.payload_channel, &mut consumer_vec);
                                debug_assert_eq!(count, frame.length as usize);
                                let session_vec = &mut self.collect[stream_index]
                                                           .entry(session_id).or_default();

                                session_vec.push(IncomingMessage {
                                    stream_id: frame.stream_id,
                                    arrival,
                                    finish: if frame.fragment_type == FragmentType::UnFragmented { Some(Instant::now()) } else { None },
                                    data: Some(consumer_vec),
                                    session_id: 0
                                });
                            },
                            FragmentType::Middle | FragmentType::End => {
                                let session_vec = &mut self.collect[stream_index]
                                    .entry(session_id).or_insert(Vec::new());
                                let collector = session_vec.last_mut().expect("vec");
                                debug_assert!(!collector.finish.is_some());

                                if let Some(ref mut data) = collector.data {
                                    let start = data.len();
                                    let desired = start + frame.length as usize;
                                    debug_assert!(desired < i32::MAX as usize);
                                    data.resize(desired, 0);
                                    let count = cmd.take_slice(&mut self.payload_channel, &mut data[start..]);
                                    debug_assert_eq!(count, frame.length as usize);                                    
                                } else {
                                    panic!("internal error, vec must be in new messages");
                                }

                                if frame.fragment_type == FragmentType::End {
                                    count += 1;
                                    collector.finish = Some(Instant::now());
                                }
                            }
                        }
                    } else {
                        warn!("Expected FrameDirection::Incoming");
                    }
                }
        );
        count
    }
}



impl AqueductTx {

    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }
    
    // // TODO: we may determine that this timestamp is not needed.
    // pub async fn try_send<CMD: SteadyCommander, T: ToSerial>(&mut self,  cmd: &mut CMD, message: T, bonus: u64) -> bool {
    //     if self.control_channel.shared_vacant_units()>0 {
    //         let length: Option<usize> = message.serialize_into(cmd, &mut self.payload_channel);
    //         assert_ne!(length, Some(0));
    //         if let Some(bytes_count) = length {
    //             let _ = cmd.try_send(&mut self.control_channel, AqueductMetadata { bytes_count, bonus });
    //             true
    //         } else {
    //             false
    //         }
    //     } else {
    //         false
    //     }
    // }
    
    //TODO:add other methods..
    
}


impl AqueductRx {

    pub fn is_closed_and_empty(&mut self) -> bool {
        self.control_channel.is_closed_and_empty() &&
        self.payload_channel.is_closed_and_empty()
    }


    async fn take_or_peek(&mut self, stream_id: i32 ) {
        //defrag
        //if channel needs more data for us await
        //if next chunk goes to vec which holds message we could become blocked !!

    }

    //not going to work until we defrag the data
    // pub async fn try_take<CMD: SteadyCommander, T:FromSerial
    // >(&mut self, cmd: &mut CMD) -> Option<T> {
    //     match cmd.try_take(&mut self.control_channel) {
    //         Some(meta) => {
    //             Some(T::deserialize_from(cmd, &mut self.payload_channel, meta))
    //         },
    //         None => None
    //     }
    // }

    //TODO:add other methods..

}



/// Builds a new Aqueduct using the specified control and payload channel builders.
///
/// This function creates a lazy-initialized `LazyAqueduct` wrapped in two convenience
/// handles: a transmitter (`LazyAqueductTx`) and a receiver (`LazyAqueductRx`).
/// The Aqueduct concept is about breaking large data into smaller fragments (control + payload)
/// for more efficient or flexible I/O handling.
///
/// # Arguments
/// - `control_builder`: A `ChannelBuilder` that sets up the control channel.
/// - `payload_builder`: A `ChannelBuilder` that sets up the payload channel.
///
/// # Returns
/// A tuple of (`LazyAqueductTx`, `LazyAqueductRx`):
/// 1. `LazyAqueductTx`: For sending (transmitting) Aqueduct fragments.
/// 2. `LazyAqueductRx`: For receiving (assembling) Aqueduct fragments.
///
pub fn build_aqueduct(control_builder: &ChannelBuilder
                      , payload_builder: &ChannelBuilder
                      , streams_first: i32
                      , streams_count: i32) -> (LazyAqueductTx, LazyAqueductRx) {
    let lazy = Arc::new(LazyAqueduct::new(control_builder, payload_builder, streams_first, streams_count));
    (
        LazyAqueductTx::new(lazy.clone()),
        LazyAqueductRx::new(lazy.clone())
    )
}

/// Type alias for ID used in Aeron. In Aeron, `i32` is commonly used for stream/session IDs.
pub type IdType = i32; // is i32 because this is what Aeron is using, unlikely to change

/// Specifies the type of fragment in a multi-part message.
///
/// This enum is used to track the position of each fragment in a larger message
/// that has been split into multiple parts. Such fragmentation may occur
/// when the data size exceeds certain limits (e.g., network MTU) or when
/// you wish to process data in smaller chunks for other reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)] // Ensure the enum is represented as an 8-bit integer.
pub enum FragmentType {
    /// Represents the first piece of a multi-part message.
    ///
    /// This indicates that no previous fragments exist for this message.
    /// Assembly code typically starts building a new message when it sees a `Begin` fragment.
    Begin = 1,

    /// Represents a piece that is neither the first nor the final fragment.
    ///
    /// Multiple `Middle` fragments may appear in a single message if it’s large.
    /// Assembly code continues adding these fragments until an `End` fragment is encountered.
    Middle = 0,

    /// Represents the final piece of a multi-part message.
    ///
    /// After receiving an `End` fragment, assembly code knows that the
    /// entire message is complete and no more fragments are expected.
    End = 2,

    /// Represents a complete message fitting into a single fragment.
    ///
    /// If the message is small enough, there’s no need for additional fragments.
    /// This indicates the message is already complete, without `Begin`, `Middle`, or `End` parts.
    UnFragmented = 3,
}


// Implement the `BitOr` operator for combining `FragmentType` values.
impl BitOr for FragmentType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        // Perform the OR operation on the numeric values and map it back to the `FragmentType`.
        match (self as u8) | (rhs as u8) {
            1 => FragmentType::Begin,
            2 => FragmentType::End,
            3 => FragmentType::UnFragmented,
            _ => FragmentType::Middle, // Default to `Middle` for all other cases.
        }
    }
}

impl FragmentType {
    /// Converts a numeric value back to `FragmentType`.
    /// This is useful if you need to decode the result of a bitwise operation.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(FragmentType::Middle),
            1 => Some(FragmentType::Begin),
            2 => Some(FragmentType::End),
            3 => Some(FragmentType::UnFragmented),
            _ => None, // Return `None` for invalid values.
        }
    }

    /// Gets the numeric value of the `FragmentType`.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Indicates whether a fragment is being sent or received, and includes
/// metadata (such as session ID and arrival time) for received fragments.
#[derive(Clone, Copy, Debug)]
pub enum FragmentDirection {
    /// Outgoing means this fragment is being transmitted from our side.
    Outgoing(),
    /// Incoming means this fragment was received.
    /// Along with the `IdType` (session ID) we store the `Instant` when it arrived.
    Incoming(IdType, Instant),
}

/// A single fragment in the Aqueduct system.
///
/// Each Aqueduct fragment includes:
/// - `fragment_type`: Which part of the message (begin, middle, end, or unfragmented).
/// - `length`: The size of the fragment in bytes.
/// - `option`: An optional parameter that can hold metadata (e.g., a user-defined marker).
/// - `stream_id`: The `IdType` for stream identification (in Aeron, typically an i32).
/// - `direction`: Indicates if it’s `Outgoing` or `Incoming`, and includes relevant metadata.
#[derive(Clone, Copy, Debug)]
pub struct AqueductFragment {
    pub(crate) fragment_type: FragmentType,
    pub(crate) length: i32,
    pub(crate) stream_id: IdType, // must be in the bound defined
    pub(crate) direction: FragmentDirection,
}

impl AqueductFragment {
    /// Creates a new `AqueductFragment` with the specified values.
    ///
    /// This constructor is the most flexible. You can specify exactly
    /// which fragment type, stream ID, direction, length, and an optional metadata.
    ///
    pub fn new(
        fragment_type: FragmentType,
        stream_id: IdType,
        direction: FragmentDirection,
        length: i32,
    ) -> Self {
        AqueductFragment {
            fragment_type,
            length,
            stream_id,
            direction
        }
    }

    /// Creates a simplified "unfragmented" outgoing fragment.
    ///
    /// If the message is small enough, there's no need for multiple fragments.
    /// This constructor helps create an `UnFragmented` outgoing piece, specifying
    /// only `stream_id` and `length`.
    pub fn simple_outgoing(stream_id: IdType, length: i32) -> Self {
        AqueductFragment {
            fragment_type: FragmentType::UnFragmented,
            length,
            stream_id,
            direction: FragmentDirection::Outgoing()
        }
    }
}

/// Represents a fully-assembled message received by the Aqueduct.
///
/// Once all fragments for a given message have been collected,
/// this `IncomingMessage` consolidates them into a single,
/// complete payload alongside relevant metadata.
///
/// # Fields
///
/// - **`stream_id`**  
///   The Aeron stream ID, which is used to differentiate data streams on a single channel.  
///   In Aeron, multiple streams can exist on the same transport session, so this field
///   helps identify which logical stream this message belongs to.
///
/// - **`session_id`**  
///   The Aeron session ID, which uniquely identifies the publisher in a multi-publisher setup.  
///   This can be useful when multiple senders are pushing data into the same stream and you
///   need to distinguish which sender produced this message.
///
/// - **`arrival`**  
///   The `Instant` (timestamp) at which the first fragment of this message was received.  
///   It provides a notion of when the message started arriving, which can help in measuring
///   latency or in scheduling follow-up actions based on reception time.
///
/// - **`finish`**  
///   An optional `Instant` indicating when the message was fully assembled.  
///   It remains `None` until all fragments for this message have been received.
///   Once the final fragment arrives, the assembly process updates `finish` to mark
///   the time at which the entire message is complete. This helps in calculating
///   total assembly time or diagnosing performance issues.
///
/// - **`data`**  
///   An optional `Vec<u8>` containing the assembled bytes of the message, if any.  
///   For some messages, you might have a control-only scenario with minimal or no actual payload,
///   in which case `data` could remain `None`. Otherwise, it holds the concatenation
///   of all fragments that were needed to reconstruct this message.
#[derive(Debug)]
pub struct IncomingMessage {
    /// The Aeron stream ID that identifies the logical stream within a single channel.
    pub stream_id: IdType,

    /// The Aeron session ID that identifies which publisher sent this message.
    pub session_id: IdType,

    /// The timestamp marking when the first fragment of this message was received.
    pub arrival: Instant,

    /// An optional timestamp marking when the final fragment arrived, completing this message.
    pub finish: Option<Instant>,

    /// An optional buffer holding the fully-assembled payload bytes of the message.
    pub data: Option<Vec<u8>>,
    
}

/// Core receiver for Aqueduct fragments.
///
/// - `control_channel`: Receives `AqueductFragment` messages (metadata about fragments).
/// - `payload_channel`: Receives the actual byte payloads of those fragments.
/// - `assembly`: Keeps track of partially or fully assembled messages.
///   * Keyed by `(stream_id, session_id)` => vector of `IncomingMessage`.
/// - `max_payload`: A limit on how large a single payload can be.
pub struct AqueductRx {
    pub(crate) control_channel: Rx<AqueductFragment>,
    pub(crate) payload_channel: Rx<u8>,
    pub(crate) stream_first: i32,
    pub(crate) stream_count: i32,
    pub(crate) collect: Vec<HashMap<IdType, Vec<IncomingMessage>>>
}

/// Thread-safe receiver reference for the Aqueduct, wrapped in `Arc<Mutex<…>>`.
pub type SteadyAqueductRx = Arc<Mutex<AqueductRx>>;

/// Describes metadata about an Aqueduct receiver, including control and payload channels.
pub trait AquaductRxDef {
    /// Returns `AquaductRxMetaData` describing the current channel metadata
    /// for both control and payload.
    fn meta_data(&self) -> AquaductRxMetaData;
}

/// Metadata about control and payload channels for a receiver, providing
/// introspection or debugging information.
pub struct AquaductRxMetaData {
    pub control: RxMetaData,
    pub payload: RxMetaData,
}

impl AquaductRxDef for SteadyAqueductRx {
    /// Retrieves metadata from the underlying channels, locking if necessary.
    ///
    /// If the lock is unavailable immediately, it uses `nuclei::block_on` to wait for it.
    /// This is a blocking call in that scenario, so be mindful of potential deadlocks
    /// in production code.
    fn meta_data(&self) -> AquaductRxMetaData {
        match self.try_lock() {
            Some(rx_lock) => {
                let m1 = RxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductRxMetaData { control: m1, payload: d2 }
            },
            None => {
                let rx_lock = nuclei::block_on(self.lock());
                let m1 = RxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = RxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductRxMetaData { control: m1, payload: d2 }
            }
        }
    }
}

/// Core transmitter for Aqueduct fragments.
///
/// - `control_channel`: Sends `AqueductFragment` metadata.
/// - `payload_channel`: Sends the actual bytes (payload) of fragments.
pub struct AqueductTx {
    pub(crate) control_channel: Tx<AqueductFragment>,
    pub(crate) payload_channel: Tx<u8>,
    pub(crate) stream_first: i32,
    pub(crate) stream_count: i32
}

impl AqueductTx {
    /// Creates a new `AqueductTx` from a control channel and a payload channel.
    ///
    /// Typically not used directly by library consumers; use `build_aqueduct`
    /// or `LazyAqueductTx::new` instead.
    pub(crate) fn new(control_channel: Tx<AqueductFragment>
                      , payload_channel: Tx<u8>
                      , stream_first:i32
                      , stream_count:i32) -> Self {
        AqueductTx {
            control_channel,
            payload_channel,
            stream_first,
            stream_count,
        }
    }
}

/// Thread-safe transmitter reference for the Aqueduct, wrapped in `Arc<Mutex<…>>`.
pub type SteadyAqueductTx = Arc<Mutex<AqueductTx>>;

/// Describes metadata about an Aqueduct transmitter, including control and payload channels.
pub trait AquaductTxDef {
    /// Returns `AquaductTxMetaData` describing the current channel metadata
    /// for both control and payload.
    fn meta_data(&self) -> AquaductTxMetaData;
}

/// Metadata about control and payload channels for a transmitter,
/// often used for diagnostics or debugging.
pub struct AquaductTxMetaData {
    pub(crate) control: TxMetaData,
    pub(crate) payload: TxMetaData,
}

impl AquaductTxDef for SteadyAqueductTx {
    /// Retrieves metadata from the underlying channels, locking if necessary.
    ///
    /// If the lock is unavailable immediately, it uses `nuclei::block_on` to wait for it.
    /// This is a blocking call in that scenario, so be mindful of potential deadlocks
    /// in production code.
    fn meta_data(&self) -> AquaductTxMetaData {
        match self.try_lock() {
            Some(rx_lock) => {
                let m1 = TxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductTxMetaData { control: m1, payload: d2 }
            },
            None => {
                let rx_lock = nuclei::block_on(self.lock());
                let m1 = TxMetaData(rx_lock.control_channel.channel_meta_data.clone());
                let d2 = TxMetaData(rx_lock.payload_channel.channel_meta_data.clone());
                AquaductTxMetaData { control: m1, payload: d2 }
            }
        }
    }
}

/// A lazy-initialized Aqueduct receiver handle.
///
/// This struct wraps a shared reference to `LazyAqueduct`. It defers
/// building the underlying channels (`AqueductRx`) until the first
/// time it's actually needed (via `clone()` or other calls).
pub struct LazyAqueductRx {
    lazy_channel: Arc<LazyAqueduct>,
}

impl LazyAqueductRx {
    /// Creates a new `LazyAqueductRx` from an existing `Arc<LazyAqueduct>`.
    pub(crate) fn new(lazy_channel: Arc<LazyAqueduct>) -> Self {
        LazyAqueductRx { lazy_channel }
    }

    /// Creates or retrieves a clone of the underlying `SteadyAqueductRx`.
    ///
    /// This uses `get_rx_clone()` on the `LazyAqueduct`, ensuring the channels
    /// are built if they weren't already. It then returns a clone of the
    /// `Arc<Mutex<AqueductRx>>`.
    ///
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyAqueductRx {
        nuclei::block_on(self.lazy_channel.get_rx_clone())
    }

    /// Asynchronous testing method that takes a frame of data from the `payload_channel`
    /// after reading a corresponding metadata fragment from the `control_channel`.
    ///
    /// # Arguments
    /// - `data`: A mutable byte slice to fill with the received payload.
    ///
    /// # Returns
    /// The number of bytes copied into `data`.
    ///
    /// # Panics
    /// - Panics if the length of the received fragment doesn't match `data.len()`.
    /// - Panics on any errors while reading the data or metadata.
    ///
    pub async fn testing_take_frame(&self, data: &mut [u8]) -> usize {
        let s = self.clone();
        let mut l = s.lock().await;
        if let Some(c) = l.control_channel.shared_take_async().await {
            assert_eq!(c.length as usize, data.len());
            let count = l.payload_channel.shared_take_slice(data);
            assert_eq!(count, c.length as usize);
            count
        } else {
            //trace!("shutdown happened while waiting to take");
            0
        }
    }
}

/// A lazy-initialized Aqueduct transmitter handle.
///
/// This struct wraps a shared reference to `LazyAqueduct`. It defers
/// building the underlying channels (`AqueductTx`) until the first
/// time it's actually needed (via `clone()` or other calls).
pub struct LazyAqueductTx {
    lazy_channel: Arc<LazyAqueduct>,
}

impl LazyAqueductTx {
    /// Creates a new `LazyAqueductTx` from an existing `Arc<LazyAqueduct>`.
    pub(crate) fn new(lazy_channel: Arc<LazyAqueduct>) -> Self {
        LazyAqueductTx { lazy_channel }
    }

    /// Creates or retrieves a clone of the underlying `SteadyAqueductTx`.
    ///
    /// This uses `get_tx_clone()` on the `LazyAqueduct`, ensuring the channels
    /// are built if they weren't already. It then returns a clone of the
    /// `Arc<Mutex<AqueductTx>>`.
    ///
    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> SteadyAqueductTx {
        nuclei::block_on(self.lazy_channel.get_tx_clone())
    }

    /// Asynchronous testing method that sends a frame of data (payload) and
    /// then pushes a corresponding metadata fragment into the `control_channel`.
    ///
    /// # Arguments
    /// - `data`: A byte slice representing the payload to send.
    ///
    /// # Panics
    /// - Panics if not all bytes are successfully sent to the `payload_channel`.
    /// - Panics on errors sending the metadata fragment to the `control_channel`.
    ///
    pub async fn testing_send_frame(&self, data: &[u8]) {
        let s = self.clone();
        let mut l = s.lock().await;
        let x = l.payload_channel.shared_send_slice_until_full(data);
        assert_eq!(x, data.len(), "Not all bytes were sent!");
        assert_ne!(x, 0);
        match l.control_channel.shared_try_send(AqueductFragment::simple_outgoing(1, x as i32)) {
            Ok(_) => {},
            Err(_) => { panic!("error sending metadata"); }
        };
    }

    /// Closes the underlying channels by marking them as closed.
    ///
    /// This signals to any receivers that no more data or fragments will arrive.
    /// This can help gracefully shut down I/O loops or polling threads.
    ///
    pub async fn testing_close(&self) {
        let s = self.clone();
        let mut l = s.lock().await;
        l.payload_channel.mark_closed();
        l.control_channel.mark_closed();
    }
}

/// A lazy-initialized Aqueduct that holds references to two `ChannelBuilder` objects:
/// one for control fragments, and one for payload data. It also stores a lazily-built
/// `(SteadyAqueductTx, SteadyAqueductRx)` pair that is created on first access.
///
/// This design allows the Aqueduct to be set up without immediately allocating channels.
/// Once `get_tx_clone()` or `get_rx_clone()` is called, it performs the `eager_build_internal()`
/// on both builders, obtaining actual channels for control and payload.
#[derive(Debug)]
struct LazyAqueduct {
    /// Builder for the control channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    control_builder: Mutex<Option<ChannelBuilder>>,
    /// Builder for the payload channel. Wrapped in a `Mutex<Option<…>>` so we can
    /// take ownership once we decide to build channels.
    payload_builder: Mutex<Option<ChannelBuilder>>,
    /// Stores the actual channel objects (`(SteadyAqueductTx, SteadyAqueductRx)`) once built.
    channel: Mutex<Option<(SteadyAqueductTx, SteadyAqueductRx)>>,
    /// first lowest stream id value
    streams_first:i32,
    /// count of streams starting with first, must be positive
    /// count+first must also be <= 31 bits
    streams_count:i32
}

impl LazyAqueduct {
    /// Creates a new `LazyAqueduct` with the given `control_builder` and `payload_builder`.
    ///
    /// The channels are not actually constructed until `get_tx_clone()` or `get_rx_clone()`
    /// is called for the first time, improving startup performance or deferring
    /// resource allocation until absolutely necessary.
    ///
    /// # Note
    /// Both builders are stored inside a `Mutex<Option<ChannelBuilder>>` so they can be
    /// moved out exactly once when building the channels.
    pub(crate) fn new(control_builder: &ChannelBuilder
                      , payload_builder: &ChannelBuilder
                      , streams_first: i32
                      , streams_count: i32
    ) -> Self {
        assert!(streams_count>=0);
        assert!((streams_first as i64 + streams_count as i64) < i32::MAX as i64);
        LazyAqueduct {
            control_builder: Mutex::new(Some(control_builder.clone())),
            payload_builder: Mutex::new(Some(payload_builder.clone())),
            channel: Mutex::new(None),
            streams_first,
            streams_count
        }
    }

    /// Ensures the underlying transmitter (`AqueductTx`) is initialized and returns
    /// a thread-safe handle to it (`SteadyAqueductTx`).
    ///
    /// If the channels haven’t been built yet, it will lock both the control and payload
    /// builders, construct the channels, store them, and then return the transmitter.
    /// If the channels are already built, it simply returns the existing transmitter.
    pub(crate) async fn get_tx_clone(&self) -> SteadyAqueductTx {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self.control_builder.lock().await
                .take().expect("internal error: control_builder missing");
            let data_builder = self.payload_builder.lock().await
                .take().expect("internal error: payload_builder missing");
            let (meta_tx, meta_rx) = meta_builder.eager_build_internal();
            let (data_tx, data_rx) = data_builder.eager_build_internal();

            let tx = Arc::new(Mutex::new(AqueductTx::new(meta_tx, data_tx, self.streams_first, self.streams_count)));
            let rx = Arc::new(Mutex::new(AqueductRx::new(meta_rx, data_rx, self.streams_first, self.streams_count)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").0.clone()
    }

    /// Ensures the underlying receiver (`AqueductRx`) is initialized and returns
    /// a thread-safe handle to it (`SteadyAqueductRx`).
    ///
    /// If the channels haven’t been built yet, it will lock both the control and payload
    /// builders, construct the channels, store them, and then return the receiver.
    /// If the channels are already built, it simply returns the existing receiver.
    pub(crate) async fn get_rx_clone(&self) -> SteadyAqueductRx {
        let mut channel = self.channel.lock().await;
        if channel.is_none() {
            let meta_builder = self.control_builder.lock().await
                .take().expect("internal error: control_builder missing");
            let data_builder = self.payload_builder.lock().await
                .take().expect("internal error: payload_builder missing");
            let (meta_tx, meta_rx) = meta_builder.eager_build_internal();
            let (data_tx, data_rx) = data_builder.eager_build_internal();

            let tx = Arc::new(Mutex::new(AqueductTx::new(meta_tx, data_tx, self.streams_first, self.streams_count)));
            let rx = Arc::new(Mutex::new(AqueductRx::new(meta_rx, data_rx, self.streams_first, self.streams_count)));
            *channel = Some((tx, rx));
        }
        channel.as_ref().expect("internal error").1.clone()
    }
}
