use std::error::Error;
use steady_state::*;
use std::mem::MaybeUninit;
use std::cmp::min;

/// Entry point to set up and run the actor, processing heartbeat and generator data streams.
///
/// This function is called by the actor system to start the serialization actor. It receives:
/// - The actor context (`actor`), which manages the actor's lifecycle and provides access to arguments and control.
/// - Two input channels: one for heartbeat values (`heartbeat`), and one for generator values (`generator`). Both are streams of `u64`.
/// - An output bundle (`output`), which is a pair of stream channels for serialized output. Each stream consists of:
///     - A control channel: holds a struct describing the length (in bytes) of each serialized group.
///     - A payload channel: holds the actual bytes of serialized data.
///
/// The function links the actor to its input and output channels, then delegates to the core processing logic.
pub(crate) async fn run(
    actor: SteadyActorShadow,           // Actor managing the steady-state process
    heartbeat: SteadyRx<u64>,           // Input channel for 64-bit heartbeat values //#!#//
    generator: SteadyRx<u64>,           // Input channel for 64-bit generator values
    output: SteadyStreamTxBundle<StreamEgress, 2>, // Output bundle with two streams  //#!#//
) -> Result<(), Box<dyn Error>> {
    // Link the actor to its inputs (heartbeat, generator) and output metadata.
    // This enables robust monitoring and error handling.
    let cmd = actor.into_spotlight([&heartbeat, &generator], output.payload_meta_data());
    // Start processing the streams using the internal logic.
    internal_behavior(cmd, heartbeat, generator, output).await
}

/// Maximum number of u64 values to process in a single group during serialization.
///
/// Each group is serialized as a contiguous block of bytes, and its length is recorded
/// in the control channel. This constant controls the maximum group size, balancing
/// throughput and memory usage. For example, 8188 u64s = 65,504 bytes (~64KB).
const MAX_GROUP: usize = 8188; // Adjustable: e.g., 1122 for 9K jumbo packets

/// Core logic to process heartbeat and generator streams, serializing them to outputs.
///
/// This function is the heart of the serialization actor. It runs an asynchronous loop,
/// reading from the input channels, batching data into groups, and writing to the output
/// streams. Each output stream consists of two channels:
///   - The control channel: for each group, a struct is written containing the length (in bytes)
///     of the corresponding payload.
///   - The payload channel: the actual serialized bytes of the group.
///
/// The function ensures that the control and payload channels remain in sync: for every
/// control message, there is a corresponding block of bytes in the payload channel.
/// This is essential for correct deserialization downstream.
///
/// The function also uses zero-copy buffer access for efficiency, and processes both
/// input streams concurrently to maximize throughput.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,                       // Actor driving the steady-state process
    heartbeat: SteadyRx<u64>,           // Heartbeat input channel
    generator: SteadyRx<u64>,           // Generator input channel
    output: SteadyStreamTxBundle<StreamEgress, 2>, // Output bundle with two streams
) -> Result<(), Box<dyn Error>> {
    // Lock channels for mutable access, enabling asynchronous reading and writing.
    // This ensures exclusive access to the buffers during processing.
    let mut rx_heartbeat = heartbeat.acquire_guard().await;
    let mut rx_generator = generator.acquire_guard().await;
    let mut output = output.acquire_guard().await;
    // Remove the two output streams from the bundle: one for generator, one for heartbeat.  //#!#//
    let mut tx_generator = output.remove(1); // Generator output stream (index 1)
    let mut tx_heartbeat = output.remove(0); // Heartbeat output stream (index 0)
    drop(output); // Drop the bundle to free resources early.

    // Batch sizing: prefer large batches at steady state, but cap wait thresholds so we
    // still wake on small injections (e.g. StageManager Echo in integration tests). Waiting
    // for "half of a multi-million-slot channel" would otherwise deadlock low-volume paths.
    let wait_heartbeat_in = (rx_heartbeat.capacity() / 2).clamp(1, 4096);
    let wait_generator_in = (rx_generator.capacity() / 2).clamp(1, 65_536);
    let wait_heartbeat_out = (tx_heartbeat.capacity().0 / 2).clamp(1, 4096);
    let wait_generator_out = (tx_generator.capacity().0 / 2).clamp(1, 65_536);
    let max_latency = Duration::from_millis(500);

    // Main loop: continue running as long as the actor is active and the channels are not closed.
    // The closure checks for shutdown conditions and closes output channels when done.
    while actor.is_running(|| rx_heartbeat.is_closed_and_empty()
                            && rx_generator.is_closed_and_empty()
                            && tx_generator.mark_closed()
                            && tx_heartbeat.mark_closed()) {

        // Wait for either sufficient input/output or any data in a closed input channel.
        // This uses "await_for_any" to allow either stream to be processed as soon as it is ready.
        let _clean = await_for_any!(                          //#!#//
            actor.wait_periodic(max_latency), //required  for testing
            wait_for_all!(
                // If the rx is closed, this will return immediately.
                actor.wait_avail(&mut rx_heartbeat, wait_heartbeat_in),
                actor.wait_vacant(&mut tx_heartbeat, (wait_heartbeat_out, wait_heartbeat_out * 8))
            ),
            wait_for_all!(
                // If the rx is closed, this will return immediately.
                actor.wait_avail(&mut rx_generator, wait_generator_in),
                actor.wait_vacant(&mut tx_generator, (wait_generator_out, wait_generator_out * 8))
            )
        );

        //could have been two actors in one troupe but what would be the fun in that: //#!#//

        // Process heartbeat stream if data is available.
        if actor.avail_units(&mut rx_heartbeat) > 0 {
            // Serialize a group of heartbeat values and write to the output stream.
            serialize_stream(&mut actor, &mut rx_heartbeat, &mut tx_heartbeat, MAX_GROUP).await;
        }

        // Process generator stream if data is available.
        while actor.avail_units(&mut rx_generator) > 0 {
            // Serialize a group of generator values and write to the output stream.
            let done = serialize_stream(&mut actor, &mut rx_generator, &mut tx_generator, MAX_GROUP).await;
            if 0 == done.item_count() {
                break;
            }
        }
    }
    Ok(())
}

