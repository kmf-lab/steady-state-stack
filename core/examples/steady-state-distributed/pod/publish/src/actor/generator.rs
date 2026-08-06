use steady_state::*;

/// The number of units (messages) that should be generated per "beat" of the system.
/// This constant must match the expectations of any client or consumer of this actor's output.
/// It is used to calculate the total number of messages to generate during the actor's lifetime.
#[cfg(test)]
const EXPECTED_UNITS_PER_BEAT: u64 = 8;
#[cfg(not(test))]
const EXPECTED_UNITS_PER_BEAT: u64 = 1_000_000; // MUST MATCH THE CLIENT EXPECTATIONS

/// Persistent state for the Generator actor.
///
/// This struct holds all the information that must survive panics, restarts, or failures.
/// By storing the total number of messages generated so far, the actor can resume exactly
/// where it left off after a crash, ensuring that no data is lost and no messages are duplicated.
pub(crate) struct GeneratorState {
    /// The total number of messages that have been generated and sent so far.
    /// This value is incremented as new messages are produced and is used to
    /// resume message generation after a restart.
    pub(crate) total_generated: u64,
}

/// Entry point for the Generator actor.
///
/// This asynchronous function is called by the actor system to start the Generator.
/// It receives a handle to the actor's context (`actor`), a sending channel (`generated_tx`)
/// for outputting generated values, and a reliable state object (`state`).
///
/// The function determines whether to run the actor's real internal logic or a simulated
/// behavior (for testing or orchestration). The internal logic is responsible for
/// generating a sequence of numbers and sending them in batches to the output channel,
/// while maintaining robust, reliable state.
pub async fn run(
    actor: SteadyActorShadow,
    generated_tx: SteadyTx<u64>,
    state: SteadyState<GeneratorState>,
) -> Result<(), Box<dyn Error>> {
    // Convert the actor context into a "spotlight" mode, which enables monitoring
    // and robust error handling. The spotlight is configured with the output channel.
    let actor = actor.into_spotlight([], [&generated_tx]);

    // Decide which behavior to use: the real internal logic or a simulated/test mode.
    if actor.use_internal_behavior {
        internal_behavior(actor, generated_tx, state).await
    } else {
        actor.simulated_behavior(vec!(&generated_tx)).await
    }
}

