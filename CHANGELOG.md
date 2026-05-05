# Changelog

All notable changes to this project are documented in this file.

## Unreleased

### `SteadyActor` index waits

- **`wait_avail_index`**, **`wait_vacant_index`**, and **`wait_avail_vacant_index`** now wait only until a lane **truly** satisfies thresholds (RX closed-or-avail and TX shutdown-or-vacant semantics preserved); spurious completions no longer return a misleading index.
- **`wait_avail_vacant_index`** no longer uses an outer poll loop with `yield_now`; each lane uses a **paired** wait helper until both RX and TX sides are ready, with **`FuturesUnordered`** and graph shutdown **`select!`** at the monitor layer.
- **Round-robin** scan order and per-method cursors are unchanged; if the winning index would **repeat** the last returned index, a **synchronous** scan prefers another ready lane when one exists.
- **`None`** continues to mean graph shutdown (or empty bundle) for these methods; cursors are not updated on **`None`**.
- Helper **`index_wait_counts_uniform_usize`** (re-exported from the crate root) builds a uniform `Vec<usize>` for `wait_avail_index` counts.

### Testing

- Unit and integration tests for index-wait helpers, spotlight/shadow paths, bundle traits, and `wait_for_index!`; **`cargo-steady-state`** tests for multi-lane driver string emission (`wait_avail_bundle` / `wait_vacant_bundle`).

### Documentation

- **`lesson-on-bundles.md`**: index waits, RR, repeat-index bypass, paired behavior, shutdown semantics, telemetry/capacity notes, bundle-trait differences.
- **`SteadyActor`** trait rustdocs: telemetry and capacity differences vs `wait_avail` / `wait_vacant`, all-zero `avail_counts`, `wait_vacant_index` zero-threshold semantics vs bundle traits.
- **`SteadyRxBundleTrait::wait_avail_index`** / **`SteadyTxBundleTrait::wait_vacant_index`**: clarified difference vs **`SteadyActor`** (shutdown, `Option`, RR).

### Examples

- **`core/examples`**: `#[allow(deprecated)]` on actors that still intentionally use **`wait_*_bundle`** for **all-lanes** (or stream) readiness; index waits are not a drop-in replacement there.

### Tooling

- **`cargo-steady-state`**: regression tests for multi-lane **EventDriven** / **CapacityDriven** driver string emission (`wait_avail_bundle` / `wait_vacant_bundle`); single-lane drivers emit **`wait_avail`** / **`wait_vacant`**. (Emitting **`wait_*_index`** in generated actors is deferred until templates pin a **`steady_state`** release that includes those APIs.)
- **`cargo-steady-state`**: **CapacityDriven** bundle path no longer reads `v[2]` when the driver vector has only two entries (fixes panic / bad codegen).

### Deprecations (unchanged)

- **`wait_avail_bundle`** / **`wait_vacant_bundle`** remain deprecated in favor of the index-returning APIs where a single winning lane is enough; bundle waits still apply when “any K of N” semantics are required.
