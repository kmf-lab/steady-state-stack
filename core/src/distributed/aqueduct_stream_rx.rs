//! StreamRx lives here so aqueduct_stream.rs stays under the 1,200-line budget.
//! Re-exported from `aqueduct_stream` — public paths unchanged.

use super::*;

/// Represents a receiver for a steady stream, managing control and payload channels.
#[derive(Debug)]
// ss[related distributed.aqueduct-stream]

// ss[related distributed.aqueduct-stream]
pub struct StreamRx<T: StreamControlItem> {
    /// The control channel for receiving stream metadata.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) control_channel: Rx<T>,
    /// The payload channel for receiving raw data bytes.
    // ss[related distributed.aqueduct-stream]
    pub(crate) payload_channel: Rx<u8>,
    /// Flag indicating whether to focus on control channel metadata.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) spotlight_control: bool,
}

/// Methods for managing receiver streams.
// ss[related distributed.aqueduct-stream]
impl<T: StreamControlItem> StreamRx<T> {
    /// Creates a new receiver stream with the specified control and payload channels.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn new(control_channel: Rx<T>, payload_channel: Rx<u8>) -> Self {
        StreamRx {
            control_channel,
            payload_channel,
            spotlight_control: false,
        }
    }

    /// Attempts to take a single message and its associated payload from the receiver.
    // ss[related distributed.aqueduct-stream]
    pub fn try_take(&mut self) -> Option<(T, Box<[u8]>)> {
        if let Some((_done, msg)) = self.shared_try_take() {
            Some(msg)
        } else {
            None
        }
    }

    /// Returns the capacity of the control channel.
    // ss[related distributed.aqueduct-stream]
    pub fn capacity(&mut self) -> usize {
        self.control_channel.capacity()
    }

    /// Returns the combined **reserved** memory of this stream's dual ring buffers.
    ///
    /// Stream channels allocate two separate buffers at build time:
    /// - **Control** — `control_capacity × size_of::<T>()` for ingress metadata slots.
    /// - **Payload** — `control_capacity × bytes_per_item` raw `u8` bytes for message bodies.
    ///
    /// This method sums both footprints (same formula as
    /// [`Rx::memory_bytes`](crate::steady_rx::Rx::memory_bytes) on each sub-channel).
    /// It reports the configured maximum, not live occupancy.
    ///
    /// # Returns
    /// Total reserved buffer footprint in bytes.
    // ss[impl channel.memory-usage-telemetry]
    pub fn memory_bytes(&self) -> usize {
        self.control_channel.memory_bytes() + self.payload_channel.memory_bytes()
    }

    /// Returns the number of available units in the control and payload channels.
    // ss[related distributed.aqueduct-stream]
    pub fn avail_units(&mut self) -> (usize, usize) {
        self.shared_avail_units()
    }

    /// Checks if both control and payload channels are closed.
    // ss[related distributed.aqueduct-stream]
    pub fn is_closed(&mut self) -> bool {
        self.control_channel.is_closed() && self.payload_channel.is_closed()
    }

    /// Checks if both control and payload channels are empty.
    // ss[related distributed.aqueduct-stream]
    pub fn is_empty(&mut self) -> bool {
        self.control_channel.is_empty() && self.payload_channel.is_empty()
    }

    /// Consumes messages from the receiver, applying a provided function to process the data up to a byte limit.
    // ss[related distributed.aqueduct-stream]
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
    // ss[related distributed.aqueduct-stream]
    pub(crate) fn extract_stream_payload_slices<'a>(
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

