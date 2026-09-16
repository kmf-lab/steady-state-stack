# Troupe packing (startup schedule)

**Who should read this:** Anyone building production graphs that must map to the CPU budget of the host process (pods, appliances, Lambda memory=vCPU, embedded boxes).

**See also:** [graph.troupes](05-graph-and-shutdown.md), [philosophy.startup-schedule](01-philosophy.md), ADR [004-startup-troupe-packing](../arch/004-startup-troupe-packing.md), [lesson-on-dynamic-troupes.md](../../lesson-on-dynamic-troupes.md), [Lambda warm graph](14-lambda.md) (pack slots map actors onto the sandbox CPU).

**Honesty:** Packing does **not** parallelize a sequential pipeline onto N cores. Extra slots help only when the graph has parallel width (bundles / many workers). This is **one process, this machine** — replica autoscaling remains HPA.

---

## Glossary

| Term | Meaning |
|------|---------|
| **Normal troupe** | `MemberOf` a troupe that spawns **one OS thread** when its guard is dropped (today’s behavior) |
| **Dynamic troupe** | Build-time **bag** of packable actors; **not** an OS thread; split at `Graph::start` into ≤ N normal sub-troupes |
| **Packable actor** | Actor added with `ScheduleAs::MemberOf` of a **dynamic** troupe |
| **Slot** | One OS thread the packer may create for packed actors |
| **Incidence** | For a channel: which actor holds TX, which holds RX, plus capacity / bundle family — known **before** finalize |

`ScheduleAs::dynamic_schedule` is **not** this feature. It only picks Solo vs an optional normal troupe. Prefer `Graph::dynamic_troupe()` / packing APIs.

---

ss[troupe.dynamic-is-a-bag]

A dynamic troupe MUST NOT spawn an OS thread for itself. After finalize it MUST be empty of pending members. Each cluster produced by packing MUST become a **normal** sub-troupe (one OS thread each).

**Tier:** 0

---

ss[troupe.dynamic-member-of]

Packable actors MUST use the same `ScheduleAs::MemberOf` surface as normal troupes. `SoloAct` and `MemberOf` of a **normal** troupe MUST remain opt-outs and MUST occupy threads that the packer MUST NOT reuse as packed slots.

**Tier:** 0

---

ss[troupe.dynamic-hold-until-start]

Packable actors MUST NOT execute `run`, establish channels via `clone()`, or call `into_spotlight` until packing finalize has spawned their sub-troupe. Finalize MUST occur before those actors are scheduled.

**Tier:** 0

---

ss[troupe.dynamic-finalize-at-start]

`Graph::start` / `start_with_timeout` MUST finalize every dynamic troupe **before** `wait_for_registrations`. Finalize MUST be idempotent. Drop of a dynamic guard before start MUST hand members to the graph (MUST NOT spawn one mega-thread). Holding the guard across `start()` MUST still allow finalize.

**Tier:** 0

---

ss[troupe.dynamic-slot-budget]

Packed OS threads MUST be `≤ slots` where  
`slots = max(1, detected − SoloAct threads − normal troupe threads − telemetry threads if telemetry is enabled)`.  
`detected` MUST prefer cgroup CPU quota when present; otherwise `available_parallelism`. Tests and examples MUST be able to pin `slots` via `GraphBuilder::with_pack_slots` and/or `SS_PACK_SLOTS`.

**Tier:** 0

---

ss[troupe.dynamic-no-repack]

After the first packed `run` on a sub-troupe thread, that actor MUST stay on that OS thread for the process lifetime. Panic restart MUST regenerate on the **same** sub-troupe thread. Live re-pack when CPU limits change without process restart is MUST NOT.

**Tier:** 0

---

ss[troupe.dynamic-channel-establish]

Packed channel `clone()` MUST establish on the **sub-troupe** thread after split (`channel.lazy.establish-on-clone`). Finalize MUST NOT establish rings on the main thread in lieu of the actor thread.

**Tier:** 0

---

ss[graph.pack.incidence-before-start]

Before finalize, when locality packing is used, the graph MUST know for every local channel connecting two packable actors: `{channel_id, capacity, tx_actor, rx_actor}` and bundle family when present. This MUST NOT depend on telemetry frames or post-spawn `into_spotlight`. Pack ports recorded at build MUST match spotlight port ids; mismatch MUST be diagnosable (e.g. warn).

**Tier:** 0

---

ss[troupe.dynamic-kruskal-capacity]

When incidence is present, finalize MUST cluster packable actors by union-find: sort edges by capacity ascending (tie-break: ring byte footprint); union while `cluster_count > slots` and both ends are packable. Each cluster MUST become one sub-troupe, pinned to a distinct core when affinity is enabled.

**Tier:** 0

---

ss[troupe.dynamic-bundle-siblings]

While `cluster_count > slots`, finalize MUST NOT union two consumers that share a producer through the same bundle family (parallel lanes). Unique small pipeline edges (A→B) MUST be preferred for merge.

**Tier:** 0

---

ss[troupe.dynamic-no-incidence-fallback]

If incidence is empty or incomplete such that Kruskal cannot run, finalize MUST still produce `≤ slots` troupes by even split (or equivalent) and MUST log that locality packing was skipped. Finalize MUST NOT hang `start`.

**Tier:** 0

---

ss[troupe.dynamic-for-testing]

`GraphBuilder::for_testing` MUST NOT pack dynamic troupes across the host’s full `available_parallelism` by default. Tests that use a dynamic troupe MUST pin `slots` (e.g. `with_pack_slots`) or the default for test graphs MUST be `slots = 1`.

**Tier:** 0

---

ss[troupe.dynamic-linedance-order]

Finalize SHOULD order actors inside a sub-troupe along intra-cluster edges (producer before consumer) for cooperative poll locality. Same-thread co-location is the Tier 0 locality guarantee; poll order is SHOULD.

**Tier:** 1

---

ss[troupe.dynamic-explicit-core]

Packed actors with `with_explicit_core` SHOULD pin or constrain their cluster. Two packed actors that declare **conflicting** explicit cores and are connected by a packing edge MUST NOT be merged onto one thread.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `troupe.dynamic-is-a-bag` | Dynamic troupe is not an OS thread | 0 |
| `troupe.dynamic-member-of` | Same MemberOf; Solo/normal opt-out | 0 |
| `troupe.dynamic-hold-until-start` | No packed run before finalize | 0 |
| `troupe.dynamic-finalize-at-start` | start finalizes before registration wait | 0 |
| `troupe.dynamic-slot-budget` | Slot math + pin for tests | 0 |
| `troupe.dynamic-no-repack` | No live re-pack; same thread on restart | 0 |
| `troupe.dynamic-channel-establish` | Establish on sub-troupe thread | 0 |
| `graph.pack.incidence-before-start` | Build-time incidence for Kruskal | 0 |
| `troupe.dynamic-kruskal-capacity` | Smallest-capacity union-find | 0 |
| `troupe.dynamic-bundle-siblings` | Do not glue parallel bundle consumers | 0 |
| `troupe.dynamic-no-incidence-fallback` | Even split if no incidence | 0 |
| `troupe.dynamic-for-testing` | Test graphs pin or slots=1 | 0 |
| `troupe.dynamic-linedance-order` | Intra-cluster poll order | 1 |
| `troupe.dynamic-explicit-core` | Explicit core vs merge | 1 |
