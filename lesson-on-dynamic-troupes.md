# Lesson: Dynamic Troupes — Startup Hardware Mapping

> Map a fixed actor graph onto the CPU budget of **this process**, once, at `Graph::start`.

Related: [`lesson-on-troups.md`](lesson-on-troups.md), [`docs/spec/13-troupe-packing.md`](docs/spec/13-troupe-packing.md),
ADR [`004-startup-troupe-packing`](docs/arch/004-startup-troupe-packing.md), example
[`core/examples/dynamic_troupe`](core/examples/dynamic_troupe).

---

## Why

The same binary may run on a 2-vCPU pod or an 8-vCPU box. You want:

- **One** packing decision at startup (no steal after the first `run`)
- Slot count from **cgroup quota** (or a pin), not host `NPROCESSORS_ONLN`
- Extra cores only when the graph has **width** (bundles / many workers)

This is **not** work-stealing and **not** live re-pack when the pod CPU limit changes without restart.

---

## Name clash: `dynamic_schedule` vs dynamic **troupe**

| API | Meaning |
|-----|---------|
| `ScheduleAs::dynamic_schedule(&mut Option<TroupeGuard>)` | Pick Solo vs an optional **normal** troupe at build time |
| `Graph::dynamic_troupe()` | Build-time **bag**; `MemberOf` adds actors; `start` packs into ≤ N OS threads |

Prefer the packing APIs when you mean startup hardware mapping.

---

## Pattern

```rust
let mut bag = graph.dynamic_troupe();

// Optional: declare incidence for Kruskal locality (must match into_spotlight).
builder
    .with_pack_ports_from(&[&rx], &[&tx])
    .build(|c| actor::run(c, rx.clone(), tx.clone()), ScheduleAs::MemberOf(&mut bag));

// SoloAct / normal MemberOf still opt out and consume reserved threads.
builder.build(|c| vip::run(c), ScheduleAs::SoloAct);

// Drop-before-start is OK (Graph already owns the bag). Holding across start() also works.
drop(bag);
graph.start(); // finalize packs here, then wait_for_registrations
```

Pin slots in tests / demos:

```rust
GraphBuilder::for_testing().with_pack_slots(2).build(());
// or SS_PACK_SLOTS=2
```

`GraphBuilder::for_testing()` defaults to **1** packed slot so unit tests do not fan out across a laptop’s cores.

---

## Honesty checklist

- Packing does **not** parallelize a sequential pipeline onto N cores.
- Kubernetes **replicas** remain HPA; this feature is one process on one machine.
- Packed `clone()` still establishes on the **sub-troupe** thread after split.
- Panic restart stays on the **same** packed OS thread (no re-pack).

---

## Try it

```bash
cargo run -p steady_state --example dynamic_troupe -- --slots 2 --beats 8
```

Compare thread ids for the small-cap pipeline pair vs the worker bundle as you change `--slots`.
