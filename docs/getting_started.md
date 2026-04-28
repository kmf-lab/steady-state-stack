# Getting Started with Steady State

Welcome to **Steady State**, a high‑performance, telemetry‑first actor framework for building resilient concurrent systems in Rust.

This guide walks you through your first actor graph — from installation to seeing live telemetry.

---

## Prerequisites

- Rust **1.75+** (stable)
- Cargo
- (Optional) An Aeron media driver if you plan to use distributed features.

---

## Installation

Add Steady State to your `Cargo.toml`:

```toml
[dependencies]
steady_state = "0.2"
```

Enable one executor feature (choose the appropriate one for your platform):

| Feature              | Description                          |
|----------------------|--------------------------------------|
| `proactor_nuclei`    | Linux io_uring (high throughput)     |
| `proactor_tokio`     | Cross‑platform Tokio backend         |
| `exec_async_std`     | Lightweight async‑std backend        |

Example:

```toml
[dependencies]
steady_state = { version = "0.2", features = ["proactor_nuclei"] }
```

> **Windows users**: Use `exec_async_std` because io_uring is not supported.

---

## Your First Actor

### Step 1: Define a message type

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Greeting(pub String);
```

### Step 2: Write the actor’s internal behavior

```rust
use steady_state::*;

async fn greeter_behavior<C: SteadyActor>(
    mut actor: C,
    mut rx: SteadyRx<Greeting>,
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
            // Process: uppercase the greeting
            let reply = Greeting(msg.0.to_uppercase());
            let _ = actor.try_send(&mut tx, reply);
        }
    }
    Ok(())
}
```

### Step 3: Create the `run` function (the dispatcher)

```rust
pub async fn run(
    context: SteadyActorShadow,
    rx: SteadyRx<Greeting>,
    tx: SteadyTx<Greeting>,
) -> Result<(), Box<dyn Error>> {
    let actor = context.into_spotlight([&rx], [&tx]);

    if actor.use_internal_behavior {
        internal_behavior(actor, rx, tx).await
    } else {
        actor.simulated_behavior(sim_runners!(rx, tx)).await
    }
}
```

### Step 4: Wire it up in `main`

```rust
use steady_state::*;

fn main() -> Result<(), Box<dyn Error>> {
    let mut graph = GraphBuilder::for_production()
        .with_telemetry_metric_features(true)  // enable the telemetry server
        .build(());

    let (tx, rx) = graph.channel_builder()
        .with_capacity(64)
        .build_channel::<Greeting>();

    graph.actor_builder()
        .with_name("greeter")
        .build(
            move |context| run(context, rx.clone(), tx.clone()),
            ScheduleAs::SoloAct,
        );

    graph.start()?;

    // Send a test message
    tx.testing_send_all(vec![Greeting("hello".into())], true);

    graph.block_until_stopped(Duration::from_secs(5))?;
    Ok(())
}
```

---

## Running & Telemetry

1. **Build and run**:

```bash
cargo run --example your_first_actor
```

2. **Open the telemetry dashboard** (if `telemetry_metric_features` is enabled):

   The server prints a URL like `http://127.0.0.1:9900`. Open it in a browser to see live actor graphs, channel fill levels, and performance metrics.

---

## Next Steps

- Read the **[Actor Lifecycle Guide](actor_lifecycle.md)** to understand shutdown voting and restarts.
- Explore **[Channel Configuration](channels.md)** for backpressure, bundles, and lazy initialization.
- Learn how to **[Test](testing.md)** your actors using `internal_behavior` directly and the `StageManager`.

---

## Common Pitfalls

- **Forgot to lock channels** – Always call `.lock().await` at the beginning of `internal_behavior`.
- **Used `run()` in unit tests** – Call `internal_behavior` directly instead (the `run` function enters simulated mode in test builds).
- **Missing `mark_closed()`** – Always include `tx.mark_closed()` in your veto closure; otherwise the downstream actor may hang.
- **Enabled both `proactor_nuclei` and `proactor_tokio`** – Only one executor feature should be enabled at a time.

---

## Need Help?

- Read the full [TLDR](../steady_state_tldr.md) – the essential architecture document.
- Join the community: [GitHub Discussions](https://github.com/kmf-lab/steady-state-stack/discussions)
- Sponsor the project: [GitHub Sponsors](https://github.com/sponsors/kmf-lab)
