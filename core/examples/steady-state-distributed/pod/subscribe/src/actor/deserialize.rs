use std::error::Error;
use steady_state::*;

/// Persistent state for the deserialization actor.
///
/// Tracks shutdown signals and the next expected sequence number for each stream.
/// Ensures correctness and enables recovery after panics or restarts.
pub(crate) struct DeserializeState {
    /// Number of shutdown signals received (shutdown after 2).
    shutdown_count: i32,
    /// Next expected number in the heartbeat stream.
    next_heartbeat: u64,
    /// Next expected number in the generator stream.
    next_generator: u64,
}

/// Entry point for the deserialization actor.
///
/// Called by the actor system to start the deserialization process. Receives:
/// - `actor`: The actor context, managing lifecycle and providing access to arguments and control.
/// - `input`: Input bundle with two stream channels (heartbeat and generator), each with control and payload.
/// - `heartbeat`, `generator`: Output channels for deserialized `u64` values.
/// - `state`: Reliable state for tracking shutdown and sequence.
///
/// Links the actor to its input/output channels, then delegates to the core processing logic.
pub(crate) async fn run(
    actor: SteadyActorShadow,
    input: SteadyStreamRxBundle<StreamIngress, 2>,
    heartbeat: SteadyTx<u64>,
    generator: SteadyTx<u64>,
    state: SteadyState<DeserializeState>,
) -> Result<(), Box<dyn Error>> {
    let actor = actor.into_spotlight(input.payload_meta_data(), [&heartbeat, &generator]);
    internal_behavior(actor, input, heartbeat, generator, state).await
}

/// Maximum number of u64s to process in a single batch during deserialization.
/// This is set to half the output channel capacity for efficient double-buffering.
const MAX_BATCH: usize = 8192;

/// Core logic for zero-copy deserialization of incoming streams.
///
/// This function runs an async loop, listening to two input streams (heartbeat and generator),
/// deserializing their payloads into `u64` values, checking sequence, and forwarding them to
/// output channels. It uses direct buffer access for zero-copy efficiency.
///
/// # Stream Structure
/// Each input stream consists of:
///   - Control channel: For each group, a struct is read containing the length (in bytes) of the payload.
///   - Payload channel: The actual serialized bytes, packed tightly.
///
/// The function ensures that the control and payload channels remain in sync: for every
/// control message, there is a corresponding block of bytes in the payload channel.
///
/// # Sequence Checking
/// The function checks that the numbers in each stream arrive in strict order, panicking
/// if any out-of-order or missing value is detected. This is crucial for distributed
/// systems where data loss or duplication can occur.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    input: SteadyStreamRxBundle<StreamIngress, 2>,      //#!#//
    heartbeat: SteadyTx<u64>,               //#!#//
    generator: SteadyTx<u64>,
    state: SteadyState<DeserializeState>,
) -> Result<(), Box<dyn Error>> {
    // Lock and extract the two input streams.
    let mut input = input.acquire_guard().await;
    let mut rx_generator = input.remove(1); // Generator stream (index 1)
    let mut rx_heartbeat = input.remove(0); // Heartbeat stream (index 0)
    drop(input);            //#!#//

    // Lock output channels for sending deserialized data.
    let mut tx_heartbeat = heartbeat.acquire_guard().await;
    let mut tx_generator = generator.acquire_guard().await;

    // Initialize or access persistent state.
    let mut state = state.acquire_guard(|| DeserializeState {
        shutdown_count: 0,
        next_heartbeat: 0,
        next_generator: 0,
    }).await;

    // Main processing loop: runs until the actor shuts down or streams are exhausted.
    while actor.is_running(|| {
        rx_heartbeat.is_closed_and_empty()
            && rx_generator.is_closed_and_empty()
            && tx_generator.mark_closed()
            && tx_heartbeat.mark_closed()
    }) {
        // Wait for data availability in either stream and space in output channels.
        await_for_any!(      //#!#//
            wait_for_all!(
                actor.wait_avail(&mut rx_heartbeat, 1),
                actor.wait_vacant(&mut tx_heartbeat, 1)
            ),
            wait_for_all!(
                actor.wait_avail(&mut rx_generator, 1),
                actor.wait_vacant(&mut tx_generator, 1)
            )
        );

        // Process heartbeat stream if data is available.
        if actor.avail_units(&mut rx_heartbeat).0 > 0 {
            let mut next_heartbeat = state.next_heartbeat;
            let mut shutdown_count = state.shutdown_count;
            process_stream::<A>(
                &mut actor,
                &mut rx_heartbeat,
                &mut tx_heartbeat,
                &mut next_heartbeat,
                &mut shutdown_count
            )
                .await?;
            state.next_heartbeat = next_heartbeat;
            state.shutdown_count = shutdown_count;
        }

        // Process generator stream if data is available.
        if actor.avail_units(&mut rx_generator).0 > 0 {
            let mut next_generator = state.next_generator;
            let mut shutdown_count = state.shutdown_count;
            process_stream(
                &mut actor,
                &mut rx_generator,
                &mut tx_generator,
                &mut next_generator,
                &mut shutdown_count
            )
                .await?;
            state.next_generator = next_generator;
            state.shutdown_count = shutdown_count;
        }

        // If two shutdown signals have been received, request shutdown.
        if state.shutdown_count >= 2 {
            actor.request_shutdown().await; //#!#//
        }
    }
    info!("Deserialization actor shut down");
    Ok(())
}

