# Changelog

All notable changes to this project are documented in this file.

## Unreleased

### Fuzz (cargo-fuzz, parse/protocol)

- `core/fuzz` libFuzzer targets: `aeron_channel_uri`, `fast_protocol_packed`, `cgroup_quota`, `pack_even_split`.
- Non-default `fuzzing` feature re-exports cgroup/pack/FAST internals for those targets only.
- Local/pre-publish campaign: `bash scripts/run-fuzz.sh` (`SS_FUZZ_SECONDS`, skip with `SS_SKIP_FUZZ=1`).
- Spec: `verify.process.fuzz` waiver lifted. Notes: `docs/fuzz.md`.

### Lambda warm graph (`for_lambda`, no Tokio)

- `GraphBuilder::for_lambda()` — telemetry off, no fail-fast exit, no StageManager, pack detect, 256 KiB stacks, **strict persist**.
- Host bridge: `Graph::lambda_bridge` → `LambdaHost` / `LambdaIngress` / `LambdaEgress` (blocking inject / wait-output on the host thread).
- `StateGuard` dirty bit + `new_persistent_state_with`; dirty-at-park under `with_strict_persist` / `for_lambda`.
- Optional Cargo feature **`lambda`**: blocking Runtime API client via `ureq` only (does **not** enable Steady `tokio`). Example: `cargo run -p steady_state --example lambda_warm --features lambda`.
- Spec: `docs/spec/14-lambda.md`. ADR: `docs/arch/005-lambda-warm-graph.md`.

### Coverage (`ss_proptest!`)

- Targeted properties for telemetry `parse_host_port` / `process_msg` / `generate_reports`, pack-edge rebuild + Kruskal leftovers, persist dirty-bit / persist-hook errors, Lambda Runtime API env/error paths, and pack-port identity.
- Follow-on: `DoubleSlice` / `QuadSlice` / `StreamQuadSliceCopy`, `SimIndexable` / `MetaIndexable`, shutdown `report_votes` veto dump, Tx/stream `one`/`log_perodic`/capacity predicates, and `GraphBuilder::default`/`for_production` test panics.
- **Gate B (merged, 2026-09-12):** `scripts/run-llvm-cov-release.sh` → **83.7%** lines (19720 / 23569), **70.7%** functions (13861 / 19606). Prior same-day remasure was 82.5% / 70.0%.

### Dynamic troupe packing (startup schedule)

- `Graph::dynamic_troupe()` — build-time bag; `MemberOf` adds packable actors; **no** OS thread on Drop.
- `Graph::start` / `start_with_timeout` finalize packing **before** `wait_for_registrations` (even-split or Kruskal on build-time incidence).
- `GraphBuilder::with_pack_slots(n)` / `SS_PACK_SLOTS`; `for_testing` defaults to 1 packed slot.
- Slot budget prefers cgroup CPU quota over host core count; SoloAct / normal troupes reserve threads.
- `ActorBuilder::with_pack_ports` / `with_pack_ports_from`; lazy `pack_channel_id()` without establish.
- Example: `cargo run -p steady_state --example dynamic_troupe`. Spec: `docs/spec/13-troupe-packing.md`. Lesson: `lesson-on-dynamic-troupes.md`.
- `ScheduleAs::dynamic_schedule` is unchanged (Solo vs optional normal troupe) — different feature.

## 0.3.1

### Telemetry: graph-share Avg load

- **Avg load %** again reports this actor's share of summed graph **mCPU** (`100 × this_mcpu / Σ all_mcpu`), not the local busy ratio (which remains **Avg mCPU**). Uses last-known mCPU per node so sparse telemetry batches still produce a correct graph total.

## Unreleased (0.3.0)

### Telemetry viewer title

- The builtin viewer title is **Live Telemetry** only after a successful recent `/graph.dot` pull. A failed pull keeps the last diagram and shows **Snapshot** (no “Live”). Initial placeholder remains **Loading…**.
- Builtin DOT layout is tighter (`nodesep=.35`, `ranksep=1.2` via `DOT_NODESEP` / `DOT_RANKSEP`). Raise `DOT_RANKSEP` first if edge labels collide.
- Telemetry viewer uses uniform **contain-fit** to the browser viewport on each successful pull and on window resize (preserves manual zoom level across refreshes).
- Actors that share a base name with distinct suffixes (`with_name_and_suffix`) are packed into one Graphviz column via `{rank=same}` for tighter LR layout.
- `connects_sidecar()` also uses `{rank=same}` (by design). Ranks are transitive: combining a sidecar on one instance of a shared name can pull the sidecar partner into that column — choose names and sidecar edges so the diagram stays readable.

### Aeron publish wake

- Aeron publish actors wait for **one** egress item (plus a 10ms tick) instead of a large batch threshold, so single-frame IPC wire probes are not starved when the periodic timer is not the first to complete.

### Threading / executor (breaking)

- Actor futures are driven with nestable **`futures_lite::future::block_on`** on the SOLO/TROUP OS thread. Exclusive executor features **`exec_async_std`**, **`proactor_nuclei`**, and **`proactor_tokio`** are removed, along with **`ProactorConfig`** and **`GraphBuilder::with_iouring_queue_length`**.
- Optional **`tokio`** feature installs a **current-thread** Tokio runtime inside `block_on` on that same thread (I/O reactor, not a work-stealing pool). Actors stay `!Send`-capable. Do not use `#[tokio::main]`. Default `cargo tree` does not include Tokio.

