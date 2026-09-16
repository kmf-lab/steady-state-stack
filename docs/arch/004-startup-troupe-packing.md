# ADR 004: Startup troupe packing

## Context

Steady State maps actors to **OS threads** (SoloAct or cooperative troupes), not to a work-stealing pool. Deployments often share one binary across differently sized hosts (Kubernetes CPU requests, appliance SKUs, Lambda memory→vCPU). Today, authors hard-code `MemberOf` / linedance / Solo counts. That either under-uses cores or oversubscribes a cgroup when `NPROCESSORS_ONLN` reports host CPUs.

Erlang/BEAM maps many actors onto N schedulers **continuously**. Steady’s differentiator is the opposite: **place once at `Graph::start`, then freeze** the mapping so cache locality and `!Send` thread identity stay stable.

`into_spotlight` records channel endpoints for telemetry **after** actors run. Packing that wants “smallest channel = same core” needs **build-time incidence**, which the Graph does not yet store.

## Decision

1. **`Graph::dynamic_troupe()`** returns a bag of packable actors. `ScheduleAs::MemberOf` is unchanged. Drop of a **dynamic** guard does **not** spawn; Drop of a **normal** `actor_troupe()` guard still spawns one OS thread.
2. **`Graph::start` / `start_with_timeout`** finalizes every dynamic bag **before** `wait_for_registrations`: compute `slots`, pack (even-split, then Kruskal when incidence exists), spawn ≤ `slots` normal sub-troupes.
3. **Slot budget** prefers cgroup CPU quota when present; else `available_parallelism`. Subtract Solo threads, normal troupes, and telemetry threads if enabled. Tests pin via `with_pack_slots` / `SS_PACK_SLOTS`. `for_testing` defaults packed graphs to `slots = 1`.
4. **Incidence** (tx/rx actor + capacity + bundle family) is recorded at build for packing. Spotlight remains the runtime/DOT topology; mismatch is warned. Kruskal merges smallest capacities first; bundle siblings are not merged while `cluster_count > slots`.
5. **No live re-pack.** More cores on a running pod require a new process. Panic restart stays on the same sub-troupe thread.

`ScheduleAs::dynamic_schedule` keeps its old meaning (optional normal troupe vs Solo). Do not overload that name.

## Consequences

- Same binary can use 2 or 8 vCPUs without rewriting the graph; extra cores help **width** (bundles), not a 3-actor chain.
- Dual topology (pack ports vs spotlight) is a maintenance risk; tests and warns mitigate it.
- Docs must teach: drop-before-start still stashes the bag on the Graph; start owns finalize — unlike normal troupes’ spawn-on-drop.

## Alternatives rejected

- Work-stealing pool: violates Steady’s OS-thread / `!Send` contract.
- Spawn one mega-thread then split: breaks lazy establish-on-clone thread affinity.
- Using only post-spawn DOT edges: too late for packing at start.