/// Processes a single input stream (heartbeat or generator) in zero-copy batches.
///
/// Reads control and payload slices directly from the ring buffer, deserializes `u64` values,
/// checks sequence, and writes them to the output channel. Handles shutdown signals and
/// ensures batch integrity.
///
/// # Arguments
/// - `actor`: The actor context.
/// - `rx`: Input stream (with control and payload).
/// - `tx`: Output channel for deserialized `u64` values.
/// - `next_expected`: Mutable reference to the next expected sequence number.
/// - `shutdown_count`: Mutable reference to the shutdown signal counter.
/// - `on_seq_error`: Closure to call on sequence error (for panic or error reporting).
async fn process_stream<A: SteadyActor>(
    actor: &mut A,
    rx: &mut StreamRx<StreamIngress>,
    tx: &mut Tx<u64>,
    next_expected: &mut u64,
    shutdown_count: &mut i32
) -> Result<(), Box<dyn Error>>
{
    // Preallocate a buffer for split payloads (max group size).
    let mut combined = Vec::with_capacity(MAX_BATCH * 8);

    // Peek control and payload slices (may be split due to ring buffer wraparound).
    let (peek_control_a, peek_control_b, peek_payload_a, peek_payload_b) = actor.peek_slice(rx);    //#!#//
    let (poke_a, poke_b) = actor.poke_slice(tx);    //#!#//

    let mut control_pos = 0;
    let mut payload_pos = 0;
    let mut output_pos = 0;
    let output_cap = poke_a.len() + poke_b.len();

    // Process as many control messages as possible, up to output capacity.
    while control_pos < peek_control_a.len() + peek_control_b.len() {
        // Read control message (group length in bytes).
        let control = if control_pos < peek_control_a.len() {
            peek_control_a[control_pos]
        } else {
            peek_control_b[control_pos - peek_control_a.len()]
        };
        let group_bytes = control.length() as usize;
        let group_count = group_bytes / 8;

        // If not enough output space for this group, break.
        if output_pos + group_count > output_cap {
            break;
        }

        // Ensure we don't overrun the payload buffer.
        let payload_end = payload_pos + group_bytes;
        let (payload_slice, next_payload_pos): (&[u8], usize) = if payload_pos < peek_payload_a.len() {
            if payload_end <= peek_payload_a.len() {
                // All in a
                (&peek_payload_a[payload_pos..payload_end], payload_end)
            } else {
                // Split between a and b
                let a_rem = peek_payload_a.len() - payload_pos;
                let b_rem = group_bytes - a_rem;
                if b_rem > peek_payload_b.len() {
                    // Not enough data in b, break
                    break;
                }
                combined.clear();
                combined.extend_from_slice(&peek_payload_a[payload_pos..]);
                combined.extend_from_slice(&peek_payload_b[..b_rem]);
                (&combined[..], group_bytes) // next_payload_pos is not used in this branch
            }
        } else {
            // All in b
            let b_start = payload_pos - peek_payload_a.len();
            let b_end = b_start + group_bytes;
            if b_end > peek_payload_b.len() {
                // Not enough data in b, break
                break;
            }
            (&peek_payload_b[b_start..b_end], payload_end)
        };

        // Deserialize each u64 in the group.
        for i in 0..group_count {
            let offset = i * 8;
            let bytes: [u8; 8] = payload_slice[offset..offset + 8]
                .try_into()
                .expect("Failed to read 8 bytes for u64");
            let value = u64::from_be_bytes(bytes);

            if value == u64::MAX {
                *shutdown_count += 1;
            } else {
                #[cfg(not(test))] // ensure nothing is dropped when we run all in order.
                if value != *next_expected {
                    panic!(
                        "Heartbeat sequence error: expected {}, got {}",
                        *next_expected, value);
                }
                *next_expected += 1;

                // Write to output (poke_a or poke_b).
                if output_pos < poke_a.len() {
                    poke_a[output_pos].write(value);
                } else {
                    poke_b[output_pos - poke_a.len()].write(value);
                }
                output_pos += 1;
            }
        }

        control_pos += 1;
        payload_pos = if payload_pos < peek_payload_a.len() {
            if payload_end <= peek_payload_a.len() {
                next_payload_pos
            } else {
                // We crossed into b, so all of a is consumed, so start at b_rem in b.
                peek_payload_a.len() + (group_bytes - (peek_payload_a.len() - payload_pos))
            }
        } else {
            // All in b
            next_payload_pos
        };
    }

    // Advance indices to reflect processed data.
    actor.advance_take_index(rx, (control_pos, payload_pos));    //#!#//
    actor.advance_send_index(tx, output_pos);    //#!#//

    Ok(())
}