### Dependencies

- **`ringbuf`** is pinned to **0.4.x** again so it matches **`async-ringbuf` 0.3.5** (same `ringbuf` trait graph). Using **ringbuf 0.5** alongside that `async-ringbuf` caused hundreds of `AsyncWrap` trait-bound errors in stream/channel code. **Do not bump one without the other** in the same change (see comments in `core/Cargo.toml`).

### Coverage (pre-release scope)

- **Goal:** Raise merged `llvm-cov` coverage incrementally (Tier 1/2 tests below); do **not** treat full **Aeron** / **aqueduct** stacks or huge **spotlight/shadow** surfaces as release blockers without dedicated CI (e.g. media driver jobs).
- **Interpretation:** When comparing totals, use the **same** two feature-set runs as `pre-publish.sh` and **merge** LCOVs; a single default `cargo llvm-cov -p steady_state` total will not match release-style numbers. Install the **`lcov`** package locally to run `lcov` / `genhtml` merge steps (`merged.lcov`, `coverage_html/`).
- **Out of scope for strict thresholds (unless policy changes):** `distributed/aeron_*`, deep **aqueduct** integration, **`test_panic_capture`** (panic harness), and **`simulate_edge`** — low or noisy coverage there is expected without extra infrastructure or exclusions.
- **Gate B (merged, 2026-09-12):** `scripts/run-llvm-cov-release.sh` → **83.7%** lines (19720 / 23569), **70.7%** functions (13861 / 19606). Informational; Aeron/spotlight/panic-harness remain waived. Earlier same-day remasure was 82.5% / 70.0%; 2026-08-18 was 82.1% / 66.0% on a smaller surface.

### `SteadyActor` index waits

- **`wait_avail_index`**, **`wait_vacant_index`**, and **`wait_avail_vacant_index`** now wait only until a lane **truly** satisfies thresholds (RX closed-or-avail and TX shutdown-or-vacant semantics preserved); spurious completions no longer return a misleading index.
- **`wait_avail_vacant_index`** no longer uses an outer poll loop with `yield_now`; each lane uses a **paired** wait helper until both RX and TX sides are ready, with **`FuturesUnordered`** and graph shutdown **`select!`** at the monitor layer.
- **Round-robin** scan order and per-method cursors are unchanged; if the winning index would **repeat** the last returned index, a **synchronous** scan prefers another ready lane when one exists.
- **`None`** continues to mean graph shutdown (or empty bundle) for these methods; cursors are not updated on **`None`**.
- Helper **`index_wait_counts_uniform_usize`** (re-exported from the crate root) builds a uniform `Vec<usize>` for `wait_avail_index` counts.

### Testing

- Additional **`ss_proptest!`** coverage: DOT same-name + sidecar `{rank=same}` coexistence, partial-vote unclean shutdown, `split_bundle` partitions, channel avg-fill when not shown.
- **Verification gates:** Gate A (`cargo nextest --profile ci-unit`), Gate B (`scripts/run-llvm-cov-release.sh`), Gate C (`scripts/run-aeron-integration.sh`); live `aeron_integration_suite` excluded from default nextest/llvm-cov.
- **Aeron release:** full-matrix sign-off via `scripts/run-aeron-release-signoff.sh` (≥17 `PASS [` scenarios); flake gate `scripts/run-aeron-flake-check.sh`; self-hosted CI job in `.github/workflows/aeron-integration.yml` (labels `self-hosted`, `aeron`); runner setup in `docs/AERON_RUNNER.md`.
- **Aeron Gate C reliability:** `suite_in_process_warmup` after script smoke (in-process wire proof); bundle lane wire retries + lane-0 warmup; `aeron_subscribe_bundle` connection bootstrap poll; harness contract tests.
- **Bundle pub/sub unit tests:** simulated graph start/stop for `aeron_publish_bundle` and `aeron_subscribe_bundle` (`GraphBuilder::for_testing`).
- **pre-publish:** `scripts/guard-llvm-cov-scope.sh` blocks accidental `cargo llvm-cov test --tests`.
- Aeron driver restart: `SS_AERON_RESTART_VIA` (docker-first), reduced in-suite refresh cadence, IPC wire probe after restart and at suite preflight (soft-skip when pub/sub not ready).
- Unit and integration tests for index-wait helpers, spotlight/shadow paths, bundle traits, and `wait_for_index!`; **`cargo-steady-state`** tests for multi-lane driver string emission (`wait_avail_bundle` / `wait_vacant_bundle`).
- **`cargo-steady-state`:** `build_driver_block` coverage for **`AtMostEvery`**, **`Other`**, **`AtLeastEvery` + `EventDriven`**, bundle **percent** parsing on RX/TX, and **`extract_percent`** decimal edge cases.
- **`graph_liveliness`:** clean shutdown path where the actor accepts stop (`is_running(|| true)`).
- **`macros`:** `split_bundle!` on a four-lane lazy TX bundle.
- **`channel_builder` / `channel_stats` / `steady_rx` / `steady_tx` / `core_rx_stream`:** small focused tests (`with_memory_usage`, Prometheus label suffix behavior, empty RX peek, TX vacant before/after send, stream capacity / avail predicates).

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
