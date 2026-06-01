# Steady State integration tests

## Aeron pub/sub (`aeron_integration_suite`)

Live-driver verification runs in a **single serial test binary** so scenarios share one `aeronmd` session without cross-binary pollution.

### Prerequisites

**Recommended (Docker + systemd):** install the media driver service, then run tests as the same user:

```bash
cd core/routing_service/aeron
sudo bash install_aeronmd.sh "$USER"
systemctl status aeronmd
```

`./scripts/run-aeron-integration.sh` auto-detects `aeronmd.service` and uses **`systemctl restart aeronmd`**, falling back to **`docker restart aeronmd`** when systemctl times out or needs elevated permissions (Docker install path). In-suite `REFRESH:` uses the same backend.

**Native binary (optional):** set `SS_AERON_DRIVER=binary` and `AERONMD` to an executable path (`test -x "$AERONMD"`).

Install details: [core/routing_service/aeron/README_linux.md](../routing_service/aeron/README_linux.md).

### Release gates

| Gate | Command |
|------|---------|
| A — unit + Tracey | `cargo nextest run --profile ci-unit` + `scripts/tracey-*-gate.sh` |
| B — coverage | `bash scripts/run-llvm-cov-release.sh` (no live `aeron_integration_suite`) |
| B — avoid | `cargo llvm-cov test --tests` runs **all** test binaries including live Aeron — use Gate C instead |
| C — live Aeron | `./scripts/run-aeron-integration.sh` |
| C — release (full matrix) | `bash scripts/run-aeron-release-signoff.sh` |
| C — flake gate | `bash scripts/run-aeron-flake-check.sh` (3× sign-off) |

Self-hosted CI: [docs/AERON_RUNNER.md](../docs/AERON_RUNNER.md)

### Validation (2026-05-28)

| Check | Result |
|-------|--------|
| `bash scripts/tracey-ci-validate.sh` | PASS |
| `bash scripts/tracey-unmapped-gate.sh` | PASS (88% mapped on rust-core) |
| `bash scripts/tracey-untested-gate.sh` | PASS (`distributed`, `stream.control-payload`) |
| `cargo test -p steady_state --lib aeron_subscribe_state_tests` | PASS |
| `cargo test -p steady_state --test aeron_integration_uri_contract` | PASS (incl. proptest) |
| `./scripts/run-aeron-integration.sh` with `SS_AERON_SCENARIO=ipc_single_one` | PASS (systemd + `docker restart aeronmd` fallback when `systemctl restart` times out) |
| `SS_AERON_RELEASE=1 ./scripts/run-aeron-integration.sh` | 15 `PASS [` lines, suite ok (2026-05-28 lab, `docker restart aeronmd` first) |
| `bash scripts/run-aeron-release-signoff.sh` | Required before release (≥17 `PASS [` full matrix); self-hosted CI: `.github/workflows/aeron-integration.yml` |

Full serial suite: run locally with `./scripts/run-aeron-integration.sh` (~15–30 min). Requires `aeronmd.service` or spawnable `AERONMD`; same OS user as the service.

Full matrix release: `bash scripts/run-aeron-release-signoff.sh` or `./scripts/run-aeron-full-matrix.sh` on lab/self-hosted runner with UDP/multicast enabled.

### Running

```bash
# Canonical live suite (one process, ordered scenarios)
cargo test -p steady_state --features exec_async_std \
  --test aeron_integration_suite -- --nocapture

# Or use the helper script (sets RUST_LOG=info)
./scripts/run-aeron-integration.sh

# URI builder tests (no media driver required)
cargo test -p steady_state --features exec_async_std --test aeron_integration_uri_contract
```

**Do not** run multiple `aeron_integration_*` driver binaries in one `cargo test` invocation — `cargo test` orders test **binaries** alphabetically and they contend on one driver. Use only `aeron_integration_suite` for live Aeron.

**Gate A vs Gate C:** `cargo nextest run --profile ci-unit` and `pre-publish.sh` must **not** run the live driver suite. Even with `aeronmd` running, `aeron_integration_serial_suite` and `aeron_preflight_wire_settle` skip instantly unless `SS_AERON_GATE_C=1` (set automatically by `./scripts/run-aeron-integration.sh`). Without this guard, pre-publish can appear to hang for 10+ minutes.

### Zero coverage (false green — fixed in Gate C script)

If the log shows `SKIP [aeron_integration_serial_suite]` with **no** `PASS [` lines, the driver preflight failed and **no scenarios ran**. Older scripts still exited 0; `./scripts/run-aeron-integration.sh` now **fails** unless `SS_AERON_ALLOW_SOFT_SKIP=1`. Fix the driver (`docker restart aeronmd`, wait for wire settle) or set `SS_AERON_REQUIRED=1` to fail fast in `cargo test`.

### Skipped (no driver)

When `aeronmd` is not running, the suite prints a block like:

```
======================================================================
SKIP [aeron_integration_serial_suite]
  Reason: Aeron media driver not available after ...
  Hint: ...
  Install: core/routing_service/aeron/README_linux.md
  Note: Test passes without running (soft skip). Use SS_AERON_REQUIRED=1 to fail instead.
======================================================================
```

This is intentional for local development without a driver.

### Failed (driver up)