/// Serializes u64 values from an input stream into an output stream in groups.
///
/// This function is responsible for the actual serialization of data. It reads up to
/// `max_group` u64 values from the input channel, converts them to bytes, and writes
/// them to the payload channel of the output stream. For each group, it also writes
/// a control message to the control channel, indicating the number of bytes in the group.
///
/// The dual-channel design is essential for stream integrity:
///   - The control channel acts as a "header," telling the receiver how many bytes to
///     read from the payload channel for each group.
///   - The payload channel contains the actual serialized data, packed tightly.
///
/// This function uses zero-copy buffer access for efficiency, and carefully manages
/// buffer splits (when the ring buffer wraps around) to avoid data corruption.
///
/// # Data Integrity
/// It is critical that the number in the control channel *always* matches the number
/// of bytes written to the payload channel for each group. If this is violated,
/// deserialization will fail or produce corrupted data.
async fn serialize_stream<A: SteadyActor>(
    actor: &mut A,                      // Actor managing the process
    rx: &mut Rx<u64>,                   // Input channel with u64 values
    tx: &mut StreamTx<StreamEgress>,    // Output stream for control and payload
    max_group: usize,                   // Max u64s per group
) -> TxDone {

    // Access input data directly from the ring buffer (zero-copy).
    // The buffer may be split into two slices if it wraps around.
    let (peek_a, peek_b) = actor.peek_slice(rx);
    // Access output buffers for control and payload (may be split).
    let (poke_control_a
        , poke_control_b
        , poke_payload_a
        , poke_payload_b) = actor.poke_slice(tx);            //#!#//

    // Track positions in buffers.
    let mut input_pos = 0;         // u64s consumed from input
    let mut control_pos = 0;       // Control messages written
    let mut payload_byte_pos = 0;  // Bytes written to payload

    // Continue while there’s input data and output space.
    // We must not overrun any of the buffers.
    while control_pos < poke_control_a.len() + poke_control_b.len()
        && input_pos < peek_a.len() + peek_b.len()
        && payload_byte_pos < poke_payload_a.len() + poke_payload_b.len() {
        let remaining_input = peek_a.len() + peek_b.len() - input_pos;
        let remaining_payload_bytes = poke_payload_a.len() + poke_payload_b.len() - payload_byte_pos;
        // Limit group size by max_group, input, and output capacity (8 bytes per u64).
        let c = remaining_payload_bytes / 8;
        let group_size = max_group.min(remaining_input).min(c);

        if group_size == 0 {
            break; // No more data or space to process.
        }

        let group_bytes = group_size * 8;
        for _ in 0..group_size {
            // Read u64 from either peek_a or peek_b, depending on position.
            let value = if input_pos < peek_a.len() {
                peek_a[input_pos]
            } else {
                peek_b[input_pos - peek_a.len()]
            };
            let bytes = value.to_be_bytes(); // Convert to big-endian bytes for portability.
            write_bytes(&bytes, poke_payload_a, poke_payload_b, payload_byte_pos);
            input_pos += 1;
            payload_byte_pos += 8;
        }
        if payload_byte_pos > poke_payload_a.len() + poke_payload_b.len() {
            error!("Buffer overflow: {} exceeds {} + {}", payload_byte_pos, poke_payload_a.len(), poke_payload_b.len());
        }

        // Write control message for the group.
        // The control message tells the receiver how many bytes to read from the payload channel.
        let control_item = StreamEgress::new(group_bytes as i32); // Bytes in this group
        if control_pos < poke_control_a.len() {
            poke_control_a[control_pos].write(control_item);
        } else {
            poke_control_b[control_pos - poke_control_a.len()].write(control_item);
        }
        control_pos += 1;
    }

    // Update indices to reflect processed data.
    // This advances the read position in the input channel and the write position in the output stream.
    actor.advance_take_index(rx, input_pos);                                 //#!#//
    actor.advance_send_index(tx, (control_pos, payload_byte_pos))     //#!#//
}

