# Steady State specification

**The normative requirement catalog for the Steady State framework.** Guides and lessons link here; Tracey links requirements to Rust via `ss[impl]` and `ss[verify]` annotations.

---

## Learning path (recommended)

1. [Conventions](00-conventions.md) — tiers, IDs, waivers  
2. [Philosophy](01-philosophy.md) — manifesto tenets  
3. [Actor](02-actor.md) — `run`, `internal_behavior`, veto, index waits  
4. [Channels](03-channel.md) — lazy, backpressure, testing APIs  
5. [Bundles](04-bundle-and-index.md) — girth, index waits, macros  
6. [Graph & shutdown](05-graph-and-shutdown.md) — liveliness, `for_testing`  
7. [Testing](06-testing-and-simulation.md) — never call `run()` in unit tests  
8. [State](07-state.md) — `SteadyState<S>`, persistence  
9. [Distributed](08-streams-and-distributed.md) — Tier 2 / integration  
10. [Telemetry](09-telemetry-and-observability.md)  
11. [Tooling](10-tooling-cargo-steady-state.md) — `cargo-steady-state`  
12. [Platform](11-dependencies-and-platform.md) — features, ringbuf pin  
13. [Verification stack](12-verification-stack.md) — CI process targets  

**Supplementary (non-normative):** [manifesto](../../steady_state_manifesto.md), [actor lifecycle](../actor_lifecycle.md), [channels](../channels.md), [testing](../testing.md), lessons.

---

## Glossary

| Term | Meaning |
|------|---------|
| **Requirement** | A testable MUST in this spec (`ss[id]` in markdown) |
| **Tier 0** | Release-blocking; needs impl + verify |
| **Lazy channel** | Blueprint until `.clone()` establishes on thread |
| **Veto** | Actor refuses shutdown while work remains |
| **Girth** | Bundle lane count `GIRTH` |
| **Shadow / spotlight** | Graph handle vs active execution context |
| **Tracey** | Traceability tool reading `.config/tracey/config.styx` |

---

## How to read `ss[...]` in code

```rust
// ss[impl actor.lock-first.channels]
async fn internal_behavior(...) { ... }

// ss[verify graph.shutdown.veto]
#[test]
fn test_unclean_shutdown_veto() { ... }
```

Run Tracey (MCP or CLI) with project root `steady-state-stack`:

- `tracey query status` — coverage by impl  
- `tracey query uncovered` / `tracey query untested` — gaps  
- `tracey query unmapped` — code units without `ss[...]` comments  
- `tracey query validate` — reference integrity  

Prefix is **`ss`** for all requirements in `docs/spec/`.

### Unmapped coverage sprint

Target **≥80%** mapped code units on `steady-state/rust-core` (see [`scripts/tracey-baseline.txt`](../../scripts/tracey-baseline.txt)).

1. Edit [`scripts/tracey-file-requirements.yaml`](../../scripts/tracey-file-requirements.yaml) for file → requirement mapping.
2. `python3 scripts/tracey_map_unmapped.py --all-core` — insert `ss[impl]` / `ss[related]` on unannotated items.
3. `python3 scripts/annotate_tracey_tests.py` — insert `ss[verify]` before `#[test]`.
4. `bash scripts/tracey-unmapped-gate.sh` — CI gate (default threshold 80%, env `TRACEY_MAPPED_PERCENT`).
5. `bash scripts/tracey-untested-gate.sh` — CI gate (0 untested for `distributed` and `stream.control-payload` prefixes).
6. Per-file drill-down: `tracey query unmapped core/src/<file>.rs`

---

## Document map

| File | Domain |
|------|--------|
| [00-conventions.md](00-conventions.md) | Tiers, IDs, waivers |
| [01-philosophy.md](01-philosophy.md) | Core tenets |
| [02-actor.md](02-actor.md) | Actor model |
| [03-channel.md](03-channel.md) | Channels |
| [04-bundle-and-index.md](04-bundle-and-index.md) | Bundles |
| [05-graph-and-shutdown.md](05-graph-and-shutdown.md) | Graph |
| [06-testing-and-simulation.md](06-testing-and-simulation.md) | Testing |
| [07-state.md](07-state.md) | State |
| [08-streams-and-distributed.md](08-streams-and-distributed.md) | Distributed |
| [09-telemetry-and-observability.md](09-telemetry-and-observability.md) | Telemetry |
| [10-tooling-cargo-steady-state.md](10-tooling-cargo-steady-state.md) | CLI codegen |
| [11-dependencies-and-platform.md](11-dependencies-and-platform.md) | Platform |
| [12-verification-stack.md](12-verification-stack.md) | CI / quality |

---

## Full requirement index (Tier 0)