1. Run with `RUST_LOG=info RUST_BACKTRACE=1`.
2. Read `AeronTestError` output: `phase=`, `stream_id=`, `channel=`, `ingress_avail=`.
3. **`ingress_avail=0` on `ipc_single_one`** — stale CNC or in-process suite warmup failed (script smoke alone is not enough). `docker restart aeronmd && sleep 20`, then rerun.
4. **`ingress_avail=0` on `ipc_bundle_lane1`** — lane 1 IPC not connected; harness warms lane 0 first; subscribe bundle polls during bootstrap. Bisect: `SS_AERON_SCENARIO=ipc_bundle_lane1`.
5. **`ingress_avail=0` + `[driver_refresh]`** — in-suite restart wire probe failed; uses subprocess `aeron_preflight_smoke` then in-process fallback; release refreshes after ten, bundle group, and backpressure (not after hundred).
6. Checklist: same OS user as the service; `/dev/shm/aeron-default` exists; `cnc.dat` not abnormally large.
7. Release (`SS_AERON_RELEASE=1`): no auto-retry — use `bash scripts/run-aeron-flake-check.sh` (3/3) after a green sign-off.

### Environment

| Variable | Effect |
|----------|--------|
| `SS_AERON_GATE_C=1` | Allow live Aeron test binaries to run (set by `run-aeron-integration.sh`; **off** in Gate A nextest) |
| `SS_AERON_SCRIPT_PREFLIGHT_OK=1` | Set by script after `aeron_preflight_smoke`; suite still runs **in-process** `suite_in_process_warmup` |
| `SS_AERON_PRE_SUITE_SETTLE_SEC` | Sleep before `cargo test` suite (default 12 release / 10 dev); not duplicated inside suite |
| `SS_AERON_REQUIRED=1` | Fail (panic) if probe fails instead of soft skip; script fails on full-suite soft-skip |
| `SS_AERON_ALLOW_SOFT_SKIP=1` | Allow script exit 0 when full suite soft-skips (no `PASS [` lines) |
| `SS_AERON_MATRIX` | `ipc` (default), `full`/`all` (UDP+multicast), or `udp` |
| `SS_AERON_POST_RESTART_SETTLE_SEC` | Sleep after driver restart (default 15); script runs `aeron_preflight_smoke` after restart |
| `SS_AERON_RESTART_VIA` | `auto` (default), `docker`, `systemctl`, or `binary` — **docker first** when container `aeronmd` exists (no sudo) |
| `SS_AERON_SYSTEMD_UNIT` | systemd unit / Docker container name (default `aeronmd`) |
| `SS_AERON_REFRESH_MINIMAL` | `1` = in-suite cooldown only, no driver restart between scenarios |
| `SS_AERON_FRESH_DRIVER` | Default **on** (`1`); set `0` to skip script restart before the suite |
| `SS_AERON_RETRY` | Default **on** (`1`); set `0` to disable one automatic restart+rerun on `ingress_avail=0` |
| `SS_AERON_DOUBLE_PASS` | Default **on** (`1`); run the serial suite twice in one script invocation without restarting `aeronmd` |
| `SS_AERON_SCENARIO_RETRY` | Default **on** (`1`); one driver refresh + rerun on `phase=Wire` / `phase=Recv` failures |
| `SS_AERON_RELEASE` | `1` = release sign-off profile (`REQUIRED`, no retries, `MIN_PASS=14`, fail on unclean shutdown WARN) |
| `SS_AERON_MIN_PASS` | Minimum `PASS [` lines required when `SS_AERON_REQUIRED=1` and `SS_AERON_MATRIX=ipc` (default 14) |
| `SS_AERON_ALLOW_UNCLEAN_SHUTDOWN` | `1` = do not fail script on unclean shutdown WARN in log |
| `SS_AERON_SCENARIO=<name>` | Run only one scenario (e.g. `ipc_single_ten`) for bisect |
| `SS_AERON_UDP=1` | Enable UDP point-to-point scenarios (off by default; some hosts need explicit enable) |
| `SS_AERON_MULTICAST=1` | Enable multicast scenario in the serial suite |
| `RUST_LOG=info` | Actor and graph diagnostics on failure |

Full matrix (IPC + UDP + multicast): `./scripts/run-aeron-full-matrix.sh` (`SS_AERON_MATRIX=full`, `SS_AERON_REQUIRED=1`)

Release sign-off (see [docs/RELEASE_AERON.md](../docs/RELEASE_AERON.md)):

```bash
docker restart aeronmd && sleep 15
bash scripts/run-aeron-release-signoff.sh
# Expect ≥17 PASS [scenario] lines (full matrix) — not only SKIP
```

Gate B coverage: `bash scripts/run-llvm-cov-release.sh` — **not** `cargo llvm-cov test --tests`.

### Bisect matrix (isolation)

| Run | Expected |
|-----|----------|
| `SS_AERON_SCENARIO=ipc_single_ten` + fresh driver | PASS → harness OK for that scenario |
| `ipc_single_one` then full suite in one process | Reproduces mid-suite flake if driver lifecycle |
| `RUST_LOG=info,steady_state::distributed=debug` on fail | Registration / “Running publish” / “running subscriber” |

```bash
SS_AERON_FRESH_DRIVER=1 SS_AERON_SCENARIO=ipc_single_ten \
  cargo test -p steady_state --features exec_async_std --test aeron_integration_suite -- --nocapture
```

Log grep targets: `Failed to add exclusive publication`, `Publication unavailable`, `new subscription registered`, `graph stopped uncleanly`.

### Examples smoke (after suite is green)

```bash
# Terminal 1 — subscriber
cargo run -p steady_state --features exec_async_std --example simple_aeron_subscriber

# Terminal 2 — publisher
cargo run -p steady_state --features exec_async_std --example simple_aeron_publisher
```

### Reusable harness

Copy patterns from [common/support/pub_sub_harness.rs](common/support/pub_sub_harness.rs).

Library probe: `steady_state::media_driver_probe_with_reason(Duration::from_secs(5))`.
