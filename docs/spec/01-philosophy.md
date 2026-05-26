# Philosophy and core tenets

**Who should read this:** New contributors and reviewers aligning code with the [manifesto](../../steady_state_manifesto.md).

---

ss[philosophy.mechanical-sympathy]

The framework MUST minimize context switches, cache misses, and unnecessary CPU cycles; abstractions MUST be chosen with hardware cost in mind.

**Rationale:** Manifesto tenet I — design for the machine.

**Tier:** 0

---

ss[philosophy.pull-reactor]

Progress MUST be driven by consumer intent and producer capacity; idle actors MUST consume negligible CPU until a registered condition is satisfied.

**Rationale:** Pull-reactor model (manifesto §II.1).

**Tier:** 0

---

ss[philosophy.structural-hierarchy]

Every actor MUST separate orchestration (`run`) from domain logic (`internal_behavior`) so simulation and unit tests can target logic without graph wiring.

**Rationale:** Manifesto tenet III.

**Tier:** 0

---

ss[philosophy.lock-first-contract]

Actors MUST acquire all channel and state guards once at the start of `internal_behavior` and MUST NOT re-lock inside the hot loop.

**Rationale:** Lock-first resource contract (manifesto §II.3).

**Tier:** 0

---

ss[philosophy.cooperative-liveliness]

Shutdown MUST be negotiated via `is_running` and a veto closure; the graph MUST NOT force-stop actors that report remaining work.

**Rationale:** Cooperative liveliness (manifesto §II.4).

**Tier:** 0

---

ss[philosophy.lazy-to-established]

Channels MUST be created lazy in the graph or test layer and MUST become established on clone to the actor thread.

**Rationale:** Lazy-to-established lifecycle (manifesto §II.5).

**Tier:** 0

---

ss[philosophy.single-wake-up]

Actors MUST consolidate wait conditions at a single wake point using `await_for_all!` / `await_for_any!` (or approved equivalents).

**Rationale:** Single point of wake-up (manifesto §II.6).

**Tier:** 0

---

ss[philosophy.zero-copy-discipline]

Peek/take semantics MUST preserve message ordering; copying data out of order MUST be treated as a protocol violation.

**Rationale:** Zero-copy discipline (manifesto §II.8).

**Tier:** 0

---

ss[philosophy.explicit-ownership]

The graph (main) MUST own system topology; actors MUST operate on clones so the system survives individual actor failure.

**Rationale:** Explicit ownership (manifesto tenet IX).

**Tier:** 0

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `philosophy.mechanical-sympathy` | Hardware-aware design | 0 |
| `philosophy.pull-reactor` | Intent-driven progress, idle = 0% CPU | 0 |
| `philosophy.structural-hierarchy` | `run` vs `internal_behavior` | 0 |
| `philosophy.lock-first-contract` | Lock once at behavior entry | 0 |
| `philosophy.cooperative-liveliness` | Negotiated shutdown | 0 |
| `philosophy.lazy-to-established` | Lazy blueprint → established on clone | 0 |
| `philosophy.single-wake-up` | Consolidated await macros | 0 |
| `philosophy.zero-copy-discipline` | Ordered peek/take | 0 |
| `philosophy.explicit-ownership` | Graph owns, actors clone | 0 |
