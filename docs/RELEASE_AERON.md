# Aeron release checklist (Gate C)

Live Aeron pub/sub is **not** part of Gate A (nextest) or Gate B (llvm-cov). Use this checklist before tagging a release that depends on Aeron aqueducts.

## Coverage vs Gate C

| Gate | Command | Proves live `aeron_*` actors? |
|------|---------|-------------------------------|
| **A** | `cargo nextest --profile ci-unit` | No (instant skip) |
| **B** | `bash scripts/run-llvm-cov-release.sh` | No (~8–19% line % on `aeron_publish*` / `aeron_subscribe*` is **expected**) |
| **C** | `bash scripts/run-aeron-release-signoff.sh` | Yes (requires `aeronmd`) |

## Warmup model (Gate C)

1. **Script process:** `docker restart` → `aeron_preflight_smoke` (stream 80000) → `SS_AERON_PRE_SUITE_SETTLE_SEC` sleep.
2. **Suite process:** `suite_in_process_warmup` (same stream 80000, in-process wire proof) — **required** even when `SS_AERON_SCRIPT_PREFLIGHT_OK=1`.
3. **Scenarios:** each uses `fresh_stream_id()` (≥10000); bundle lane 1 tests warm lane 0 first.
4. **In-suite refresh (release):** restart after `ipc_single_ten` (2), bundle group (6), `shutdown_bundle` (9), and `backpressure_ipc` (10), plus explicit pre-UDP refresh; **no** restart after `ipc_single_hundred` (3). In-suite restarts use `SS_AERON_POST_RESTART_SETTLE_SEC` (default 15s) and subprocess `aeron_preflight_smoke`, matching the script.

`SS_AERON_SCRIPT_PREFLIGHT_OK` does **not** skip in-process warmup.

## Prerequisites

- Media driver installed: `core/routing_service/aeron/install_aeronmd.sh`
- Same OS user as tests and `aeronmd` (Docker/systemd)
- `/dev/shm/aeron-default` present and not abnormally large (`cnc.dat` ≤ ~64MB recommended)
- Self-hosted runner setup (CI): [AERON_RUNNER.md](AERON_RUNNER.md)

## Gate C release sign-off (required — full matrix)

**Canonical command** (IPC + UDP + multicast, 17 scenarios):

```bash
docker restart aeronmd && sleep 15
bash scripts/run-aeron-release-signoff.sh
```

This sets `SS_AERON_RELEASE=1`, `SS_AERON_MATRIX=full`, and expects **≥17** `PASS [scenario]` lines (15 IPC + 3 UDP/multicast when the full matrix runs; minimum enforced by `SS_AERON_MIN_PASS`).

### Scenarios (full matrix)

**IPC (15):** `ipc_single_one`, `ipc_single_ten`, `ipc_single_hundred`, `ipc_bundle_lane0`, `ipc_bundle_lane1`, `ipc_bundle_both_lanes`, `stream_id_mismatch`, `shutdown_single`, `shutdown_bundle`, `backpressure_ipc`, `aqueduct_single_start`, `aqueduct_bundle_start`, `aqueduct_all_impls`, `aqueduct_roundtrip`, `uri_live_transports`

**UDP (2):** `udp_p2p_roundtrip`, `udp_p2p_many_small`

**Multicast (1):** `multicast_roundtrip`

### Strict profile (`SS_AERON_RELEASE=1`)

| Variable | Value |
|----------|--------|
| `SS_AERON_REQUIRED` | 1 |
| `SS_AERON_ALLOW_SOFT_SKIP` | 0 |
| `SS_AERON_RETRY` | 0 |
| `SS_AERON_DOUBLE_PASS` | 0 |
| `SS_AERON_SCENARIO_RETRY` | 0 |
| `SS_AERON_MIN_PASS` | 17 (`full` matrix) or 14 (`ipc` only) |

**Pass criteria:**

- Exit code 0
- No `SKIP [aeron_integration_serial_suite]`
- ≥17 `PASS [scenario]` lines (full matrix)
- No `graph stop timed out or unclean` in log (unless `SS_AERON_ALLOW_UNCLEAN_SHUTDOWN=1`)

Save log: `SS_AERON_LOG_FILE=/tmp/aeron-release-signoff.log`

## Flake gate (before tag)

```bash
bash scripts/run-aeron-flake-check.sh
```

Runs `run-aeron-release-signoff.sh` **3 times** (fresh driver on run 1 only). All must pass.

## IPC-only sign-off (optional)

```bash
SS_AERON_RELEASE=1 SS_AERON_MATRIX=ipc ./scripts/run-aeron-integration.sh
```

Expect ≥14 `PASS [` lines.

## Examples smoke

After Gate C is green on the same host:

```bash
bash scripts/smoke-aeron-examples.sh
```

## CI

- **Gate A / B:** `.github/workflows/rust.yml` + `scripts/run-llvm-cov-release.sh`
- **Gate C:** `.github/workflows/aeron-integration.yml` on runner labels `[self-hosted, aeron]`

Do **not** run `cargo llvm-cov test --tests` for coverage; use `scripts/run-llvm-cov-release.sh` only.

## Bisect

```bash
SS_AERON_FRESH_DRIVER=0 SS_AERON_SCENARIO=ipc_single_one \
  cargo test -p steady_state --test aeron_integration_suite -- --nocapture
```
