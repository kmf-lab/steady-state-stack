┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Advanced Guide: Mastering Channel Bundles in Steady State                                                                                                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

Channel bundles are the architectural backbone for scaling Steady State applications. A bundle allows an actor to treat a fixed-size collection of identical channels as a single logical unit. This is essential for implementing patterns like Fan-out/Fan-in, Sharding, and Parallel Processing.

This guide provides a deep dive into the lifecycle of a bundle, from definition in `main.rs` to execution and shutdown.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Blueprint: Defining Bundles in main.rs

In Steady State, bundles are defined during the graph construction phase. The `ChannelBuilder` is used to specify the "Girth"—the number of parallel channels.

How to Build a Bundle

```rust
const WORKER_COUNT: usize = 8;

let (tx_lazy, rx_lazy) = graph.channel_builder()
    .with_capacity(2048) // Capacity per individual channel in the bundle
    .with_avg_rate()     // Enable telemetry for throughput
    .build_channel_bundle::<MyData, WORKER_COUNT>();

// CRITICAL: Call .clone() to convert the Lazy handles into active Bundles 
// that can be passed into actors.
let tx_bundle = tx_lazy.clone();
let rx_bundle = rx_lazy.clone();
```

Why this matters:

 • Scalability: You define the parallelism (Girth) at the graph level, independent of the actor logic.
 • Lazy Initialization: `build_channel_bundle` returns Lazy handles (`LazySteadyTxBundle`, `LazySteadyRxBundle`). 
 • Activation: Calling `.clone()` on these lazy handles produces the concrete `SteadyTxBundle` / `SteadyRxBundle` required by actors. Memory for the ring buffers is allocated only when the first actor starts using these channels, ensuring optimal memory locality.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. The Actor Entry Point: The run Function

Actors handling bundles must be generic over the GIRTH. This allows the same actor logic to be reused regardless of whether you are running 2 parallel channels or 200.

Idiomatic Signature

```rust
pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    rx_bundle: SteadyRxBundle<MyData, GIRTH>,
    tx_bundle: SteadyTxBundle<MyResult, GIRTH>,
) -> Result<(), Box<dyn Error>> {
    // 1. Spotlight the bundles for telemetry
    let mut actor = context.into_spotlight(
        rx_bundle.meta_data(), 
        tx_bundle.meta_data()
    );

    // 2. Lock the bundles to get guards (Vec<MutexGuard>)
    let mut rx_guards = rx_bundle.lock().await;
    let mut tx_guards = tx_bundle.lock().await;

    // 3. Enter the loop
    while actor.is_running(&mut || {
        i!(rx_guards.is_closed_and_empty()) && i!(tx_guards.mark_closed())
    }) {
        // Work happens here
    }
    Ok(())
}
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. The Spotlight: Telemetry and Monitoring

When you move from a `SteadyActorShadow` to a `SteadyActorSpotlight`, you are "spotlighting" which channels the actor is strictly monitoring for telemetry and shutdown conditions.

How to use into_spotlight with Bundles

```rust
// Monitor the metadata for all channels in both bundles
let mut actor = context.into_spotlight(
    rx_bundle.meta_data(), 
    tx_bundle.meta_data()
);
```

Why this matters:

 • Comprehensive Monitoring: Passing the array of metadata from the bundle ensures that every individual channel is tracked.
 • Hot Path Detection: If one channel in your bundle is consistently full while others are empty, the `graph.dot` visualization will highlight the bottleneck.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. The Lifecycle Loop: is_running

The `is_running` loop is the heartbeat of a Steady State actor. For bundles, the shutdown predicate must be comprehensive.

The Shutdown Predicate

```rust
while actor.is_running(&mut || {
    i!(rx_guards.is_closed_and_empty()) && i!(tx_guards.mark_closed())
}) {
    // ...
}
```

Why this matters:

 • Parallel Drain: `rx_guards.is_closed_and_empty()` returns true only if every single channel in the bundle is closed and has zero remaining messages.
 • The i! Macro: Always wrap your conditions in the `i!` macro. If the actor hangs, the framework uses these boolean flags to report exactly which channel is "vetoing" the shutdown in telemetry.
 • Closing Downstream: `tx_guards.mark_closed()` signals to all downstream actors that no more data is coming across the entire bundle.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Orchestration: Waiting on Bundles

Efficient bundle orchestration means waiting on the *right* channel — not just knowing *that* a condition is met, but *which* channel satisfied it.

The Index-Returning Methods (Preferred)

`wait_avail_index` and `wait_vacant_index` solve the key limitation of the older API: they return the index of the first channel that satisfies its per-channel item count, so you can act immediately without scanning.

```rust
// Supply a slice of per-channel requirements. The first to satisfy its
// count wins. A count of 0 means "immediately ready" (skip).
let counts = [500; GIRTH]; // wait for 500 items on any single channel

match actor.wait_avail_index(&mut rx_guards, &counts).await {
    Some(idx) => {
        // Channel idx has 500+ items — process it directly
        let batch = actor.take_slice(&mut rx_guards[idx], &mut buffer);
    }
    None => {
        // Shutdown was signaled — exit the loop
    }
}
```

For the vacant side (sharded output):

```rust
let counts = [1000; GIRTH]; // need 1000 vacant slots on any channel

