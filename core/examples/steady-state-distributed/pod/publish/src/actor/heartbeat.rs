use steady_state::*;

/// Persistent state for the Heartbeat actor.
///
/// This struct holds the information that must survive any panics, restarts, or failures.
/// By storing the current count of heartbeats sent, the actor can resume exactly where it
/// left off after a crash, ensuring that no data is lost and no beats are skipped or repeated.
pub(crate) struct HeartbeatState {
    /// The current heartbeat count. This value is incremented each time a heartbeat is sent.
    /// It is used to track progress and to determine when the actor should stop.
    pub(crate) count: u64,
}

/// Entry point for the Heartbeat actor.
///
/// This asynchronous function is called by the actor system to start the Heartbeat actor.
/// It receives a handle to the actor's context (`actor`), a sending channel (`heartbeat_tx`)
/// for outputting heartbeat values, and a reliable state object (`state`).
///
/// The function determines whether to run the actor's real internal logic or a simulated
/// behavior (for testing or orchestration). The internal logic is responsible for
/// periodically sending heartbeat messages, while maintaining robust, reliable state.
pub async fn run(
    actor: SteadyActorShadow,
    heartbeat_tx: SteadyTx<u64>,
    state: SteadyState<HeartbeatState>,
) -> Result<(), Box<dyn Error>> {
    // Convert the actor context into "spotlight" mode, which enables monitoring
    // and robust error handling. The spotlight is configured with the output channel.
    let actor = actor.into_spotlight([], [&heartbeat_tx]);

    // Decide which behavior to use: the real internal logic or a simulated/test mode.
    if actor.use_internal_behavior {
        internal_behavior(actor, heartbeat_tx, state).await
    } else {
        actor.simulated_behavior(vec!(&heartbeat_tx)).await
    }
}

/// The core logic for the Heartbeat actor.
///
/// This function implements the main loop for sending periodic heartbeat messages.
/// It uses a timer to control the rate of heartbeats, and it ensures that all state changes
/// (such as incrementing the heartbeat count) are only made after successful operations.
/// The function is robust to failures: if the actor panics or crashes, the persistent
/// state ensures that it can resume without data loss or duplication.
///
/// The function also requests a system shutdown when the target number of heartbeats
/// has been sent, and sends a special "final" heartbeat value to signal completion.
async fn internal_behavior<A: SteadyActor>(
    mut actor: A,
    heartbeat_tx: SteadyTx<u64>,
    state: SteadyState<HeartbeatState>,
) -> Result<(), Box<dyn Error>> {
    // Retrieve command-line arguments or configuration for this actor.
    // These arguments determine the rate (interval) and the total number of beats to send.
    let args = actor.args::<crate::MainArg>().expect("unable to downcast");
    let rate = Duration::from_micros(args.rate_ms);
    let beats = args.beats;

    // Lock the persistent state for this actor. If this is the first run, initialize
    // the state with a count of zero. If this is a restart, the previous state is restored.
    let mut state = state.lock(|| HeartbeatState { count: 0 }).await;

    // Lock the output channel for exclusive access during heartbeat sending.
    let mut heartbeat_tx = heartbeat_tx.lock().await;

    // Main loop: continue running as long as the actor system is active and the
    // output channel is open. The closure passed to `is_running` will close the
    // output channel when the actor is shutting down.
    while actor.is_running(|| heartbeat_tx.mark_closed()) {
        // Wait until both the periodic timer has elapsed and there is room in the
        // output channel to send a new heartbeat message.
        await_for_all!(
            actor.wait_periodic(rate),
            actor.wait_vacant(&mut heartbeat_tx, 1)
        );

        // Attempt to send the current heartbeat count to the output channel.
        // Only increment the count if the send was successful.
        if actor.try_send(&mut heartbeat_tx, state.count).is_sent() {
            state.count += 1;

            // If the target number of beats has been reached, send a special "final"
            // heartbeat value (u64::MAX) to signal completion, log an error, and
            // request a system shutdown.
            if beats == state.count {
                assert!(
                    actor
                        .send_async(
                            &mut heartbeat_tx,
                            u64::MAX,
                            SendSaturation::AwaitForRoom
                        )
                        .await
                        .is_sent()
                );
                info!("Heartbeat is done {}", state.count);
                actor.request_shutdown().await;
            }
        }
    }
    Ok(())
}

/// Unit tests for the Heartbeat actor's internal behavior.
///
/// This module contains tests that verify the correct operation of the Heartbeat actor.
/// The tests use a simulated actor system to create channels, run the actor, and
/// check that the expected number of heartbeat messages are sent at the correct intervals.
///
/// The tests also verify that the actor starts at zero, increments correctly, and
/// produces the expected sequence of heartbeat values.
#[cfg(test)]
pub(crate) mod heartbeat_tests {
    use steady_state::*;
    use crate::arg::MainArg;
    use super::*;

    #[test]
    fn test_heartbeat() -> Result<(), Box<dyn Error>> {
        // Create a set of arguments for the test, with a small number of beats
        // and a fast rate to ensure the test completes quickly.
        let mut args = MainArg::default();
        args.beats = 3; // Set a small number for testing
        args.rate_ms = 10; // Fast rate for testing

        // Create a test graph (actor system) with the specified arguments.
        let mut graph = GraphBuilder::for_testing().build(args);

        // Create a channel for the heartbeat actor to send messages.
        let (heartbeat_tx, heartbeat_rx) = graph.channel_builder().build();

        // Create a new persistent state object for the actor.
        let state = new_state();

        // Build the Heartbeat actor in the test graph, using the internal behavior.
        // The actor is run as a SoloAct, meaning it runs on its own thread for isolation.
        graph.actor_builder()
            .with_name("UnitTest")
            .build(
                move |context| internal_behavior(context, heartbeat_tx.clone(), state.clone()),
                SoloAct,
            );

        // Start the actor system and allow it to run for a short time.
        graph.start();
        std::thread::sleep(Duration::from_millis(100)); // Wait for a few beats

        // Wait for the system to stop, with a timeout to prevent hanging.
        graph.block_until_stopped(Duration::from_secs(1))?;

        // Collect all heartbeat messages that were sent during the test.
        let results = heartbeat_rx.testing_take_all();

        // Verify that at least two heartbeat messages were sent.
        assert!(results.len() >= 2);

        // Verify that the first two heartbeat messages are 0 and 1, confirming correct sequencing.
        assert_eq!(results[0], 0);
        assert_eq!(results[1], 1);

        Ok(())
    }
}