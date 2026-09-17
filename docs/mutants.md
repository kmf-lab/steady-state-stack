# Mutation testing (cargo-mutants)

Critical `core` modules are mutated with **cargo-mutants**. Actor `is_running` loops and live Aeron are **out of scope** for this campaign (hang-prone). Timeouts count as **killed**. Do not skip a mutant to hide a survivor unless the skip is hang-prone control flow named in the campaign log.

Kill-rate:

`(caught + timeout) / (caught + timeout + missed)`

`unviable` (does not compile) is excluded from the denominator.

## Requirements

- `cargo install cargo-mutants` (27.x)
- nextest (`--profile mutants` excludes live Aeron suites)

## Run

From the repository root, **one file at a time**:

```bash
bash scripts/run-mutants-iterate.sh -f dot_unify.rs
```

On a survivor the script **stops** (exit 2). Add an `ss_proptest!` (preferred) or a focused `#[test]`, then:

```bash
bash scripts/snapshot-mutants-score.sh
bash scripts/run-mutants-iterate.sh -f dot_unify.rs
```

`--iterate` skips mutants already in `mutants.out/caught.txt`. Skip the whole step when the toolchain is unavailable:

```bash
SS_SKIP_MUTANTS=1 bash scripts/run-mutants-iterate.sh
```

Per-command timeout defaults to **240s** so the unmutated nextest baseline (`--test-threads 1`) can finish; hang-mutants still floor at **30s** (TIMEOUT = killed).

`pre-publish.sh` calls the iterate script (file-scoped). `mutants.out/` is gitignored. After every stop or clean file, snapshot into `docs/mutants/latest/`.

Restore after a wiped `mutants.out/`: the iterate script copies `docs/mutants/latest/caught.txt` (and timeout/unviable) back, then `--iterate`.

## File order (Stage-1)

1. `dot_unify.rs` — DOT edge unification
2. `graph/liveliness.rs` — liveliness machine (not the 8-line `graph_liveliness.rs` shim)
3. `core_rx_stream.rs`
4. `core_tx_stream.rs`

Stage-2 (if Stage-1 is too thin for an honest V2V Mutants B): `actor_builder/pack.rs`, `actor_builder/slot_budget.rs`, `graph/shutdown.rs`, then `core_rx.rs` / `core_tx.rs`.

## Skip policy

- TIMEOUT = killed. Hang-prone wait/shutdown mutants timed out (not skipped):
  - `StreamRx::shared_wait_shutdown_or_avail_units` (`delete !`)
  - `StreamTx<StreamIngress>::shared_wait_shutdown_or_vacant_units` / `shared_wait_empty` (`delete !`)
  - `StreamTx<StreamEgress>::shared_wait_shutdown_or_vacant_units` / `shared_wait_empty` (`delete !`)
  - `GraphLiveliness::wait_for_registrations` (`>` → `==`), `internal_request_shutdown` (`()`), `is_in_state` (`true`)
- `#[mutants::skip]` only on hang-prone `is_running` / `await_for_*` control flow, each skip listed here. None in Stage-1.
- Do not weaken production asserts to make a mutant “unviable.”

## Current score

<!-- CAMPAIGN_CURRENT_START -->
- **When:** 2026-09-17T15:57:16Z
- **git:** `3fcdfd5`  **crate:** 0.3.1  **file:** `core_tx_stream.rs`
- **caught:** 308  **missed:** 0  **timeout:** 8  **unviable:** 32
- **kill-rate:** 100.0%  (`(caught + timeout) / (caught + timeout + missed)`)
<!-- CAMPAIGN_CURRENT_END -->

## Campaign log

| Date | git | File | Caught | Missed | Timeout | Unviable | Kill-rate |
|------|-----|------|--------|--------|---------|----------|-----------|
<!-- CAMPAIGN_LOG_ROWS -->
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 308 | 0 | 8 | 32 | 100.0% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 289 | 1 | 8 | 22 | 99.7% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 285 | 1 | 8 | 22 | 99.7% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 274 | 1 | 7 | 21 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 263 | 1 | 6 | 21 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 255 | 1 | 6 | 21 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 230 | 1 | 6 | 20 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 228 | 1 | 6 | 16 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 225 | 1 | 6 | 13 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 220 | 1 | 6 | 10 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 219 | 1 | 6 | 9 | 99.6% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 207 | 1 | 6 | 9 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 201 | 1 | 6 | 9 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 196 | 1 | 5 | 8 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 192 | 1 | 5 | 8 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 188 | 1 | 4 | 8 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 186 | 1 | 4 | 8 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 182 | 1 | 4 | 8 | 99.5% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 168 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 166 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 164 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 163 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 158 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_tx_stream.rs | 155 | 1 | 4 | 8 | 99.4% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 152 | 0 | 4 | 6 | 100.0% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 143 | 1 | 4 | 6 | 99.3% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 142 | 1 | 4 | 6 | 99.3% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 133 | 1 | 4 | 6 | 99.3% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 128 | 1 | 4 | 6 | 99.2% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 123 | 1 | 4 | 6 | 99.2% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 112 | 1 | 4 | 4 | 99.1% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 107 | 1 | 3 | 4 | 99.1% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 94 | 1 | 3 | 4 | 99.0% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 89 | 1 | 3 | 3 | 98.9% |
| 2026-09-17 | `3fcdfd5` | core_rx_stream.rs | 78 | 1 | 3 | 2 | 98.8% |
| 2026-09-17 | `3fcdfd5` | graph/liveliness.rs | 78 | 0 | 3 | 2 | 100.0% |
| 2026-09-16 | `3fcdfd5` | graph/liveliness.rs | 60 | 1 | 2 | 2 | 98.4% |
| 2026-09-16 | `3fcdfd5` | graph/liveliness.rs | 33 | 1 | 1 | 1 | 97.1% |
| 2026-09-16 | `3fcdfd5` | graph/liveliness.rs | 32 | 1 | 1 | 1 | 97.1% |
| 2026-09-16 | `3fcdfd5` | graph/liveliness.rs | 28 | 1 | 0 | 1 | 96.6% |
| 2026-09-16 | `3fcdfd5` | graph/liveliness.rs | 26 | 1 | 0 | 1 | 96.3% |
| 2026-09-16 | `3fcdfd5` | dot_unify.rs | 16 | 0 | 0 | 1 | 100.0% |
| 2026-09-16 | `3fcdfd5` | dot_unify.rs | 15 | 1 | 0 | 1 | 93.8% |
| 2026-09-16 | `3fcdfd5` | dot_unify.rs | 12 | 1 | 0 | 1 | 92.3% |
| 2026-09-16 | `3fcdfd5` | dot_unify.rs | 4 | 1 | 0 | 0 | 80.0% |
| 2026-09-16 | `3fcdfd5` | dot_unify.rs | 0 | 1 | 0 | 0 | 0.0% |