match actor.wait_vacant_index(&mut tx_guards, &counts).await {
    Some(idx) => {
        // Channel idx has room for 1000 items — send to it
        actor.send_slice(&mut tx_guards[idx], &batch);
    }
    None => { /* shutdown */ }
}
```

Dual-Time: Index + Periodic (The wait_for_index! Macro)

When you need to race an index wait against a `wait_periodic` heartbeat, the `wait_for_index!` macro bridges the two worlds by turning an `Option<usize>`-returning method into a `bool`-returning future compatible with `await_for_any!`:

```rust
let mut ready_idx: Option<usize> = None;

let _clean = await_for_any!(
    actor.wait_periodic(Duration::from_millis(500)),
    wait_for_index!(actor.wait_avail_index(&mut rx_guards, &[100; GIRTH]) => ready_idx)
);

if let Some(i) = ready_idx {
    // Channel i has 100+ items — process it
} else {
    // Periodic tick or shutdown — do housekeeping, telemetry, etc.
}
```

The `wait_for_index!` macro works identically with `wait_vacant_index`:

```rust
let mut ready_idx: Option<usize> = None;

let _clean = await_for_any!(
    actor.wait_periodic(Duration::from_millis(500)),
    wait_for_index!(actor.wait_vacant_index(&mut tx_guards, &[(1, 64); GIRTH]) => ready_idx)
);

if let Some(i) = ready_idx {
    // Channel i has room for your stream message — send it
}
```

Old Methods (Deprecated)

The older `wait_avail_bundle` and `wait_vacant_bundle` methods are deprecated. They accept a flat `ready_channels` count and only return a boolean, leaving you to manually scan for which channel is ready:

```rust
// ⚠️ Deprecated — use wait_avail_index instead
#[allow(deprecated)]
actor.wait_avail_bundle(&mut rx_guards, 500, 1).await;
```

While still functional, these should be migrated to the index-returning equivalents.

Why this matters:

 • Pinpoint Wake-Up: The index tells you *exactly* which channel is ready. No scanning loops, no speculation.
 • Per-Channel Requirements: Each channel gets its own count threshold (e.g., `[500, 100, 0, 250]`). Zero means "skip."
 • Composable: Use plain `match` for simple cases, or `wait_for_index!` + `await_for_any!` when racing against a periodic timer.
 • Shutdown-Aware: The actor methods return `Option<usize>` — `None` means shutdown was signaled, so you can exit cleanly.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Splitting and Recombining Bundles

Steady State provides macros and helpers to manipulate bundles efficiently while maintaining compile-time type safety for `GIRTH`.

Splitting a Bundle with split_bundle!

If an actor receives a large bundle and needs to delegate subsets of it, use the `split_bundle!` macro.

```rust
// Splits an Arc<[SteadyRx<T>; 10]> into two arrays of [SteadyRx<T>; 5]
let (part1, part2) = split_bundle!(rx_bundle, 5, 5);

// To treat a part as an active bundle again:
let active_sub_bundle = steady_rx_bundle_active(part1);
```

Recombining Channels

You can aggregate individual channels or smaller arrays into a larger bundle using `steady_rx_bundle_active`.

```rust
// Create a 3-channel bundle from three existing individual channels
let combined_bundle = steady_rx_bundle_active([chan_a, chan_b, chan_c]);
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

7. Advanced Patterns

Pattern: Round-Robin Consumption

To prevent a single high-volume channel from starving others, iterate with a rotating start index.

```rust
let mut next_idx = 0;
// ... inside loop ...
for i in 0..GIRTH {
    let idx = (next_idx + i) % GIRTH;
    if actor.avail_units(&mut rx_guards[idx]) > 0 {
        if let Some(msg) = actor.try_take(&mut rx_guards[idx]) {
             process(msg);
        }
    }
}
next_idx = (next_idx + 1) % GIRTH;
```

Pattern: Sharded/Key-Based Routing (Fan-out)

When distributing work, use a deterministic hash of a key to ensure related data stays on the same channel (preserving order for that key).

```rust
let target_idx = (data.user_id as usize) % GIRTH;

// Wait specifically for the target channel to have room
if actor.wait_vacant(&mut tx_guards[target_idx], 1).await {
    let _ = actor.try_send(&mut tx_guards[target_idx], data.payload);
}
```

Pattern: Parallel Barrier Join

Wait for one item from EVERY source before proceeding. The trick is to maintain a `remaining` set of indices and call `wait_avail_index` in a loop until all have been seen:

```rust
// Wait for ALL channels to have at least 1 item
let mut remaining: Vec<usize> = (0..GIRTH).collect();
while !remaining.is_empty() {
    let mut counts = [0usize; GIRTH];
    for &i in &remaining {
        counts[i] = 1;
    }
    match actor.wait_avail_index(&mut rx_guards, &counts).await {
        Some(idx) => {
            if let Some(item) = actor.try_take(&mut rx_guards[idx]) {
                process_sync(idx, item);
                remaining.retain(|&x| x != idx);
            }
        }
        None => break, // shutdown
    }
}
```

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

8. Best Practices Checklist

 1. Girth Selection: Match your `GIRTH` to the number of physical CPU cores if the actors are CPU-bound.
 2. Locking: Always call `.lock().await` outside the `while actor.is_running` loop (or at the very beginning). Never lock/unlock individual bundle members inside a tight loop.
 3. Batching: Bundles are designed for high volume. Use `take_slice` and `send_slice` to move data in blocks for maximum performance.
 4. Telemetry: Use the `graph.dot` visualization to detect skewed routing (one channel full, others empty).
 5. Simulation: In unit tests, use the `sim_runners!` macro. It automatically flattens bundles into the required collection of simulation runners for `actor.simulated_behavior`.
