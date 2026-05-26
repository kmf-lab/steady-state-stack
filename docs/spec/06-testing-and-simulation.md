# Testing and simulation

**Who should read this:** Test authors and anyone using `StageManager` or sim edges.

**See also:** [Testing guide](../testing.md), `graph_testing.rs`, `simulate_edge.rs`.

---

ss[testing.never-run-in-unit]

Unit tests with `GraphBuilder::for_testing()` MUST NOT call `run()`; they MUST call `internal_behavior` directly to avoid hanging in `simulated_behavior`.

**Tier:** 0

---

ss[testing.internal-behavior-direct]

Tests MUST inject data via `testing_send_all` / take via testing helpers, then invoke `internal_behavior` with established channel clones.

**Tier:** 0

---

ss[testing.stage-manager-integration]

Integration tests MUST use `StageManager` to drive `simulated_behavior` when exercising full `run` paths.

**Tier:** 0

---

ss[testing.sim-producer-close]

Simulated producers MUST close outputs when simulated stop is requested (`SimTx` auto-close behavior).

**Tier:** 0

---

ss[testing.assert-steady-rx]

Test macros such as `assert_steady_rx_eq_take!` MUST compare channel contents deterministically without sleeps.

**Tier:** 0

---

ss[testing.deterministic-no-sleep]

Framework tests MUST NOT rely on wall-clock sleeps for correctness; timeouts are only for blocking until stopped guards.

**Tier:** 0

---

ss[testing.graph-for-testing]

`for_testing` graphs MUST allow synchronous channel injection before/after actor behavior without a media driver.

**Tier:** 0

---

ss[testing.mock-main-thread]

In unit tests, the test thread acts as “main”: it owns the graph builder and clones lazy channels into behaviors.

**Tier:** 1

---

ss[testing.pipeline-worker-allowlist]

StageManager pipeline tests MUST use documented WORKER allowlists when driving multi-actor simulations.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `testing.never-run-in-unit` | No `run()` in unit tests | 0 |
| `testing.internal-behavior-direct` | Direct behavior call | 0 |
| `testing.stage-manager-integration` | StageManager for `run` | 0 |
| `testing.sim-producer-close` | Sim stop closes TX | 0 |
| `testing.assert-steady-rx` | Deterministic asserts | 0 |
| `testing.deterministic-no-sleep` | No sleep-based tests | 0 |
| `testing.graph-for-testing` | Test graph builder | 0 |
| `testing.mock-main-thread` | Test as main | 1 |
| `testing.pipeline-worker-allowlist` | Pipeline allowlist | 1 |
