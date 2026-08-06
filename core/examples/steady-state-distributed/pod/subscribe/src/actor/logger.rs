use steady_state::*;
use crate::actor::worker::FizzBuzzMessage;

/// Reliable state for the Logger actor.
///
/// This struct tracks all statistics and counters needed for robust, high-throughput logging.
/// By storing these values in a reliable state, the logger can recover from panics or restarts
/// without losing track of how many messages have been processed or the breakdown of Fizz/Buzz types.
pub(crate) struct LoggerState {
    /// Total number of messages logged by this actor.
    pub(crate) messages_logged: u64,
    /// The batch size used for high-performance processing.
    pub(crate) batch_size: usize,
    /// Number of Fizz messages processed.
    pub(crate) fizz_count: u64,
    /// Number of Buzz messages processed.
    pub(crate) buzz_count: u64,
    /// Number of FizzBuzz messages processed.
    pub(crate) fizzbuzz_count: u64,
    /// Number of Value (plain number) messages processed.
    pub(crate) value_count: u64,
}

#[cfg(test)]
const VALUES_PER_HEARTBEAT: u64 = 16;
#[cfg(not(test))]
const VALUES_PER_HEARTBEAT: u64 = 1_000_000;


/// Entry point for the Logger actor.
///
/// This function is called by the actor system to start the Logger. It receives:
/// - The actor context (`actor`), which manages the actor's lifecycle and provides access to arguments and control.
/// - An input channel (`fizz_buzz_rx`) for receiving FizzBuzzMessage values from upstream.
/// - A persistent state object (`state`) for tracking all logging statistics.
///
/// The function links the actor to its input channel, then delegates to the core processing logic.
pub async fn run(
    actor: SteadyActorShadow,
    fizz_buzz_rx: SteadyRx<FizzBuzzMessage>,
    state: SteadyState<LoggerState>,
) -> Result<(), Box<dyn Error>> {
    // Prepare the actor for robust monitoring and error handling.
    let actor = actor.into_spotlight([&fizz_buzz_rx], []);
    // Choose between real internal logic or simulated/test mode.
    if actor.use_internal_behavior {
        internal_behavior(actor, fizz_buzz_rx, state).await
    } else {
        actor.simulated_behavior(vec!(&fizz_buzz_rx)).await
    }
}

/// Core logic for the Logger actor: high-throughput, robust batch logging.
///
/// This function implements the main loop for processing and logging FizzBuzz messages.
/// It uses batching to maximize throughput, and it ensures that all state changes
/// (such as incrementing counters) are only made after successful operations.
/// The function is robust to failures: if the actor panics or crashes, the reliable
/// steady state ensures that it can resume without data loss or duplication.
///
/// The function also periodically logs statistics for monitoring and diagnostics.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    rx: SteadyRx<FizzBuzzMessage>,
    state: SteadyState<LoggerState>,
) -> Result<(), Box<dyn Error>> {
    let args = actor.args::<crate::MainArg>().expect("unable to downcast");
    let beats = args.beats;
    let expected_total = beats * VALUES_PER_HEARTBEAT;

    // Lock the input channel for exclusive access during message processing.
    let mut rx = rx.lock().await;

    // Lock the persistent state for this actor. If this is the first run, initialize
    // the state with zeroed counters and a batch size based on channel capacity.
    let mut state = state.lock(|| LoggerState {
        messages_logged: 0,
        batch_size: rx.capacity() / 2,
        fizz_count: 0,
        buzz_count: 0,
        fizzbuzz_count: 0,
        value_count: 0,
    }).await;

    // Pre-allocate a buffer for batch processing. This avoids repeated allocations
    // and enables high-performance, zero-copy message handling.
    let mut batch = vec![FizzBuzzMessage::default(); state.batch_size];

    let max_latency = Duration::from_millis(40);
    // Main loop: continue running as long as the actor system is active and the
    // input channel is not closed and empty.
    while actor.is_running(|| rx.is_closed_and_empty()) {
        // Wait for either a periodic timer or enough messages to fill a batch.
        // This ensures that the logger is responsive even if the input rate is low.
        await_for_all_or_proceed_upon!(
            actor.wait_periodic(max_latency),
            actor.wait_avail(&mut rx, state.batch_size)
        );

        // Determine how many messages are available to process.
        let available = actor.avail_units(&mut rx);
        if available > 0 {
            // Process up to batch_size messages at once for maximum throughput.
            let batch_size = available.min(state.batch_size);
            let taken = actor.take_slice(&mut rx, &mut batch[..batch_size]).item_count();

            if taken > 0 {
                // Process the entire batch efficiently.
                for &msg in &batch[..taken] {
                    match msg {
                        FizzBuzzMessage::Fizz => {
                            state.fizz_count += 1;
                        }
                        FizzBuzzMessage::Buzz => {
                            state.buzz_count += 1;
                        }
                        FizzBuzzMessage::FizzBuzz => {
                            state.fizzbuzz_count += 1;
                        }
                        FizzBuzzMessage::Value(_) => {
                            state.value_count += 1;
                        }
                    }

                    state.messages_logged += 1;


                    // Log statistics for monitoring and diagnostics.
                    // - Log every message for the first 16, then at large intervals for performance.
                    if state.messages_logged < 16
                        || (state.messages_logged & ((1u64 << 27) - 1)) == 0
                    {
                        info!(
                            "Logger: {} messages processed (F:{}, B:{}, FB:{}, V:{})",
                            state.messages_logged,
                            state.fizz_count,
                            state.buzz_count,
                            state.fizzbuzz_count,
                            state.value_count
                        );
                    }
                }
                if state.messages_logged == expected_total {
                    info!("shutdown requested");
                    actor.request_shutdown().await;
                }



            }
        }
    }

    // When the loop exits, log the final totals and return successfully.
    info!(
        "Logger shutting down. Total: {} (F:{}, B:{}, FB:{}, V:{})",
        state.messages_logged,
        state.fizz_count,
        state.buzz_count,
        state.fizzbuzz_count,
        state.value_count
    );
    Ok(())
}

/// Unit test for the Logger actor's batch processing and statistics.
///
/// This test verifies that the logger can process a large batch of FizzBuzz messages
/// and that persistent state reflects the correct totals after shutdown.
#[test]
fn test_logger() -> Result<(), Box<dyn std::error::Error>> {
    use std::thread::sleep;
    use std::time::Duration;
    use crate::arg::MainArg;

    let mut graph = GraphBuilder::for_testing().build(MainArg::default());
    let (fizz_buzz_tx, fizz_buzz_rx) = graph.channel_builder()
        .with_capacity(4096) // Large capacity for performance
        .build();

    let state = new_state();
    let state_after = state.clone();
    graph.actor_builder().with_name("UnitTest")
        .build(
            move |context| {
                internal_behavior(context, fizz_buzz_rx.clone(), state.clone())
            },
            SoloAct,
        );

    graph.start();

    let test_messages: Vec<FizzBuzzMessage> = (0..1000)
        .map(FizzBuzzMessage::new)
        .collect();
    fizz_buzz_tx.testing_send_all(test_messages, true);

    sleep(Duration::from_millis(500));
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(1))?;

    let mut locked = None;
    for _ in 0..50 {
        if let Some(s) = state_after.try_lock_sync() {
            locked = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let locked = locked.expect("logger state should be lockable after graph stop");
    assert_eq!(locked.messages_logged, 1000);

    Ok(())
}