#[cfg(test)]
pub(crate) mod deserialize_tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;
    use crate::arg::MainArg;

    /// Tests the basic deserialization and sequence checking functionality.
    ///
    /// This test sends two batches of sequential numbers to each input stream and verifies
    /// that the deserialized output matches the expected sequence. It also checks that
    /// shutdown and sequence validation work as intended.
    #[test]
    fn test_deserialize() -> Result<(), Box<dyn Error>> {
        let mut graph = GraphBuilder::for_testing().build(MainArg::default());

        // Set up communication channels for testing.
        let (stream_tx, stream_rx) = graph.channel_builder().build_stream_bundle::<_, 2>(8);
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder().build();
        let (generator_tx, generator_rx) = graph.channel_builder().build();

        let state = new_state();
        graph
            .actor_builder()
            .with_name("UnitTest")
            .build(
                move |context| {
                    internal_behavior(
                        context,
                        stream_rx.clone(),
                        heartbeat_tx.clone(),
                        generator_tx.clone(),
                        state.clone(),
                    )
                },
                SoloAct,
            );

        // Utility function to serialize a sequence of numbers into a byte vector.
        fn serialize_numbers(nums: &[u64]) -> Vec<u8> {
            let mut bytes = Vec::with_capacity(nums.len() * 8);
            for &num in nums {
                bytes.extend_from_slice(&num.to_be_bytes());
            }
            bytes
        }

        let now = Instant::now();
        // Send sequential numbers to heartbeat stream: [0, 1] and [2, 3].
        let heartbeat_payload1 = serialize_numbers(&[0, 1]);
        let heartbeat_payload2 = serialize_numbers(&[2, 3]);
        stream_tx[0].testing_send_all(vec![
            StreamIngress::by_ref(16, now, now, &heartbeat_payload1),
            StreamIngress::by_ref(16, now, now, &heartbeat_payload2),
        ], true);

        // Send sequential numbers to generator stream: [0, 1] and [2, 3].
        let generator_payload1 = serialize_numbers(&[0, 1]);
        let generator_payload2 = serialize_numbers(&[2, 3]);
        stream_tx[1].testing_send_all(vec![
            StreamIngress::by_ref(16, now, now, &generator_payload1),
            StreamIngress::by_ref(16, now, now, &generator_payload2),
        ], true);

        // Start the graph, allow processing, then shut down.
        graph.start();
        sleep(Duration::from_millis(100));
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(1))?;

        // Verify that all numbers were received in order.
        let heartbeat_received: Vec<u64> = heartbeat_rx.testing_take_all();
        assert_eq!(heartbeat_received, vec![0, 1, 2, 3], "Heartbeat stream did not receive expected sequence");

        let generator_received: Vec<u64> = generator_rx.testing_take_all();
        assert_eq!(generator_received, vec![0, 1, 2, 3], "Generator stream did not receive expected sequence");

        Ok(())
    }
}