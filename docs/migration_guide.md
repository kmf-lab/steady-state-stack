# Migration guide

**Normative API contracts:** [docs/spec/README.md](spec/README.md) and [CHANGELOG](../CHANGELOG.md).

---

## 0.3.1

### Lambda warm graph

- New: `GraphBuilder::for_lambda()`, `Graph::lambda_bridge`, `new_persistent_state_with`, optional feature `lambda` + `ureq` Runtime API.
- Tests: use `for_testing().with_strict_persist()` (do not call `for_lambda` under `cfg(test)`).
- Persist before every park when `on_persist` is set; `/tmp` is not durable across sandboxes.
- Do **not** enable Steady `tokio` for Lambda. Cargo-lambda remains packaging only.
- See [14-lambda](spec/14-lambda.md) and ADR [005](arch/005-lambda-warm-graph.md).

### Dynamic troupe packing

- New: `Graph::dynamic_troupe()`, `GraphBuilder::with_pack_slots`, packing finalize inside `start`.
- Drop of a **normal** troupe still spawns; drop of a **dynamic** bag does not (Graph owns the bag; `start` packs).
- `ScheduleAs::dynamic_schedule` is **unchanged** — do not confuse with dynamic troupe packing.
- Tests: pin slots with `with_pack_slots`; `for_testing` defaults to 1 packed slot.
- See [13-troupe-packing](spec/13-troupe-packing.md) and [lesson-on-dynamic-troupes](../lesson-on-dynamic-troupes.md).

### Threading / executor (breaking)

- Remove Cargo features `exec_async_std`, `proactor_nuclei`, and `proactor_tokio`. The default crate uses OS-thread `block_on`.
- Remove public `ProactorConfig` and `GraphBuilder::with_iouring_queue_length`.
- To use Tokio I/O in an actor: `steady_state = { version = "0.3.1", features = ["tokio"] }`. That is a current-thread reactor on the actor's OS thread, not a Tokio pool.

### Dependencies

- Keep **`ringbuf` 0.5.x** aligned with **`async-ringbuf` 0.3.9** — do not bump one without the other (`platform.ringbuf-pin`; addresses RUSTSEC-2026-0293).

### Actor index waits

- Prefer `wait_avail_index` / `wait_vacant_index` / `wait_avail_vacant_index` over deprecated `wait_*_bundle` when a single winning lane is enough.  
- Index waits are truthful (no spurious index), round-robin, with repeat-index bypass — see CHANGELOG **0.3.1** and [04-bundle-and-index](spec/04-bundle-and-index.md).

### Testing

- Unit tests: call `internal_behavior`, not `run()`, on `for_testing` graphs.  
- CI moving to `cargo nextest` per [12-verification-stack](spec/12-verification-stack.md).

### Codegen

- `cargo-steady-state`: CapacityDriven bundle fix (no out-of-bounds driver vector access). Index-wait emission in generated actors remains deferred until template pins a release with index APIs.

---

## Version upgrades

1. Read [CHANGELOG](../CHANGELOG.md) for your target version.  
2. Run `cargo test` or `cargo nextest run` with the same feature set you ship.  
3. Reconcile Tracey: `tracey_status` after pulling spec changes.

---

## See also

- [Specification README](spec/README.md)  
- [Platform requirements](spec/11-dependencies-and-platform.md)
