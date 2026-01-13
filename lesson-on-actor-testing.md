┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Lesson: Deep Logic Verification—Unit Testing with internal_behavior                                                                                      ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

In high-concurrency systems, the most common bugs aren't in the "wiring" (how actors connect), but in the "brain" (the internal logic of the actor). 
Steady State provides a powerful simulation engine for integration tests, but for verifying math, state transitions, and data transformations, 
you need pure isolation.

To achieve this, Steady State follows a mandatory architectural pattern: **Unit tests must bypass the `run` function and call `internal_behavior` directly.**

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Dispatcher Pattern: Understanding the `run` Function

Every idiomatic Steady State actor provides a `run` function. It is crucial to understand that `run` is not where your logic lives; it is a **dispatcher**. 
Its only job is to decide which "mode" the actor should operate in based on the `use_internal_behavior` flag.

```rust
pub async fn run(context: SteadyActorShadow, rx: SteadyRx<Msg>, tx: SteadyTx<Msg>) -> Result<(), Box<dyn Error>> {
    // Convert the shadow context into a spotlight (monitor) context
    let mut actor = context.into_spotlight([&rx], [&tx]);

    // THE SWITCH
    if actor.use_internal_behavior {
        // PRODUCTION / UNIT TEST MODE
        // The actor executes its real, hand-written logic.
        internal_behavior(actor, rx, tx).await
    } else {
        // INTEGRATION TEST MODE (Simulation)
        // The actor becomes a "Puppet" controlled by the StageManager.
        // It ignores its internal logic and waits for side-channel commands.
        actor.simulated_behavior(sim_runners!(rx, tx)).await
    }
}
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. The "Puppet Trap": Why `run` Fails in Unit Tests

When you use `GraphBuilder::for_testing()`, the framework assumes you are building a full-system simulation. Consequently, it sets `use_internal_behavior` 
to `false` for every actor added to that graph.

If you call `run` inside a unit test:
1. The actor sees `use_internal_behavior == false`.
2. It enters `simulated_behavior`.
3. It sits idle, waiting for a `StageManager` to tell it what to do.
4. Since your unit test doesn't use a `StageManager` (it uses direct channel injection), the actor never does anything.
5. Your test hangs indefinitely or times out.

**The Golden Rule:** If you want to test the logic inside `internal_behavior`, you must call it yourself.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. Anatomy of a Logic-First Unit Test

A robust actor unit test follows a specific six-step orchestration. This ensures that you are testing the actor in a "real" environment but with 
total control over the inputs and outputs.

### Step 1: Initialize the Test Graph
Use `GraphBuilder::for_testing()`. This provides the necessary infrastructure (telemetry, shutdown signals) without spawning real OS threads.

### Step 2: Create Test Channels
Use the `channel_builder` to create the "pipes" for your actor. Ensure the capacity is large enough to hold your entire test data set to avoid 
blocking before the actor even starts.

### Step 3: The Direct Call (The "Magic" Step)
When building the actor, pass a closure that calls `internal_behavior` directly. This bypasses the `run` dispatcher and forces the actor into 
"Production Mode" even though it's in a test graph.

### Step 4: Start and Inject
Start the graph, then use `testing_send_all` to push data into the input channels. Setting the `close` flag to `true` is vital; it tells the 
actor's `is_running` loop that no more data is coming.

### Step 5: Orchestrated Shutdown
Call `graph.request_shutdown()` and `graph.block_until_stopped()`. This ensures the actor has finished processing all injected data and has 
exited its loop cleanly.

### Step 6: Verification
Use the provided macros to verify the results in the output channels.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. Comprehensive Example: Testing a Batching Actor

Imagine an actor that batches 3 numbers together and sends their sum.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_summation_logic() -> Result<(), Box<dyn Error>> {
        // 1. Setup Graph
        let mut graph = GraphBuilder::for_testing().build(());
        
        // 2. Setup Channels
        let (tx_in, rx_in) = graph.channel_builder().with_capacity(10).build::<u64>();
        let (tx_out, rx_out) = graph.channel_builder().with_capacity(10).build::<u64>();

        // 3. Build Actor - CALLING internal_behavior DIRECTLY
        graph.actor_builder()
            .with_name("SumActor")
            .build(move |c| {
                internal_behavior(c, rx_in.clone(), tx_out.clone())
            }, SoloAct);

        graph.start();

        // 4. Inject Data
        // We send 6 items, expecting 2 sums out.
        tx_in.testing_send_all(vec![1, 1, 1, 2, 2, 2], true);

        // 5. Shutdown
        graph.request_shutdown();
        graph.block_until_stopped(Duration::from_secs(1))?;

        // 6. Verify
        assert_steady_rx_eq_count!(rx_out, 2);
        assert_steady_rx_eq_take!(rx_out, vec![3, 6]);
        Ok(())
    }
}
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Advanced Unit Testing Scenarios

### Testing with `SteadyState<S>`
If your actor is stateful, unit testing is the best time to verify that state survives panics or restarts.
1. Create a `SteadyState` in the test.
2. Pass a clone to the actor.
3. After `block_until_stopped`, use `state.try_lock_sync()` to inspect the final state of the struct.

### Testing Error Handling
You can simulate a "bad" upstream actor by closing the input channel prematurely.
```rust
tx_in.testing_close(); // Close without sending data
graph.block_until_stopped(Duration::from_secs(1))?;
// Verify that the actor exited cleanly instead of hanging
```

### Testing Batching Performance
Use `take_slice` and `send_slice` inside your `internal_behavior`. In your unit test, you can verify that the actor correctly handles "partial" 
batches (e.g., if you send 5 items but the batch size is 10, does it wait or process what it has?).

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Summary: Logic vs. Wiring

| Feature | Unit Test (Logic) | Integration Test (Wiring) |
| :--- | :--- | :--- |
| **Target** | `internal_behavior` | `run` |
| **Mechanism** | Direct Function Call | `StageManager` / Side-Channels |
| **Data Flow** | `testing_send_all` | `stage.actor_perform(...)` |
| **Goal** | Verify the "Brain" | Verify the "Pipes" |
| **Complexity** | Low (Isolated) | High (System-wide) |

**Final Lesson:** The `run` function is for the system; `internal_behavior` is for the developer. Never confuse the two in your tests, or you will 
find yourself debugging the simulation instead of your logic.
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛
