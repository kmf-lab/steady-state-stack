# Testing Steady State Actors

Steady State provides a comprehensive testing framework that lets you verify actor logic without starting the full graph, and integration tests that exercise the whole pipeline. This guide explains both approaches.

---

## Why Testing Matters

- **Unit tests** call `internal_behavior` directly, giving you full control over input and output channels.
- **Integration tests** use the `StageManager` to drive actors from the outside, validating end‑to‑end flows.
- Both approaches are **fast** and **deterministic** – no sleeping or polling for timeouts.

---

## Unit Testing Actors

### Rule: Never call `run()` in unit tests

When you use `GraphBuilder::for_testing()`, all actors are configured with `use_internal_behavior = false`. Calling `run()` enters `simulated_behavior`, which **hangs forever** because there is no `StageManager` driving it.

**Always call `internal_behavior` directly in your unit tests.**

### Example: Testing a simple greeter

```rust
use steady_state::*;

// The internal behavior we want to test
async fn greeter_behavior<C: SteadyActor>(
    mut actor: C,
    rx: SteadyRx<Greeting>,
    tx: SteadyTx<Greeting>,
) -> Result<(), Box<dyn Error>> {
    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    while actor.is_running(|| rx.is_closed_and_empty() && tx.mark_closed()) {
        let _clean = await_for_all!(
            actor.wait_avail(&mut rx, 1),
            actor.wait_vacant(&mut tx, 1)
        );

        if let Some(msg) = actor.try_take(&mut rx) {
            let reply = Greeting(msg.0.to_uppercase());
            let _ = actor.try_send(&mut tx, reply);
        }
    }
    Ok(())
}

#[test]
fn test_greeter() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx_in, rx_in) = graph.channel_builder()
        .with_capacity(10)
        .build_channel::<Greeting>();
    let (tx_out, rx_out) = graph.channel_builder()
        .with_capacity(10)
        .build_channel::<Greeting>();

    // Register the actor with internal_behavior directly
    graph.actor_builder()
        .with_name("greeter")
        .build(move |context| greeter_behavior(context, rx_in.clone(), tx_out.clone()),
               SoloAct);

    graph.start();

    // Inject test data
    tx_in.testing_send_all(vec![Greeting("hello".into())], true);

    // Wait for shutdown
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(5))?;

    // Verify output
    assert_steady_rx_eq_count!(&rx_out, 1);
    assert_steady_rx_eq_take!(&rx_out, vec![Greeting("HELLO".into())]);

    Ok(())
}
```

### Key Methods for Unit Tests

| Method | Purpose |
|--------|---------|
| `testing_send_all(data, close)` | Sends all items and optionally closes the channel. |
| `testing_take_all()` | Takes all available items from a channel. |
| `testing_close()` | Closes the channel without sending anything. |
| `assert_steady_rx_eq_count!(rx, expected)` | Asserts available item count. |
| `assert_steady_rx_eq_take!(rx, expected_vec)` | Asserts the exact sequence of values. |

---

## Integration Testing with StageManager

The `StageManager` lets you send side‑channel commands to an actor while the graph is running. This is useful for testing distributed or complex workflows.

### How Side Channels Work

When you call `GraphBuilder::for_testing()` with a `StageManager`:

1. A **backplane** is created – a set of side channels (one per actor).
2. Each actor’s `run()` function detects `use_internal_behavior == false` and enters **simulated behavior**.
3. The `StageManager` can **send commands** to an actor via its side channel, and the actor responds automatically (echoing, waiting, etc.).

### Example: Testing a consumer with a StageManager

```rust
use steady_state::*;

#[test]
fn test_consumer_with_stage() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder()
        .with_capacity(10)
        .build_channel::<Widget>();

    graph.actor_builder()
        .with_name("consumer")
        .build(move |context| consumer_behavior(context, rx.clone()), SoloAct);

    graph.start();

    let stage = graph.stage_manager();

    // Send a message via the backplane to be echoed to the output
    stage.actor_perform("consumer", StageDirection::Echo(Widget { id: 1 }))?;

    // Wait for a specific message to appear in the output
    stage.actor_perform("consumer", StageWaitFor::Message(Widget { id: 2 }, Duration::from_secs(2)))?;

    stage.final_bow(); // release the lock
    graph.block_until_stopped(Duration::from_secs(5))?;

    Ok(())
}
```

