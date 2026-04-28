# Channels in Steady State

Channels are the communication backbone of any Steady State actor graph. This guide explains how to create, configure, and use channels effectively.

---

## Overview

A channel connects a **transmitter** (Tx) to a **receiver** (Rx). Messages travel from one actor to another through the channel. Steady State channels are **unbounded** in the sense that they are fixed‑capacity ring buffers, but they **never lose data** – if the receiver is down or slow, the sender blocks or backs off.

Every channel consists of:

- A **`Tx<T>`** – the sending endpoint.
- An **`Rx<T>`** – the receiving endpoint.

Channels can be either **eager** (allocated immediately) or **lazy** (allocated on first use). They can be combined into **bundles** to fan out to multiple actors.

---

## Creating a Channel

The simplest way to create a channel is through the `ChannelBuilder` obtained from a `Graph`:

```rust
let mut graph = GraphBuilder::for_testing().build(());

let channel_builder = graph.channel_builder();

let (tx, rx) = channel_builder
    .with_capacity(256)
    .build_channel::<MyMessage>();
```

`build_channel()` returns a pair of **lazy** wrappers (`LazySteadyTx<T>`, `LazySteadyRx<T>`). The underlying ring buffer is allocated only when you call `.clone()` on the lazy wrapper (which happens automatically inside actor code).

---

## Lazy vs Eager Channels

### Lazy Channels (Recommended)

```rust
let (tx_lazy, rx_lazy) = builder.build_channel::<MyMsg>();
```

- **Allocation is deferred** until the channel is actually used.
- Cloning a `LazySteadyTx` or `LazySteadyRx` triggers allocation.
- This is the **default and preferred** approach because it avoids wasting memory for channels that may never be used (e.g., due to actor panics or dynamic topology).

### Eager Channels (Test / Internal)

```rust
let (tx, rx) = builder.eager_build::<MyMsg>();        // Returns SteadyTx / SteadyRx
let (tx_raw, rx_raw) = builder.eager_build_internal(); // Returns raw Tx / Rx structs
```

- The ring buffer is allocated immediately.
- `eager_build` returns `Arc<Mutex<Tx<T>>>` / `Arc<Mutex<Rx<T>>>`.
- Used internally by the framework and useful for testing.

---

## Channel Families

### Standard Channels: `Tx<T>` / `Rx<T>`

Used for simple point‑to‑point message passing. Messages are moved (or copied if you use slice methods).

### Stream Channels: `StreamTx<T>` / `StreamRx<T>`

Used for streaming binary data together with control items. They consist of **two** internal ring buffers:

- **Control channel** – holds the control item (e.g., `StreamEgress` or `StreamIngress`).
- **Payload channel** – holds the raw bytes.

```rust
let (tx, rx) = channel_builder
    .with_capacity(100)
    .build_stream::<StreamEgress>(1024);  // 1024 bytes per item
```

Stream channels are essential for Aeron‑based distributed communication.

---

## Channel Capacity

Set the capacity (maximum number of items) via `with_capacity`:

```rust
channel_builder.with_capacity(10_000)
```

The default is **64**. Choose a capacity large enough to absorb bursts, but not so large that it wastes memory. A good rule of thumb:

- **Low‑latency** actors: 64 – 256.
- **Bursty** actors: 1 000 – 10 000.
- **Aeron streams** 4 K – 1 M items (payload capacity is computed separately).

---

## Backpressure

Backpressure is built into the channel model. The sender's `wait_vacant` method blocks (async) until space is available. The receiver's `wait_avail` blocks until messages are ready.

The key invariants:

- A sender **never** overwrites unread messages.
- A receiver **never** reads a message that hasn't been fully written.
- Channels are **lock‑free** for the fast path (ring buffer) with an `await` path for when the channel is full/empty.

---

## Bundles

A **bundle** is an array of channels that share the same type and capacity. Bundles allow fan‑out (one sender to many receivers) or fan‑in (many senders to one receiver).

```rust
use steady_state::macros::steady_tx_bundle;
// Build a bundle of 4 channels
let (tx_bundle, rx_bundle) = channel_builder
    .build_channel_bundle::<Packet, 4>();
// tx_bundle: [LazySteadyTx<Packet>; 4]
// rx_bundle: [LazySteadyRx<Packet>; 4]
```

To lock all channels in a bundle at once:

```rust
let guards = tx_bundle.lock().await;
```

Bundle operations in the `SteadyActor` trait:

