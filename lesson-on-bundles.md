┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Advanced Guide: Mastering Bundled Streams in Steady State                                                                                                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

Bundled streams are the architectural backbone for scaling Steady State applications. While a single stream provides a robust pipe for control and payload
data, a Bundle allows an actor to treat a collection of identical streams as a single logical unit. This is essential for implementing patterns like
Fan-out/Fan-in, Sharding, and Parallel Processing.

This guide provides a deep dive into the lifecycle of a bundle, from definition in main.rs to execution and shutdown.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

1. The Blueprint: Defining Bundles in main.rs

In Steady State, bundles are defined during the graph construction phase. The ChannelBuilder is used to specify the "Girth"—the number of parallel streams.

How to Build a Bundle


const WORKER_COUNT: usize = 8;
const AVG_MSG_SIZE: usize = 512;

let (tx_bundle, rx_bundle) = graph.channel_builder()
    .with_capacity(2000) // Capacity per individual stream
    .with_avg_rate()     // Enable telemetry for throughput
    .build_stream_bundle::<StreamIngress, WORKER_COUNT>(AVG_MSG_SIZE);


Why this matters:

 • Memory Efficiency: By providing bytes_per_item (e.g., 512), the builder automatically calculates the payload channel's capacity. If the control channel
   holds 2,000 items, the payload channel will hold $2,000 \times 512$ bytes.
 • Lazy Initialization: build_stream_bundle returns Lazy handles. This means the actual ring buffers aren't allocated until the actor starts, ensuring
   memory is allocated on the NUMA node or CPU socket where the actor is actually running.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

2. The Actor Entry Point: The run Function

Actors handling bundles must be generic over the GIRTH. This allows the same actor logic to be reused regardless of whether you are running 2 parallel
streams or 200.

Idiomatic Signature


pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    rx: SteadyStreamRxBundle<StreamIngress, GIRTH>,
    tx: SteadyStreamTx<StreamEgress>, // Example: Fan-in to a single output
) -> Result<(), Box<dyn Error>> {
    // 1. Convert to Spotlight
    // 2. Lock the bundle
    // 3. Enter the loop
}


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

3. The Spotlight: Telemetry and Monitoring

When you move from a SteadyActorShadow to a SteadyActorSpotlight, you are telling the framework which channels to watch.

How to use into_spotlight with Bundles


// Idiomatic: Monitor the control metadata
let mut actor = context.into_spotlight(rx.control_meta_data(), [&tx]);


Why this matters:

 • Control vs. Payload: A stream bundle has two sets of metadata. .control_meta_data() tracks the number of messages (items). .payload_meta_data() tracks
   the number of bytes.
 • Lesson: In 99% of cases, you want to monitor the control metadata. Seeing "10,000 items/sec" in your telemetry dashboard is much more actionable for
   debugging logic than seeing "5.2 MB/sec."

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

4. The Lifecycle Loop: is_running

The is_running loop is the heartbeat of a Steady State actor. For bundles, the shutdown predicate must be comprehensive.

The Shutdown Predicate


let mut rx_guards = rx.lock().await;
let mut tx_guard = tx.lock().await;

while actor.is_running(&mut || {
    i!(rx_guards.is_closed_and_empty()) && i!(tx_guard.mark_closed())
}) {
    // Work happens here
}


Why this matters:

 • Parallel Drain: rx_guards.is_closed_and_empty() is a trait method from StreamRxBundleTrait. It returns true only if every single stream in the bundle is
   closed and has zero remaining messages.
 • The i! Macro: Always wrap your conditions in the i! macro. If the actor hangs and refuses to shut down, Steady State telemetry will report exactly which
   part of the expression (e.g., which specific stream in the bundle) evaluated to false, preventing "zombie actors."

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

5. Orchestration: Waiting on Bundles

One of the most common mistakes is polling a bundle in a manual loop. This wastes CPU. Instead, use the specialized bundle-wait methods.

wait_avail_bundle and wait_vacant_bundle

These methods take a ready_channels parameter.


// Scenario: We want to process data as soon as ANY stream has a batch ready.
let batch_size = 500;
let clean = actor.wait_avail_bundle(&mut rx_guards, batch_size, 1).await;

// Scenario: We are a synchronizer and need ALL streams to have room before we proceed.
let clean = actor.wait_vacant_bundle(&mut tx_guards, (1, 1), GIRTH).await;


Why this matters:

 • Cooperative Scheduling: These methods allow the actor to yield its execution thread to other actors until the specific condition is met.
 • Lesson: Setting ready_channels to 1 is ideal for Work Stealing. Setting it to GIRTH is ideal for Batch Synchronization.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

6. Data Processing Patterns

Pattern: Round-Robin Consumption

To prevent a single high-volume stream from starving others in the bundle, iterate with a rotating start index.


