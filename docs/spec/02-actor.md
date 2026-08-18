# Actor model

**Who should read this:** Authors implementing `run`, `internal_behavior`, and `SteadyActor` loops.

**See also:** [Actor lifecycle](../actor_lifecycle.md), `core/src/steady_actor.rs`, `steady_actor_core.rs`, `steady_actor_shadow.rs`.

---

ss[actor.run-dispatcher]

`run` MUST convert `SteadyActorShadow` to spotlight, select production vs simulation mode, and delegate to `internal_behavior` or `simulated_behavior`.

**Acceptance:** Production and test graphs call `run`; unit tests call `internal_behavior` directly per `testing.never-run-in-unit`.

**Tier:** 0

---

ss[actor.internal-behavior-logic]

`internal_behavior` MUST contain domain logic only: guard-first, `is_running` loop, consolidated waits, and message processing without graph orchestration.

**Tier:** 0

---

ss[actor.lock-first.channels]

At the start of `internal_behavior`, the actor MUST call `.acquire_guard().await` on every `SteadyRx` / `SteadyTx` (and bundle guards) it uses in the loop.

**Tier:** 0

---

ss[actor.is-running-loop]

`is_running` MUST return `Some(true)` while the graph runs and MUST invoke the veto closure only when shutdown is requested.

**Tier:** 0

---

ss[actor.shutdown-veto]

The veto closure MUST return `false` when inbound channels are not `is_closed_and_empty()` or outbound is not appropriately closed; returning `false` MUST veto shutdown.

**Tier:** 0

---

ss[actor.regeneration-survives]

On panic or error, the supervisor MUST restart the actor with an incremented regeneration counter; channel data MUST NOT be lost.

**Tier:** 0

---

ss[actor.shadow-spotlight]

`into_spotlight` MUST register channel metadata for telemetry before `internal_behavior` executes.

**Tier:** 1

---

ss[actor.wait-avail-vacant]

`wait_avail` and `wait_vacant` MUST complete only when the guarded channel satisfies availability or vacancy thresholds (or graph shutdown applies).

**Tier:** 0

---

ss[actor.index-wait-truthful]

`wait_avail_index`, `wait_vacant_index`, and `wait_avail_vacant_index` MUST return an index only when that lane truly satisfies thresholds; spurious wakeups MUST NOT return a misleading index.

**Acceptance:** CHANGELOG unreleased; shadow unit tests.

**Tier:** 0

---

ss[actor.index-wait-round-robin]

Index waits MUST scan lanes in round-robin order from a per-method cursor; cursors MUST NOT advance on `None` (shutdown or empty bundle).

**Tier:** 0

---

ss[actor.index-wait-repeat-bypass]

If the winning index would repeat the last returned index, a synchronous scan MUST prefer another ready lane when one exists.

**Tier:** 0

---

ss[actor.index-wait-paired]

`wait_avail_vacant_index` MUST use paired per-lane waits without an outer `yield_now` poll loop; shutdown MUST integrate via monitor `select!`.

**Tier:** 0

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `actor.run-dispatcher` | Shadow → spotlight, mode dispatch | 0 |
| `actor.internal-behavior-logic` | Domain-only hot path | 0 |
| `actor.lock-first.channels` | One lock per channel at entry | 0 |
| `actor.is-running-loop` | Running + veto semantics | 0 |
| `actor.shutdown-veto` | Veto when work remains | 0 |
| `actor.regeneration-survives` | Restart without data loss | 0 |
| `actor.shadow-spotlight` | Telemetry registration | 1 |
| `actor.wait-avail-vacant` | Threshold-correct waits | 0 |
| `actor.index-wait-truthful` | No spurious index | 0 |
| `actor.index-wait-round-robin` | RR scan + cursors | 0 |
| `actor.index-wait-repeat-bypass` | Avoid sticky lane | 0 |
| `actor.index-wait-paired` | Paired lane readiness | 0 |
