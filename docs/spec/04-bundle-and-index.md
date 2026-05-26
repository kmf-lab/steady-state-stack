# Bundles and index waits

**Who should read this:** Authors of multi-lane actors and bundle macros.

**See also:** [lesson-on-bundles.md](../../lesson-on-bundles.md), CHANGELOG Unreleased, `steady_actor_shadow.rs`.

---

ss[bundle.girth-const-generic]

Bundle actors MUST be generic over `GIRTH: usize` so the same logic scales from few to many parallel lanes.

**Tier:** 0

---

ss[bundle.clone-establishes]

Calling `.clone()` on lazy bundle handles MUST produce `SteadyTxBundle` / `SteadyRxBundle` with per-lane established channels.

**Tier:** 0

---

ss[bundle.index-wait-readiness]

Bundle index waits MUST mirror single-channel readiness: RX closed-or-available, TX shutdown-or-vacant per lane.

**Tier:** 0

---

ss[bundle.index-wait-repeat-bypass]

`index_wait_avoid_repeat_lane` MUST prefer a different ready lane when the RR winner would repeat the stored cursor.

**Tier:** 0

---

ss[bundle.index-wait-shutdown-none]

Index wait methods MUST return `None` on graph shutdown or empty bundle without updating cursors.

**Tier:** 0

---

ss[bundle.split-macro]

`split_bundle!` MUST destructure lazy TX/RX bundles into named lane variables for tests and actors.

**Tier:** 0

---

ss[bundle.wait-for-index-macro]

`wait_for_index!` MUST delegate to the actor’s index wait APIs with correct bundle/count lengths.

**Tier:** 1

---

ss[bundle.uniform-counts-helper]

`index_wait_counts_uniform_usize` MUST build a uniform `Vec<usize>` for per-lane `wait_avail_index` thresholds.

**Tier:** 0

---

ss[bundle.deprecated-bundle-waits]

`wait_avail_bundle` / `wait_vacant_bundle` MAY remain for all-lanes semantics but MUST stay deprecated where a single winning lane suffices.

**Tier:** 1

---

ss[bundle.trait-vs-actor-index]

`SteadyRxBundleTrait::wait_avail_index` MUST document semantic differences vs `SteadyActor` index waits (shutdown, `Option`, RR).

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `bundle.girth-const-generic` | `const GIRTH` bundles | 0 |
| `bundle.clone-establishes` | Lazy bundle → active | 0 |
| `bundle.index-wait-readiness` | Per-lane readiness | 0 |
| `bundle.index-wait-repeat-bypass` | Anti-sticky lane | 0 |
| `bundle.index-wait-shutdown-none` | `None` on shutdown | 0 |
| `bundle.split-macro` | `split_bundle!` | 0 |
| `bundle.wait-for-index-macro` | `wait_for_index!` | 1 |
| `bundle.uniform-counts-helper` | Uniform count vec | 0 |
| `bundle.deprecated-bundle-waits` | Legacy all-lane waits | 1 |
| `bundle.trait-vs-actor-index` | Trait vs actor docs | 1 |
