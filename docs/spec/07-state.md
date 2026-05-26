# State management

**Who should read this:** Authors using `SteadyState<S>` and persistence.

**See also:** [lesson-on-steadystate.md](../../lesson-on-steadystate.md), `state_management.rs`.

---

ss[state.steady-state-persistence]

`SteadyState<S>` MUST live in the graph root and MUST survive actor regeneration across panics on the same graph instance.

**Tier:** 0

---

ss[state.save-on-drop]

When `on_persist` is configured, dropping a `StateGuard` MUST invoke persistence hooks so durable state is written.

**Tier:** 0

---

ss[state.lock-init-once]

`lock(init)` MUST run `init` only when inner state is `None`; subsequent locks MUST return existing data.

**Tier:** 0

---

ss[state.clone-shared]

Cloning `SteadyState` MUST share the same `Arc` so actors and main observe one logical state.

**Tier:** 0

---

ss[state.persistent-load]

`new_persistent_state` MUST load JSON from disk when valid; MUST initialize when file missing.

**Tier:** 0

---

ss[state.try-lock-sync]

`try_lock_sync` MUST allow post-shutdown inspection in tests without async runtime blocking.

**Tier:** 1

---

ss[state.on-drop-hook]

Optional `on_drop` hooks MUST run when guards drop for non-persisted side effects.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `state.steady-state-persistence` | Graph-owned state | 0 |
| `state.save-on-drop` | Persist on guard drop | 0 |
| `state.lock-init-once` | Init once | 0 |
| `state.clone-shared` | Shared Arc | 0 |
| `state.persistent-load` | JSON load/save | 0 |
| `state.try-lock-sync` | Sync test lock | 1 |
| `state.on-drop-hook` | on_drop callback | 1 |
