# Actor Lifecycle in Steady State

This guide explains how actors are created, run, restarted, and shut down in **Steady State**.  
Understanding the lifecycle is essential for writing robust actors that handle failures gracefully.

---

## Overview

Every Steady State actor goes through these stages:

1. **Building** – The graph is constructed and actors are registered.
2. **Registration** – Each actor is added to the liveliness state as a voter.
3. **Running** – The actor executes its `internal_behavior` loop.
4. **Shutdown Voting** – When the graph requests shutdown, each actor votes.
5. **Stopped** – All actors have agreed to stop; the graph is shut down.

During the **Running** phase, if an actor panics or returns an error, it is automatically **restarted** (with an incremented `regeneration` counter). Data on channels is never lost.

---

## The `run()` / `internal_behavior()` Split

Every actor **must** provide a `run()` function that acts as a dispatcher, and an `internal_behavior()` function that contains the actual logic.

### `run()` – The Dispatcher

```rust
pub async fn run(
    context: SteadyActorShadow,
    rx: SteadyRx<MyMsg>,
    tx: SteadyTx<MyReply>,
) -> Result<(), Box<dyn Error>> {
    let actor = context.into_spotlight([&rx], [&tx]);

    if actor.use_internal_behavior {
        internal_behavior(actor, rx, tx).await
    } else {
        actor.simulated_behavior(sim_runners!(rx, tx)).await
    }
}
```

- **`context.into_spotlight(...)`** converts the shadow into a spotlight, setting up telemetry.
- **`use_internal_behavior`** is `true` for production and for unit tests that call `internal_behavior` directly. It is `false` only when the actor is being driven by a `StageManager` in integration tests.
- **`simulated_behavior`** is a test‑only mode where the actor does not process real work; instead it responds to side‑channel commands.

### `internal_behavior()` – Your Logic

```rust
async fn internal_behavior<C: SteadyActor>(
    mut actor: C,
    rx: SteadyRx<MyMsg>,
    tx: SteadyTx<MyReply>,
) -> Result<(), Box<dyn Error>> {
    // LOCK ALL CHANNELS FIRST
    let mut rx = rx.lock().await;
    let mut tx = tx.lock().await;

    while actor.is_running(|| rx.is_closed_and_empty() && tx.mark_closed()) {
        let clean = await_for_all!(
            actor.wait_avail(&mut rx, 1),
            actor.wait_vacant(&mut tx, 1)
        );

        if let Some(msg) = actor.try_take(&mut rx) {
            let reply = process(msg);
            let _ = actor.try_send(&mut tx, reply);
        }
    }
    Ok(())
}
```

Key points:

- **Lock upfront** – Call `.lock().await` on every channel **once** at the start of the function. Never lock inside the loop.
- **`actor.is_running(|| ...)`** – Returns `true` while the graph is running. The closure is the **veto closure** – it is called only when a shutdown is requested, and its return value decides the actor’s vote.
- **Single wake point** – Use `await_for_all!` or `await_for_all_or_proceed_upon!` to consolidate all wait conditions.
- **No scattered awaits** – After the consolidated wait, process messages without further blocking awaits.

---

## Shutdown Voting (The Veto Pattern)

Shutdown is **negotiated**, not forced. When the graph transitions to `StopRequested`, every actor’s `is_running` loop calls the veto closure. The closure must return:

- **`true`** → “I agree to shut down now.”
- **`false`** → “I veto – I still have work to do.”

```rust
while actor.is_running(|| {
    // Veto: keep running if there are still messages in the inbound channel
    // or we have not yet closed our outbound channel.
    rx.is_closed_and_empty() && tx.mark_closed()
}) {
    // ...
}
```

### Common Veto Conditions

| Condition                      | Why                                                              |
|--------------------------------|------------------------------------------------------------------|
| `rx.is_closed_and_empty()`     | The input channel is closed and empty (no more data incoming).   |
| `tx.mark_closed()`             | The output channel is closed (no more data outgoing).            |
| Custom state check             | E.g., drain an internal buffer before stopping.                  |

