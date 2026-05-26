# Channels

**Who should read this:** Anyone building graphs, configuring capacity, or testing with lazy channels.

**See also:** [Channels guide](../channels.md), `channel_builder.rs`, `channel_builder_lazy.rs`.

---

ss[channel.lazy.defer-allocation]

`build_channel` MUST return lazy wrappers whose ring buffers are not allocated until first establishment (typically via `.clone()` on the actor thread).

**Tier:** 0

---

ss[channel.lazy.establish-on-clone]

Cloning `LazySteadyTx` or `LazySteadyRx` MUST establish the channel on the cloning thread with initialized buffers and wakers.

**Tier:** 0

---

ss[channel.backpressure-never-drop]

When the receiver is slow or absent, senders MUST block or back off; the framework MUST NOT silently drop queued messages.

**Tier:** 0

---

ss[channel.default-capacity]

`ChannelBuilder` without `with_capacity` MUST use the documented default capacity (64 items unless overridden in code).

**Tier:** 1

---

ss[channel.testing-send-all]

Lazy/established channels in test graphs MUST support `testing_send_all` to inject messages and optionally close the sender side.

**Tier:** 0

---

ss[channel.testing-take-all]

Test APIs MUST support `testing_take_all` (and related helpers) to drain channels deterministically after behavior runs.

**Tier:** 0

---

ss[channel.internal-behavior-no-lazy]

`internal_behavior` signatures MUST accept established `SteadyRx` / `SteadyTx` (or bundles), not lazy blueprint types.

**Tier:** 0

---

ss[channel.stream-dual-buffer]

`build_stream` MUST allocate separate control and payload ring buffers with independent capacity semantics.

**Tier:** 1

---

ss[channel.eager-build-test]

`eager_build` / `eager_build_internal` MUST allocate immediately for tests and internal framework paths.

**Tier:** 1

---

ss[channel.memory-usage-telemetry]

`with_memory_usage` MUST enable memory-usage telemetry hooks on the builder when configured.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `channel.lazy.defer-allocation` | Deferred buffer alloc | 0 |
| `channel.lazy.establish-on-clone` | Clone establishes on thread | 0 |
| `channel.backpressure-never-drop` | No silent drops | 0 |
| `channel.default-capacity` | Default 64 | 1 |
| `channel.testing-send-all` | Test injection | 0 |
| `channel.testing-take-all` | Test drain | 0 |
| `channel.internal-behavior-no-lazy` | No lazy in behavior sig | 0 |
| `channel.stream-dual-buffer` | Stream TX/RX pair | 1 |
| `channel.eager-build-test` | Eager for tests | 1 |
| `channel.memory-usage-telemetry` | Builder memory hook | 1 |