- `wait_vacant_bundle(&mut tx_bundle, count, ready_channels)` – waits until `ready_channels` of the bundle lanes have `count` vacant units.
- `wait_avail_bundle(&mut rx_bundle, count, ready_channels)` – waits until `ready_channels` lanes have `count` units available.

---

## Telemetry Configuration

You can attach rich telemetry to channels:

```rust
channel_builder
    .with_avg_filled()                         // Average fill level
    .with_avg_rate()                           // Average throughput
    .with_avg_latency()                        // Average latency
    .with_filled_percentile(Percentile::p95()) // Fill distribution
    .with_rate_trigger(Trigger::AvgAbove(Rate::per_seconds(1000)), AlertColor::Red)
    .with_latency_trigger(Trigger::AvgAbove(Duration::from_millis(50)), AlertColor::Yellow);
```

These metrics appear in the DOT graph (edge labels) and in Prometheus output.

To see telemetry on a channel, you must **enable telemetry features** in the graph:

```rust
GraphBuilder::for_production()
    .with_telemetry_metric_features(true) // Enables both built‑in server and prometheus
```

And you must set a non‑zero `frame_rate_ms` (default is `100`).

---

## Channel Metadata

Each channel has a unique `id` (assigned at creation) and optional labels:

```rust
channel_builder
    .with_labels(&["order_flow", "priority"], true);
```

Labels appear in the DOT graph and are used in Prometheus metric names.

---

## Marking Channels as Closed

When an actor finishes its work, it **must** close its outgoing channels by calling `mark_closed()` in its veto closure:

```rust
while actor.is_running(|| rx.is_closed_and_empty() && tx.mark_closed()) {
    // ...
}
```

Closing a channel signals to downstream actors that no more data will arrive. The `mark_closed()` call is **idempotent** and safe to call multiple times.

---

## Stream Channels (Deep Dive)

Stream channels are used when you need to send binary payloads alongside control items. The control item carries a **length** field that tells the receiver how many bytes to expect.

### Egress Stream (Sending Data)

```rust
// Sender side
let (tx, rx) = channel_builder.build_stream::<StreamEgress>(bytes_per_item);

// To send a message:
let (item, payload) = StreamEgress::build(&data);   // Returns (StreamEgress, Box<[u8]>)
actor.send_async(&mut tx, (item, &payload[..]), SendSaturation::AwaitForRoom).await;
```

### Ingress Stream (Receiving Data)

```rust
// Receiver side
let mut rx_guard = rx.lock().await;
if let Some((item, payload)) = rx_guard.shared_try_take() {
    // item is StreamIngress, payload is Box<[u8]>
}
```

Stream channels are the foundation for Aeron publish/subscribe actors.

---

## Common Anti‑Patterns

| Anti‑Pattern | Why It’s Wrong | Correct Approach |
|---|---|---|
| `tx.blocking_send()` (if available) | Can deadlock the executor | Use `send_async` with `SendSaturation::AwaitForRoom` |
| Not calling `mark_closed()` | Downstream actors hang forever | Always include `tx.mark_closed()` in the veto closure |
| Using `eager_build` in production | Wastes memory for inactive channels | Use `build_channel` (lazy) |
| Forgetting to lock before operations | Panic: “Send called after channel marked closed” | Always call `.lock().await` at the beginning of `internal_behavior` |

---

## Configuration Reference

| Method | Description |
|---|---|
| `with_capacity(n)` | Max number of items |
| `with_labels(&["a","b"], true)` | Assign labels for telemetry |
| `with_type()` | Show the type name in DOT labels |
| `with_avg_filled()` | Show rolling average fill % |
| `with_avg_rate()` | Show rolling average throughput (per sec) |
| `with_avg_latency()` | Show rolling average latency (µs) |
| `with_filled_percentile(p)` | Show a percentile of fill distribution |
| `with_rate_percentile(p)` | Show a percentile of throughput distribution |
| `with_latency_percentile(p)` | Show a percentile of latency distribution |
| `with_filled_standard_deviation(s)` | Show std dev of fill |
| `with_rate_standard_deviation(s)` | Show std dev of rate |
| `with_filled_trigger(condition, color)` | Alert when fill exceeds a threshold |
| `with_rate_trigger(condition, color)` | Alert when rate exceeds a threshold |
| `with_latency_trigger(condition, color)` | Alert when latency exceeds a threshold |
| `with_no_refresh_window()` | Disable telemetry rolling window (set bits to 0) |
| `with_compute_refresh_window_floor(refresh, window)` | Fine‑tune telemetry window sizing |

---

**Next**: [Testing Guide](testing.md) – Learn how to test your actors using `internal_behavior` directly and the `StageManager`.