### StageManager API

| Method | Description |
|--------|-------------|
| `actor_perform(name, action)` | Send a `StageDirection` or `StageWaitFor` to an actor. |
| `actor_perform_with_suffix(name, suffix, action)` | Same but target a specific suffix instance. |
| `final_bow()` | Release the side‑channel lock (required after using the stage). |

### Supported Stage Actions

- **`StageDirection::Echo(value)`** – Send a message and expect it to be echoed back.
- **`StageDirection::EchoAt(index, value)`** – Echo to a specific bundle lane.
- **`StageWaitFor::Message(value, timeout)`** – Wait until the actor produces a matching message.
- **`StageWaitFor::MessageAt(index, value, timeout)`** – Wait on a specific lane of a bundle.

---

## Testing State Persistence

When using `SteadyState` to hold actor state across restarts, you can inspect the final state after the graph stops:

```rust
let state = my_state.try_lock_sync().expect("state should be available");
assert_eq!(state.counter, expected_value);
```

---

## Testing Common Patterns

### Testing Shutdown Voting

```rust
#[test]
fn test_actor_vetoes_shutdown() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    graph.actor_builder()
        .with_name("vetoer")
        .build(|mut actor| {
            Box::pin(async move {
                // Veto shutdown by returning false
                while actor.is_running(|| false) {
                    actor.wait(Duration::from_millis(10)).await;
                }
                Ok(())
            })
        }, SoloAct);

    graph.start();
    graph.request_shutdown();

    let result = graph.block_until_stopped(Duration::from_millis(100));
    assert!(result.is_err()); // unclean shutdown because of veto
    Ok(())
}
```

### Testing Backpressure

Create a channel with capacity `1`, fill it, then try to send again. The actor should block until space is freed:

```rust
#[test]
fn test_backpressure() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder()
        .with_capacity(1)
        .build_channel::<u8>();

    graph.actor_builder()
        .with_name("producer")
        .build(|mut actor| {
            Box::pin(async move {
                let mut tx = tx.lock().await;
                let mut rx = rx.lock().await;
                // First send succeeds
                let _ = actor.send_async(&mut tx, 1, SendSaturation::AwaitForRoom).await;
                // Second send blocks; we must consume it to return
                if let Some(_) = actor.take_async(&mut rx).await {
                    // Room became available, now send second
                    let _ = actor.send_async(&mut tx, 2, SendSaturation::AwaitForRoom).await;
                }
                actor.request_shutdown().await;
                Ok(())
            })
        }, SoloAct);

    graph.start();
    graph.block_until_stopped(Duration::from_secs(5))?;
    assert_steady_rx_eq_take!(&rx, vec![1, 2]);
    Ok(())
}
```

---

## Common Pitfalls

| Issue | Cause | Fix |
|-------|-------|-----|
| Test hangs | Calling `run()` instead of `internal_behavior` | Always call `internal_behavior` in unit tests. |
| Test times out | Missing `mark_closed()` in veto closure | Include `tx.mark_closed()` in `is_running` closure. |
| Data appears incomplete | Channels not flushed before shutdown | Wait for `block_until_stopped` before reading output. |
| State not initialized | `SteadyState` lock not called | Use `state.lock(|| MyState::default()).await` before use. |

---

## Next Steps

- Read the **[Telemetry Guide](telemetry.md)** to understand how to configure and interpret metrics.
- Explore the **[Distributed Systems Guide](distributed.md)** for testing Aeron pipelines.
- Check the **[Migration Guide](migration_guide.md)** when upgrading between versions.

---

**Reference**: For a deeper understanding of the actor lifecycle, shutdown voting, and channel configuration, see the [steady_state_tldr.md](../steady_state_tldr.md).
