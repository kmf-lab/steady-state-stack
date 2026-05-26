# Migration guide

**Normative API contracts:** [docs/spec/README.md](spec/README.md) and [CHANGELOG](../CHANGELOG.md).

---

## Unreleased (current branch)

### Dependencies

- Keep **`ringbuf` 0.4.x** aligned with **`async-ringbuf` 0.3.5** — do not bump one without the other (`platform.ringbuf-pin`).

### Actor index waits

- Prefer `wait_avail_index` / `wait_vacant_index` / `wait_avail_vacant_index` over deprecated `wait_*_bundle` when a single winning lane is enough.  
- Index waits are truthful (no spurious index), round-robin, with repeat-index bypass — see CHANGELOG Unreleased and [04-bundle-and-index](spec/04-bundle-and-index.md).

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