/// The core logic for the Generator actor.
///
/// This function implements the main loop for generating and sending messages.
/// It uses batching to maximize throughput, and it ensures that all state changes
/// (such as incrementing the message counter) are only made after successful operations.
/// The function is robust to failures: if the actor panics or crashes, the persistent
/// state ensures that it can resume without data loss or duplication.
///
/// The function also periodically logs throughput statistics and requests a system
/// shutdown when the target number of messages has been generated.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    generated: SteadyTx<u64>,
    state: SteadyState<GeneratorState>,
) -> Result<(), Box<dyn Error>> {
    // Retrieve command-line arguments or configuration for this actor.
    // These arguments determine how many "beats" (iterations) the actor should run.
    let args = actor.args::<crate::MainArg>().expect("unable to downcast");
    let beats = args.beats;

    // Calculate the total number of messages to generate, based on the number of beats
    // and the expected number of units per beat.
    let final_total = beats * EXPECTED_UNITS_PER_BEAT;

    // Lock the output channel for exclusive access during message generation.
    let mut generated = generated.lock().await;

    // Lock the persistent state for this actor. If this is the first run, initialize
    // the state with zero messages generated. If this is a restart, the previous state
    // is restored automatically.
    let mut state = state.lock(|| GeneratorState {
        total_generated: 0,
    }).await;

    // Determine the batch size for sending messages. This is set to one quarter of the
    // channel's total capacity, allowing for efficient batch processing without
    // overwhelming the channel.
    let wait_for = generated.capacity() / 4;

    // Start generating messages from the last value stored in the persistent state.
    let mut next_value = state.total_generated;

    // Main loop: continue running as long as the actor system is active and the
    // output channel is open.
    while actor.is_running(|| generated.mark_closed()) {
        // Wait until there is enough space in the output channel to send a full batch.
        await_for_all!(actor.wait_vacant(&mut generated, wait_for));

        if state.total_generated < final_total {
            // Prepare to write a batch of messages into the channel's buffer.
            // The buffer may be split into two slices for efficient access.
            let (poke_a, poke_b) = actor.poke_slice(&mut generated);

            // Calculate the total number of slots available for writing in this batch.
            let count = poke_a.len() + poke_b.len();

            // Write sequential values into the first slice of the buffer.
            for i in 0..poke_a.len() {
                poke_a[i].write(next_value);
                next_value += 1;
            }
            // Write sequential values into the second slice of the buffer, if present.
            for i in 0..poke_b.len() {
                poke_b[i].write(next_value);
                next_value += 1;
            }

            //only advance up to our final_total
            let count = if count as u64 + state.total_generated < final_total {
                count
            } else {
                (final_total - state.total_generated) as usize
            };

            // Commit the batch of messages to the channel, advancing the send index.
            // The number of messages actually sent is returned.
            let sent_count = actor.advance_send_index(&mut generated, count).item_count();

            // Update the persistent state to reflect the total number of messages generated.
            state.total_generated += sent_count as u64;

            // Periodically log the total number of messages sent for monitoring and diagnostics.
            // This log is written every 8192 messages (2^13).
            if 0 == (state.total_generated & ((1u64 << 13) - 1)) {
                trace!("Generator: {} total messages sent", state.total_generated);
            }

        } else {
                info!("Generator is done {}", state.total_generated);
                actor.request_shutdown().await;
        }
        
    }

    // When the loop exits, log the final total and return successfully.
    info!("Generator shutting down. Total generated: {}", state.total_generated);
    Ok(())
}

/// Unit tests for the Generator actor's internal behavior.
///
/// This module contains tests that verify the correct operation of the Generator actor.
/// The tests use a simulated actor system to create channels, run the actor, and
/// check that the expected number of messages are generated and sent in batches.
///
/// The tests also verify that the actor starts at zero, increments correctly, and
/// produces at least one full batch of messages.
#[cfg(test)]
pub(crate) mod generator_tests {
    use std::thread::sleep;
    use steady_state::*;
    use crate::arg::MainArg;
    use super::*;

    #[test]
    fn test_generator() -> Result<(), Box<dyn Error>> {
        // Create a test graph (actor system) with default configuration.
        let mut graph = GraphBuilder::for_testing().build(MainArg::default());

        // Create a channel for the generator to send messages, with a larger capacity
        // to allow for batch processing during the test.
        let (generate_tx, generate_rx) = graph.channel_builder()
            .with_capacity(1024) // Larger capacity for testing
            .build();

        // Create a new persistent state object for the actor.
        let state = new_state();

        // Build the Generator actor in the test graph, using the internal behavior.
        // The actor is run as a SoloAct, meaning it runs on its own thread for isolation.
        graph.actor_builder()
            .with_name("UnitTest")
            .build(
                move |context| internal_behavior(context, generate_tx.clone(), state.clone()),
                SoloAct,
            );

        // Start the actor system and allow it to run for a short time.
        graph.start();
        sleep(Duration::from_millis(100));

        // Request a shutdown of the actor system.
        graph.request_shutdown();

        // Wait for the system to stop, with a timeout to prevent hanging.
        graph.block_until_stopped(Duration::from_secs(1))?;

        // Collect all messages that were sent by the generator during the test.
        let messages: Vec<u64> = generate_rx.testing_take_all();

        // Verify that at least one full batch of messages was generated.
        assert!(messages.len() >= 256); // At least one full batch

        // Verify that the first two messages are 0 and 1, confirming correct sequencing.
        assert_eq!(messages[0], 0);
        assert_eq!(messages[1], 1);

        Ok(())
    }
}