let mut next_idx = 0;
// ... inside loop ...
for i in 0..GIRTH {
    let idx = (next_idx + i) % GIRTH;
    let (avail_items, avail_bytes) = actor.avail_units(&mut rx_guards[idx]);

    if avail_items > 0 {
        // Process items from rx_guards[idx]
    }
}
next_idx = (next_idx + 1) % GIRTH;


Pattern: Key-Based Routing (Fan-out)

If you are distributing work to a bundle, use a deterministic hash to ensure related data stays on the same stream (preserving order for specific keys).


let target_idx = (data.user_id as usize) % GIRTH;
actor.send_async(&mut tx_guards[target_idx], data, SendSaturation::WarnThenAwait).await;


────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

7. Distributed Bundles (Aeron Aqueducts)

Bundles are the primary way to interface with Aeron. When you build an aqueduct for a bundle, Steady State handles the complexity of stream ID management.

How to Build an Aqueduct


rx_bundle.build_aqueduct(
    AqueTech::Aeron(aeron_channel, 1000), // Base Stream ID 1000
    &graph.actor_builder().with_name("AeronSubscriber"),
    ScheduleAs::SoloAct
);


Why this matters:

 • Automatic Offsetting: If GIRTH is 4, Steady State will automatically subscribe to Aeron Stream IDs 1000, 1001, 1002, and 1003.
 • Transparency: Your actor logic remains identical whether the data is coming from a local memory bundle or a distributed Aeron bundle.

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

8. Best Practices Checklist

 1 Girth Selection: Match your GIRTH to the number of physical CPU cores available if the actor is CPU-bound.
 2 Locking: Always call .lock().await outside the while actor.is_running loop if possible, or at the very top of the loop. Never lock/unlock individual
   members of a bundle inside a tight processing loop.
 3 Batching: Bundles are designed for high volume. Use take_slice and send_slice to move data in blocks. The overhead of a bundle is amortized over the size
   of the batch.
 4 Telemetry: If a bundle member is consistently full while others are empty, your routing logic (the code choosing the index) is skewed. Use the graph.dot
   visualization to spot these "hot" streams.
 5 Simulation: In unit tests, use actor.simulated_behavior. It will automatically handle the complexity of mocking all GIRTH streams in the bundle.

#################


┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
┃ Appendix: Common Patterns and Examples for Stream Bundles                                                                                                ┃
┗━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┛

1. The Sharded Producer (Key-Based Fan-out)

Why: Ensures that all data related to a specific ID (like a user_id) is processed by the same downstream stream to maintain strict sequential ordering for
that ID.


// Inside internal_behavior
let mut tx_bundle = tx_bundle.lock().await;

while actor.is_running(&mut || i!(tx_bundle.mark_closed())) {
    // Wait until at least one stream has room for 1 message
    let _clean = await_for_all!(actor.wait_vacant_bundle(&mut tx_bundle, (1, 1), 1));

    if let Some(data) = get_next_packet() {
        let target_idx = (data.user_id as usize) % GIRTH;
        // Use try_send because we already verified vacancy
        let _ = actor.try_send(&mut tx_bundle[target_idx], data.payload);
    }
}


2. The Work-Stealing Consumer (Fan-in)

Why: Efficiently aggregates data from multiple upstream sources. The actor wakes up as soon as any stream has a significant batch ready.


let mut rx_bundle = rx_bundle.lock().await;

while actor.is_running(&mut || i!(rx_bundle.is_closed_and_empty())) {
    // Wake up when ANY (1) stream has at least 100 messages
    let _clean = await_for_all!(actor.wait_avail_bundle(&mut rx_bundle, 100, 1));

    for rx in rx_bundle.iter_mut() {
        // Drain everything currently available in this specific stream
        while let Some((item, payload)) = actor.try_take(rx) {
            process(item, payload);
        }
    }
}


3. The Barrier Synchronizer (Join)

Why: Used when an actor requires one piece of data from every input stream before it can perform a calculation (e.g., a parallel zip or join).


let mut rx_bundle = rx_bundle.lock().await;

while actor.is_running(&mut || i!(rx_bundle.is_closed_and_empty())) {
    // Wake up only when ALL (GIRTH) streams have at least 1 item
    let _clean = await_for_all!(actor.wait_avail_bundle(&mut rx_bundle, 1, GIRTH));

    if _clean {
        for i in 0..GIRTH {
            if let Some((meta, data)) = actor.try_take(&mut rx_bundle[i]) {
                sync_process(i, meta, data);
            }
        }
    }
}


4. The Round-Robin Dispatcher

Why: Distributes work evenly across a bundle to maximize throughput when message ordering between different keys does not matter.


let mut tx_bundle = tx_bundle.lock().await;
let mut next_worker = 0;

while actor.is_running(&mut || i!(tx_bundle.mark_closed())) {
    if let Some(work) = source.try_take() {
        let target = next_worker % GIRTH;
        // Wait for the specific target to have room
        if actor.wait_vacant(&mut tx_bundle[target], (1, 1)).await {
            let _ = actor.try_send(&mut tx_bundle[target], work);
            next_worker = (next_worker + 1) % GIRTH;
        }
    }
}