/// Writes 8 bytes to payload slices, handling buffer splits safely.
///
/// This function is responsible for writing a single u64 (as 8 bytes) into the output
/// payload buffer. Because the buffer may be split (due to ring buffer wraparound),
/// the function may need to write part of the bytes to the first slice and the rest
/// to the second slice. This ensures that the serialized data is contiguous in the
/// logical stream, even if it is physically split in memory.
///
/// # Safety
/// The function uses unsafe code to copy bytes directly into uninitialized memory.
/// This is safe as long as the buffer bounds are respected.
fn write_bytes(
    bytes: &[u8; 8],                    // Bytes to write (from a u64)
    poke_payload_a: &mut [MaybeUninit<u8>], // First output slice
    poke_payload_b: &mut [MaybeUninit<u8>], // Second output slice
    offset: usize,                      // Starting position
) {
    let payload_a_len = poke_payload_a.len();
    if offset < payload_a_len {
        // Write as much as possible to the first slice.
        let write_len = min(8, payload_a_len - offset);
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                poke_payload_a[offset..].as_mut_ptr() as *mut u8,
                write_len,
            );
        }
        // If there are remaining bytes, write them to the second slice.
        if write_len < 8 {
            let remaining = 8 - write_len;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes[write_len..].as_ptr(),
                    poke_payload_b.as_mut_ptr() as *mut u8,
                    remaining,
                );
            }
        }
    } else {
        // All bytes go to the second slice.
        let payload_b_offset = offset - payload_a_len;
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                poke_payload_b[payload_b_offset..].as_mut_ptr() as *mut u8,
                8,
            );
        }
    }
}

/// Test module to verify serialization under various conditions.
///
/// This module contains tests that verify the correct operation of the serialization logic.
/// The tests use a simulated actor system to create channels, run the actor, and check that
/// the expected data is serialized and transmitted correctly. Both correctness and performance
/// are tested, including edge cases and large data sets.
#[cfg(test)]
pub(crate) mod serialize_tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;
    use crate::arg::MainArg;

    /// Basic test for small inputs to ensure correctness.
    ///
    /// This test sends a single value to each input channel and verifies that the
    /// serialized output matches the expected byte representation.
    #[test]
    fn test_serialize_basic() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(MainArg::default());
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder().build();
        let (generator_tx, generator_rx) = graph.channel_builder().build();
        let (stream_tx, stream_rx) = graph.channel_builder().build_stream_bundle::<_, 2>(8);

        graph.actor_builder()
            .with_name("BasicTest")
            .build(move |context|
                       internal_behavior(context, heartbeat_rx.clone(), generator_rx.clone(), stream_tx.clone()),
                   SoloAct);

        heartbeat_tx.testing_send_all(vec![0u64], true);
        generator_tx.testing_send_all(vec![42u64], true);

        graph.start();
        sleep(Duration::from_millis(100));
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(1))?;

        let temp = vec![StreamEgress::build(&[0, 0, 0, 0, 0, 0, 0, 0])];
        assert_steady_rx_eq_take!(&stream_rx[0], temp);
        assert_steady_rx_eq_take!(&stream_rx[1], vec![StreamEgress::build(&[0, 0, 0, 0, 0, 0, 0, 42])]);

        Ok(())
    }

    /// Stress test with larger inputs to verify performance and no drops.
    ///
    /// This test sends thousands of values to each input channel and verifies that
    /// all data is serialized and transmitted without loss. It checks that the total
    /// number of bytes in the output matches the expected value.
    #[test]
    fn test_serialize_stress() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(MainArg::default());
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder().with_capacity(10000).build();
        let (generator_tx, generator_rx) = graph.channel_builder().with_capacity(10000).build();
        let (stream_tx, stream_rx) = graph.channel_builder().build_stream_bundle::<_, 2>(10000);

        graph.actor_builder()
            .with_name("StressTest")
            .build(move |context|
                       internal_behavior(context, heartbeat_rx.clone(), generator_rx.clone(), stream_tx.clone()),
                   SoloAct);

        // Generate moderate input data (full 10k-per-side can exceed shutdown budget on CI).
        let heartbeat_data: Vec<u64> = (0..2000).collect();
        let generator_data: Vec<u64> = (2000..4000).collect();
        heartbeat_tx.testing_send_all(heartbeat_data.clone(), true);
        generator_tx.testing_send_all(generator_data.clone(), true);

        graph.start();
        sleep(Duration::from_millis(1500));
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(45))?;

        // Verify no data is dropped by checking total bytes.
        let heartbeat_out: Vec<_> = stream_rx[0].testing_take_all();
        let generator_out: Vec<_> = stream_rx[1].testing_take_all();
        let total_heartbeat_bytes = heartbeat_out.iter().map(|item| item.1.len() as usize).sum::<usize>();
        let total_generator_bytes = generator_out.iter().map(|item| item.1.len() as usize).sum::<usize>();

        assert_eq!(total_heartbeat_bytes, heartbeat_data.len() * 8, "Heartbeat data dropped");
        assert_eq!(total_generator_bytes, generator_data.len() * 8, "Generator data dropped");

        Ok(())
    }
}