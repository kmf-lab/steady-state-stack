# Verification stack (process)

**Who should read this:** CI maintainers adopting nextest, llvm-cov, proptest, fuzz, and mutants.

**Note:** Process requirements here define targets. llvm-cov CI remains follow-on work per [00-conventions](00-conventions.md) waivers. Fuzz runs via `scripts/run-fuzz.sh`. Mutants run via `scripts/run-mutants-iterate.sh` (local/pre-publish, not a GitHub PR job).

---

ss[verify.process.nextest]

CI and release scripts SHOULD run workspace tests via `cargo nextest` for parallelism and clearer failure reports.

**Tier:** 1

---

ss[verify.process.llvm-cov]

Coverage gates SHOULD use `cargo llvm-cov` with merged LCOV per `platform.coverage-merge`.

**Tier:** 1

---

ss[verify.process.proptest]

Property tests cover Tier-0 channel, actor, stats, graph, and telemetry invariants at **2048** cases per property (`ss_proptest!`, `core/src/proptest_support/`). See `docs/testing.md` for conventions.

**Tier:** 1

---

ss[verify.process.fuzz]

The repository MUST contain `cargo-fuzz` targets under `core/fuzz` covering parse and protocol edges: Aeron channel URI construction, FAST packed longs, cgroup CPU quota strings, and even-split troupe packing. Release and local campaigns MUST run via `scripts/run-fuzz.sh` (honor `SS_SKIP_FUZZ=1` when nightly or clang is unavailable). Campaign duration SHOULD be at least 20 seconds per target (`SS_FUZZ_SECONDS`).

**Tier:** 1

---

ss[verify.process.mutants]

A current nextest `cargo-mutants` campaign MUST cover the Stage-1 critical modules listed in `core/mutants.toml` `examine_globs` (`dot_unify.rs`, `graph/liveliness.rs`, `core_rx_stream.rs`, `core_tx_stream.rs`). Local and pre-publish runs MUST use `scripts/run-mutants-iterate.sh` (honor `SS_SKIP_MUTANTS=1` when the toolchain is unavailable; keep the call **file-scoped**, not whole-crate). Kill-rate MUST be recorded in `docs/mutants.md` as `(caught + timeout) / (caught + timeout + missed)` with `missed = 0` on that set. Timeouts count as killed. There is no GitHub PR mutants job.

**Tier:** 1

---

ss[verify.process.tracey-gate]

Pull requests SHOULD run Tracey `validate` and fail on uncovered/untested Tier-0 requirements once annotations are complete.

**Tier:** 1

---

ss[verify.process.file-size]

Rust sources under `core/src/` SHOULD stay below **1,200 lines** per file (soft target) and MUST NOT exceed **1,800 lines** (hard cap). CI SHOULD run `bash scripts/check-file-size.sh` on pull requests.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `verify.process.nextest` | nextest in CI | 1 |
| `verify.process.llvm-cov` | llvm-cov merge | 1 |
| `verify.process.proptest` | proptest (deferred) | 1 |
| `verify.process.fuzz` | cargo-fuzz parse/protocol targets | 1 |
| `verify.process.mutants` | cargo-mutants Stage-1 campaign | 1 |
| `verify.process.tracey-gate` | Tracey on PR | 1 |
| `verify.process.file-size` | per-file line cap | 1 |