5. The Zero-Copy Batch Processor

Why: The fastest way to process data. It avoids the overhead of try_take by processing the internal ring-buffer slices directly.


let mut rx_bundle = rx_bundle.lock().await;

for i in 0..GIRTH {
    // Access the raw internal slices (quadruple slice for streams)
    let (item_a, item_b, payload_a, payload_b) = actor.peek_slice(&mut rx_bundle[i]);

    // Process the first contiguous chunk
    for (item, byte_chunk) in item_a.iter().zip(payload_a.chunks(item_a[0].length() as usize)) {
        ultra_fast_process(item, byte_chunk);
    }

    // Manually advance the index to "consume" the data
    actor.advance_take_index(&mut rx_bundle[i], (item_a.len(), payload_a.len()));
}


6. The Clean Shutdown Veto

Why: Prevents the actor from shutting down prematurely if there is still data in flight in any of the parallel streams.


let mut rx_bundle = rx_bundle.lock().await;
let mut tx_bundle = tx_bundle.lock().await;

while actor.is_running(&mut || {
    // Veto shutdown if ANY stream is not empty
    let rx_ready = rx_bundle.is_closed_and_empty();
    // Signal downstream that we are closing
    let tx_ready = tx_bundle.mark_closed();

    i!(rx_ready) && i!(tx_ready)
}) {
    // Loop continues until all streams are empty and closed
}


7. The Multi-Bundle Spotlight

Why: Correctly configures telemetry for an actor that bridges two different bundles (e.g., a sharded-to-sharded transformer).


pub async fn run<const IN: usize, const OUT: usize>(
    context: SteadyActorShadow,
    rx: SteadyStreamRxBundle<StreamIngress, IN>,
    tx: SteadyStreamTxBundle<StreamEgress, OUT>,
) -> Result<(), Box<dyn Error>> {
    // Spotlight both bundles using their control metadata arrays
    let mut actor = context.into_spotlight(
        rx.control_meta_data(),
        tx.control_meta_data()
    );
    // ...
}


8. The High-Availability Broadcast

Why: Sends the exact same message to every stream in a bundle, typically used for replicating state or sending control commands to all workers.


let mut tx_bundle = tx_bundle.lock().await;

while actor.is_running(&mut || i!(tx_bundle.mark_closed())) {
    // Wait until EVERY stream has room for 1 message
    let _clean = await_for_all!(actor.wait_vacant_bundle(&mut tx_bundle, (1, 1), GIRTH));

    if let Some(cmd) = control_source.try_take() {
        for i in 0..GIRTH {
            let _ = actor.try_send(&mut tx_bundle[i], cmd.clone());
        }
    }
}


9. The "Hot Stream" Telemetry Check

Why: Uses relay_stats_smartly to ensure the graph.dot visualization stays updated. If one stream is red and others are green, your sharding logic is
unbalanced.


while actor.is_running(&mut || i!(rx_bundle.is_closed_and_empty())) {
    // ... process data ...

    // Periodically push metrics to the collector
    actor.relay_stats_smartly();
}


10. The Lazy Memory Pattern

Why: Demonstrates how to define a bundle in main.rs without actually allocating the memory until the actor starts.


// In main.rs
let (tx, rx) = graph.channel_builder()
    .with_capacity(5000)
    .build_stream_bundle::<StreamIngress, 16>(256);

// The 16 x 5000-item buffers are NOT allocated yet.
// Allocation happens inside the actor's thread when it calls:
// let mut rx_guards = rx.lock().await;


11. The Weighted Priority Consumer

Why: Processes data from a bundle but gives more "attention" to the first few streams (e.g., high-priority queues).


let mut rx_bundle = rx_bundle.lock().await;

while actor.is_running(&mut || i!(rx_bundle.is_closed_and_empty())) {
    let _ = actor.wait_avail_bundle(&mut rx_bundle, 1, 1).await;

    // Check high priority (index 0) more frequently
    if let Some(msg) = actor.try_take(&mut rx_bundle[0]) {
        process_high_priority(msg);
        continue; // Jump back to top to check index 0 again
    }

    // Otherwise, check the rest
    for i in 1..GIRTH {
        if let Some(msg) = actor.try_take(&mut rx_bundle[i]) {
            process_normal(msg);
        }
    }
}


12. The Bundle Unit Test (Simulator)

Why: How to write a test for a bundle-based actor using the built-in simulation engine.


#[test]
fn test_bundle_actor() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().build_stream_bundle::<StreamEgress, 4>(100);

    graph.actor_builder().with_name("Test").build(move |actor| {
        let rx_bundle = rx.clone();
        async move {
            let mut actor = actor.into_spotlight(rx_bundle.control_meta_data(), []);
            // Map the bundle into a Vec of trait objects for the simulator
            let sims: Vec<_> = rx_bundle.iter().map(|s| s as &dyn IntoSimRunner<_>).collect();
            actor.simulated_behavior(sims).await
        }
    }, SoloAct);
}