If an actor vetoes too long, the graph will eventually timeout and stop **uncleanly** (an error is returned from `block_until_stopped`). The timeout is set by the argument to `block_until_stopped(Duration)`.

### Debugging Vetoes with `i!()`

Wrap each condition in the `i!()` macro to log which expression vetoed:

```rust
while actor.is_running(|| i!(rx.is_closed_and_empty()) && i!(tx.mark_closed())) { }
```

When shutdown fails, telemetry prints the file and line of the first `false` condition.

---

## Restart on Failure

Actors are automatically restarted if they panic or return `Err`. The `regeneration()` method returns how many times the actor has been restarted.

```rust
fn regeneration(&self) -> u32;
```

- The restart creates a **new** `SteadyActorShadow` (with a fresh `oneshot_shutdown` future).
- Channels (locks) are **not** reset – data is preserved.
- The `regeneration` count is incremented on each restart.

> **Important**: If an actor panics repeatedly, it will keep restarting. There is no back‑off by default. Use the `disable_actor_restart_on_failure` feature or a custom supervisor if needed.

---

## The `run()` / `is_running()` Loop

The standard loop structure in `internal_behavior`:

```
while actor.is_running(veto_closure) {
    // 1. Consolidated wait (await_for_all!)
    // 2. Take message(s)
    // 3. Process
    // 4. Send result(s)
    // 5. (Optional) relay telemetry with relay_stats_smartly()
}
```

- **`is_running`** is called **at the beginning** of each iteration.
- The veto closure is **only** called when the graph is in `StopRequested`.
- If the actor is still in the `Building` state, `is_running` yields and returns `true`.

---

## Telemetry During Lifecycle

- **`relay_stats_smartly()`** – Sends telemetry if enough time has passed since the last send (non‑blocking, returns `bool`).
- **`relay_stats()`** – Forces an immediate send.
- **`relay_stats_periodic(duration)`** – Waits for the given duration and then sends telemetry.

Telemetry is automatically sent before the actor exits (in the `Drop` impl of `SteadyActorSpotlight`), so final data is not lost.

---

## Testing the Lifecycle

### Unit Tests: Call `internal_behavior` Directly

```rust
#[test]
fn test_my_actor_shutdown() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx_in, rx_in) = graph.channel_builder().with_capacity(10).build_channel::<MyMsg>();
    let (tx_out, rx_out) = graph.channel_builder().with_capacity(10).build_channel::<MyReply>();

    graph.actor_builder()
        .with_name("my_actor")
        .build(move |context| {
            internal_behavior(context, rx_in.clone(), tx_out.clone())
        }, SoloAct);

    graph.start();

    // Inject data, then close the input to trigger shutdown
    tx_in.testing_send_all(vec![msg1, msg2], true);
    graph.request_shutdown();
    graph.block_until_stopped(Duration::from_secs(5))?;

    // Verify output
    assert_steady_rx_eq_count!(&rx_out, 2);
    Ok(())
}
```

### Integration Tests: `StageManager`

Use `stage_manager()` to drive an actor from the outside. See the [Testing Guide](testing.md) for details.

---

## Summary

| Concept                 | Implementation                                                                |
|-------------------------|-------------------------------------------------------------------------------|
| Actor creation          | `GraphBuilder::for_production()`, `.actor_builder()`, `.build(f, SoloAct)`   |
| Execution loop          | `internal_behavior` with `while actor.is_running(veto) { ... }`              |
| Shutdown negotiation    | Veto closure in `is_running()`                                               |
| Restart                 | Automatic on panic/error; `regeneration()` returns count                      |
| Telemetry               | `relay_stats_smartly()` inside the loop                                      |
| Testing                 | Call `internal_behavior` directly; use `testing_send_all` and `assert_steady_rx_eq_count!` |

---

**Next**: [Channel Configuration Guide](channels.md) – Learn about backpressure, bundles, lazy initialization, and stream channels.