| ID | Summary | Spec file |
|----|---------|-----------|
| `philosophy.mechanical-sympathy` | Hardware-aware design | 01 |
| `philosophy.pull-reactor` | Intent-driven progress | 01 |
| `philosophy.structural-hierarchy` | run vs internal_behavior | 01 |
| `philosophy.lock-first-contract` | Lock once at entry | 01 |
| `philosophy.cooperative-liveliness` | Negotiated shutdown | 01 |
| `philosophy.lazy-to-established` | Lazy → established | 01 |
| `philosophy.single-wake-up` | Consolidated awaits | 01 |
| `philosophy.zero-copy-discipline` | Ordered peek/take | 01 |
| `philosophy.explicit-ownership` | Graph owns topology | 01 |
| `actor.run-dispatcher` | Shadow → spotlight | 02 |
| `actor.internal-behavior-logic` | Domain hot path | 02 |
| `actor.lock-first.channels` | Lock at behavior entry | 02 |
| `actor.is-running-loop` | is_running semantics | 02 |
| `actor.shutdown-veto` | Veto closure | 02 |
| `actor.regeneration-survives` | Restart, no data loss | 02 |
| `actor.wait-avail-vacant` | Threshold waits | 02 |
| `actor.index-wait-truthful` | True index readiness | 02 |
| `actor.index-wait-round-robin` | RR cursors | 02 |
| `actor.index-wait-repeat-bypass` | Anti-sticky lane | 02 |
| `actor.index-wait-paired` | Paired lane waits | 02 |
| `channel.lazy.defer-allocation` | Deferred alloc | 03 |
| `channel.lazy.establish-on-clone` | Clone establishes | 03 |
| `channel.backpressure-never-drop` | No silent drops | 03 |
| `channel.testing-send-all` | Test inject | 03 |
| `channel.testing-take-all` | Test drain | 03 |
| `channel.internal-behavior-no-lazy` | No lazy in behavior | 03 |
| `bundle.girth-const-generic` | const GIRTH | 04 |
| `bundle.clone-establishes` | Bundle clone | 04 |
| `bundle.index-wait-readiness` | Lane readiness | 04 |
| `bundle.index-wait-repeat-bypass` | Repeat bypass | 04 |
| `bundle.index-wait-shutdown-none` | None on shutdown | 04 |
| `bundle.split-macro` | split_bundle! | 04 |
| `bundle.uniform-counts-helper` | Uniform counts | 04 |
| `graph.for-testing` | for_testing builder | 05 |
| `graph.shutdown.veto` | Veto vote | 05 |
| `graph.shutdown.accept` | Accept shutdown | 05 |
| `graph.block-until-stopped` | block_until_stopped | 05 |
| `graph.request-shutdown` | request_shutdown | 05 |
| `graph.liveliness-voters` | Voter registry | 05 |
| `graph.panic-restart` | Panic restart | 05 |
| `testing.never-run-in-unit` | No run() in unit tests | 06 |
| `testing.internal-behavior-direct` | Direct behavior | 06 |
| `testing.stage-manager-integration` | StageManager | 06 |
| `testing.sim-producer-close` | Sim TX close | 06 |
| `testing.assert-steady-rx` | assert macros | 06 |
| `testing.deterministic-no-sleep` | No sleep tests | 06 |
| `testing.graph-for-testing` | Test graphs | 06 |
| `state.steady-state-persistence` | Graph-owned state | 07 |
| `state.save-on-drop` | Persist on drop | 07 |
| `state.lock-init-once` | Init once | 07 |
| `state.clone-shared` | Shared Arc | 07 |
| `state.persistent-load` | JSON persistence | 07 |
| `tooling.cargo-driver-strings` | Codegen drivers | 10 |
| `tooling.cargo-percent-parse` | Percent parse | 10 |
| `tooling.cargo-bundle-codegen` | Safe bundle codegen | 10 |
| `tooling.cargo-capacity-driven` | CapacityDriven driver blocks | 10 |
| `tooling.cargo-index-wait-deferred` | Index wait codegen deferred | 10 |
| `platform.ringbuf-pin` | ringbuf 0.4 pin | 11 |
| `platform.executor-features` | Bare-metal block_on; optional tokio reactor | 11 |

---

## Tier 1 / 2 index (abbreviated)

| ID | Tier | File |
|----|------|------|
| `actor.shadow-spotlight` | 1 | 02 |
| `channel.default-capacity` | 1 | 03 |
| `channel.stream-dual-buffer` | 1 | 03 |
| `bundle.deprecated-bundle-waits` | 1 | 04 |
| `graph.actor-identity` | 1 | 05 |
| `telemetry.prometheus-metrics` | 1 | 09 |
| `telemetry.live-title` | 1 | 09 |
| `verify.process.nextest` | 1 | 12 |
| `tooling.cargo-capacity-driven` | 1 | 10 |
| `tooling.cargo-index-wait-deferred` | 1 | 10 |
| `distributed.aeron-uri` | 2 | 08 |
| `distributed.media-driver-testing` | 2 | 08 |
| `platform.aeron-out-of-scope-coverage` | 2 | 11 |

See each domain file for complete Tier 1/2 tables.

---

## Related links

- [TLDR](../../steady_state_tldr.md) — architecture summary  
- [Getting started](../getting_started.md)  
- Tracey config: [`.config/tracey/config.styx`](../../.config/tracey/config.styx)
