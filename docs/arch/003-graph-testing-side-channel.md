# ADR 003: Graph testing side channel

## Context

Full-graph tests need to inject and observe messages without rewriting production actors. Calling `run()` under `GraphBuilder::for_testing()` enters `simulated_behavior` and waits for StageManager — unit tests that only use `testing_send_all` hang.

## Decision

- **`graph_testing`** is a **public** module: `StageManager` / side channels are part of the supported test API.
- **Integration tests**: build the real graph, `stage.actor_perform(...)`, then `final_bow` before shutdown.
- **Unit tests**: call `internal_behavior` directly; never `run()`.

## Consequences

Side-channel commands (`Echo`, `WaitFor::Message`) validate wiring. Logic bugs belong in `internal_behavior` tests. Large StageManager suites live in `graph_testing_tests.rs` / `graph_testing_proptest.rs` so the production module stays small.
