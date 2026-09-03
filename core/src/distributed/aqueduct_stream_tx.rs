//! StreamTx and Defrag live here so aqueduct_stream.rs stays under the 1,200-line budget.
//! Re-exported from `aqueduct_stream` — public paths unchanged.

use super::*;

/// Represents a transmitter for a steady stream, managing control and payload channels with defragmentation support.
// ss[related distributed.aqueduct-stream]
pub struct StreamTx<T: StreamControlItem> {
    /// The control channel for sending stream metadata.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) control_channel: Tx<T>,
    /// The payload channel for sending raw data bytes.
    // ss[related distributed.aqueduct-stream]
    pub(crate) payload_channel: Tx<u8>,
    /// A map of session IDs to defragmentation entries for reassembling fragmented messages.
    defrag: AHashMap<i32, Defrag<T>>,
    /// A queue of session IDs with ready messages for processing.
    // ss[related distributed.aqueduct-stream]
    pub(crate) ready_msg_session: VecDeque<i32>,
    /// The timestamp of the last input data received.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) last_input_instant: Instant,
    /// The timestamp of the last output data sent.
    // ss[related distributed.aqueduct-stream]
    pub(crate) last_output_instant: Instant,
    /// The current index for the input rate collector.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) input_rate_index: usize,
    /// An array collecting input rate statistics (duration, messages, bytes).
    // ss[related distributed.aqueduct-stream]
    pub(crate) input_rate_collector: [(Duration, u32, u32); RATE_COLLECTOR_LEN],
    /// The maximum latency allowed for polling operations.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) max_poll_latency: Duration,
    /// The current index for the output rate collector.
    // ss[related distributed.aqueduct-stream]
    pub(crate) output_rate_index: usize,
    /// An array collecting output rate statistics (duration, messages, bytes).
    // ss[related philosophy.structural-hierarchy]
    pub(crate) output_rate_collector: [(Duration, u32, u32); RATE_COLLECTOR_LEN],
    /// Cached values for available message and byte capacities.
    // ss[related distributed.aqueduct-stream]
    pub(crate) stored_vacant_values: (i32, i32),
    /// Flag indicating whether to focus on control channel metadata.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) spotlight_control: bool,
}

/// Implementation of debug formatting for transmitter streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> Debug for StreamTx<T> {
    // ss[related philosophy.structural-hierarchy]
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
// ss[related distributed.aqueduct-stream]
pub struct Defrag<T: StreamControlItem> {
    /// The time when the first fragment was received, if available.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) arrival: Option<Instant>,
    /// The time when the last fragment was received, if available.
    // ss[related distributed.aqueduct-stream]
    pub(crate) finish: Option<Instant>,
    /// The session identifier for the defragmentation entry.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) session_id: i32,
    /// The cumulative length of data in the defragmentation buffer.
    // ss[related distributed.aqueduct-stream]
    pub(crate) running_length: usize,
    /// Ring buffers for storing stream control items (producer and consumer).
    #[allow(clippy::type_complexity)]
    // ss[related distributed.aqueduct-stream]
    pub(crate) ringbuffer_items: (
        AsyncWrap<Arc<AsyncRb<Heap<T>>>, true, false>,
        AsyncWrap<Arc<AsyncRb<Heap<T>>>, false, true>,
    ),
    /// Ring buffers for storing raw byte data (producer and consumer).
    // ss[related distributed.aqueduct-stream]
    pub(crate) ringbuffer_bytes: (
        AsyncWrap<Arc<AsyncRb<Heap<u8>>>, true, false>,
        AsyncWrap<Arc<AsyncRb<Heap<u8>>>, false, true>,
    ),
}

/// Methods for managing defragmentation entries.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> Defrag<T> {
    /// Creates a new defragmentation entry with the specified session ID and buffer capacities.
    // ss[related philosophy.structural-hierarchy]
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
    // ss[related distributed.aqueduct-stream]
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
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> StreamTx<T> {
    /// Creates a new transmitter stream with the specified control and payload channels.
    // ss[impl distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
    pub(crate) fn set_stored_vacant_values(&mut self, messages: i32, total_bytes_for_messages: i32) {
        self.stored_vacant_values = (messages, total_bytes_for_messages);
    }

    /// Retrieves the cached values for available message and byte capacities.
    // ss[related distributed.aqueduct-stream]
    pub(crate) fn get_stored_vacant_values(&mut self) -> (i32, i32) {
        self.stored_vacant_values
    }

    /// Records input data rate statistics, including duration, message count, and byte count.
    // ss[related distributed.aqueduct-stream]
    pub fn store_input_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.input_rate_index += 1;
        self.input_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration, messages, total_bytes_for_messages);
    }

    /// Records output data rate statistics, including duration, message count, and byte count.
    // ss[related distributed.aqueduct-stream]
    pub fn store_output_data_rate(&mut self, duration: Duration, messages: u32, total_bytes_for_messages: u32) {
        self.output_rate_index += 1;
        self.output_rate_collector[RATE_COLLECTOR_MASK & self.input_rate_index] = (duration, messages, total_bytes_for_messages);
    }

    /// Estimates the minimum and maximum durations for processing pending data based on available capacity and historical rates.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
    pub fn mark_closed(&mut self) -> bool {
        self.control_channel.mark_closed();
        self.payload_channel.mark_closed();
        true
    }

    /// Returns the capacities of the control and payload channels.
    // ss[related distributed.aqueduct-stream]
    pub fn capacity(&self) -> (usize, usize) {
        (self.control_channel.capacity(), self.payload_channel.capacity())
    }

    /// Returns the combined **reserved** memory of this stream's dual ring buffers.
    ///
    /// Stream channels allocate two separate buffers at build time:
    /// - **Control** — `control_capacity × size_of::<T>()` for `StreamEgress` metadata slots.
    /// - **Payload** — `control_capacity × bytes_per_item` raw `u8` bytes for message bodies.
    ///
    /// This method sums both footprints (same formula as
    /// [`Tx::memory_bytes`](crate::steady_tx::Tx::memory_bytes) on each sub-channel).
    /// It reports the configured maximum, not live occupancy.
    ///
    /// # Returns
    /// Total reserved buffer footprint in bytes.
    // ss[impl channel.memory-usage-telemetry]
    pub fn memory_bytes(&self) -> usize {
        self.control_channel.memory_bytes() + self.payload_channel.memory_bytes()
    }

    /// Flushes ready defragmented messages to the control and payload channels, returning the number of messages and bytes processed.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
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
