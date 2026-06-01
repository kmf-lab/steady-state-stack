# Streams and distributed

**Who should read this:** Aeron/aqueduct integrators. Many requirements are **Tier 2** (integration waiver).

**See also:** [Distributed stub](../distributed.md), `core/src/distributed/`.

---

ss[distributed.aeron-uri]

Aeron channel builders MUST produce valid URI strings for publish/subscribe configuration documented in crate READMEs.

**Tier:** 2 — **integration waiver** until media-driver CI job exists.

---

ss[distributed.aqueduct-stream]

Aqueduct stream types MUST multiplex control and payload per stream channel semantics.

**Tier:** 2

---

ss[distributed.subscribe-publish]

Subscribe and publish bundles MUST wire lazy bundles through graph establishment like standard channels.

**Tier:** 2

---

ss[distributed.media-driver-testing]

`for_testing` graphs MUST skip or mock media driver requirements when `is_for_testing` is set.

**Tier:** 2

---

ss[stream.control-payload]

Stream ingress/egress MUST keep control items and byte payloads on paired buffers with independent capacity.

**Tier:** 1

---

## Tracey baseline (2026-05-28)

Recorded with `tracey query validate`, `uncovered --prefix distributed`, and `untested --prefix distributed` after linkage and audit pass:

| Requirement | `ss[impl]` (rust-core) | `ss[verify]` (rust-core + rust-core-tests) | Tracey |
|-------------|------------------------|--------------------------------------------|--------|
| `distributed.aeron-uri` | `aeron_channel_builder`, structs | channel unit tests, `aeron_integration_uri_contract`, suite | covered, verified |
| `distributed.aqueduct-stream` | `aqueduct_stream`, `aqueduct_builder` | aqueduct unit tests, builder tests, suite `aqueduct_*` | covered, verified |
| `distributed.subscribe-publish` | publish/subscribe actors + bundles + aqueduct | harness, bundles, publish/subscribe state unit tests, suite | covered, verified |
| `distributed.media-driver-testing` | `graph_liveliness`, probe, timeouts | `polling`, `scheduling`, `aeron_gate`, driver helpers, suite | covered, verified |
| `stream.control-payload` | serialize + aqueduct dual-buffer types (`ss[depends]` on `StreamIngress`/`StreamEgress`) | `byte_buffer_packer` unit tests | covered, verified |

**CI gates:** `scripts/tracey-ci-validate.sh`, `scripts/tracey-unmapped-gate.sh` (≥80% mapped on rust-core), `scripts/tracey-untested-gate.sh` (0 untested for `distributed` and `stream.control-payload` prefixes).

**Process waivers (not blocking):** `verify.process.proptest`, `verify.process.fuzz` — see [12-verification-stack](12-verification-stack.md).

---

## Verification gates (release)

| Gate | Command | Includes live Aeron? |
|------|---------|----------------------|
| **A** — unit + Tracey | `cargo nextest run --profile ci-unit`, `scripts/tracey-*-gate.sh` | No live driver I/O (`SS_AERON_GATE_C` unset; suites skip in &lt;1s via [`.config/nextest.toml`](../../.config/nextest.toml) + gate in [`aeron_gate.rs`](../../core/tests/common/aeron_gate.rs)) |
| **B** — merged coverage | `bash scripts/run-llvm-cov-release.sh` | No (lib + `aeron_integration_uri_contract` only) |
| **C** — live pub/sub | `bash scripts/run-aeron-integration.sh` or `bash scripts/run-aeron-release-signoff.sh` (full matrix) | Yes (requires `aeronmd` / Docker container) |

Gate C is required for production Aeron deployments. **Release** sign-off uses the full matrix (IPC + UDP + multicast, ≥17 `PASS [` lines) on a self-hosted runner — see [AERON_RUNNER.md](../AERON_RUNNER.md) and [RELEASE_AERON.md](../RELEASE_AERON.md). Gate B must not fail because Gate C was omitted from the same `cargo test --tests` invocation.

---

## Production readiness (IPC pub/sub)

| Criterion | Required |
|-----------|----------|
| Gate C passes on **first run** (no `SS_AERON_RETRY` dependency) | Yes |
| Gate C log contains ≥1 `PASS [` scenario line (not full-suite `SKIP`) | Yes |
| `scripts/run-aeron-integration.sh` fails on full-suite soft-skip unless `SS_AERON_ALLOW_SOFT_SKIP=1` | Yes |
| Same OS user as media driver (Docker/systemd install) | Yes |
| Strict graph shutdown on normal roundtrip scenarios (no stop timeout) | Yes |
| Aqueduct *start-only* scenarios (`aqueduct_*_start`, `aqueduct_all_impls`) may use lenient shutdown | Yes |
| Integration graphs use `GraphBuilder::for_testing()` (no telemetry voters on shutdown) | Yes |
| Release sign-off: `bash scripts/run-aeron-release-signoff.sh` (full matrix, ≥17 `PASS [`); flake: `bash scripts/run-aeron-flake-check.sh` | Yes |
| In-process `suite_in_process_warmup` in suite when `SS_AERON_SCRIPT_PREFLIGHT_OK=1` (script smoke ≠ suite process ready) | Yes |
| Self-hosted CI Gate C (`.github/workflows/aeron-integration.yml`, labels `self-hosted`, `aeron`) | Yes |
| `SS_AERON_UDP=1` / multicast validated on release (included in full matrix) | Yes for network transports |
| Tracey: 0 untested for `distributed.*` and `stream.control-payload` | Yes (maintain) |

Driver restart for tests: prefer `SS_AERON_RESTART_VIA=docker` when `install_aeronmd.sh` was used; use `SS_AERON_REFRESH_MINIMAL=1` for local iteration without repeated restarts. Release sign-off (`SS_AERON_RELEASE=1`) refreshes in-suite after ten (2), bundle group (6), shutdown_bundle (9), and backpressure (10), not after hundred (3); in-suite restarts use `SS_AERON_POST_RESTART_SETTLE_SEC` (default 15) and subprocess `aeron_preflight_smoke`.

---

## Acceptance (verification matrix)

Live-driver scenarios run in [`core/tests/aeron_integration_suite.rs`](../../core/tests/aeron_integration_suite.rs) (serial, one `aeronmd` session). URI contracts run without a driver in [`core/tests/aeron_integration_uri_contract.rs`](../../core/tests/aeron_integration_uri_contract.rs).

### `distributed.subscribe-publish`

| Scenario | Mandatory (Gate C default) | Proves |
|----------|----------------------------|--------|
| `ipc_single_one`, `ipc_single_ten`, `ipc_single_hundred` | Yes | IPC single-stream pub/sub roundtrip |
| `ipc_bundle_lane0`, `ipc_bundle_lane1`, `ipc_bundle_both_lanes` | Yes | Bundle lane pub/sub |
| `stream_id_mismatch` | Yes | Mismatched stream IDs do not deliver |
| `shutdown_single`, `shutdown_bundle` | Yes | Clean graph shutdown with Aeron actors |
| `backpressure_ipc` | Yes | Sustained send with ingress pacing |
| `uri_live_transports` | Yes | `AeronConfig` channels on live driver (IPC; +UDP when matrix enables) |
| `ipc_wire_only` | No (bisect: `SS_AERON_SCENARIO=ipc_wire_only`) | Isolated wire probe |
| `udp_p2p_roundtrip`, `udp_p2p_many_small` | No (`SS_AERON_MATRIX=full` or `SS_AERON_UDP=1`) | UDP P2P pub/sub |
| `multicast_roundtrip` | No (`SS_AERON_MATRIX=full` or `SS_AERON_MULTICAST=1`) | Multicast pub/sub |

### `distributed.aqueduct-stream`

| Scenario | Mandatory (Gate C default) | Proves |
|----------|----------------------------|--------|
| `aqueduct_single_start`, `aqueduct_bundle_start` | Yes | Aqueduct graph start (single + bundle) |
| `aqueduct_all_impls` | Yes | All aqueduct actor wiring variants start |
| `aqueduct_roundtrip` | Yes | Aqueduct + Aeron roundtrip |
| Bundle IPC scenarios | Yes | Multiplexed lanes over shared channel |

### `distributed.aeron-uri`

| Test / scenario | Proves |
|-----------------|--------|
| `aeron_integration_uri_*` (contract tests) | IPC/UDP/multicast URI strings and builder enums |
| `uri_live_transports` (suite) | `AeronConfig`-built channels on live driver (IPC; UDP when matrix enables) |
| UDP/multicast suite scenarios | URI forms used on live driver |

### `distributed.media-driver-testing`

| Code / test | Proves |
|-------------|--------|
| `GraphBuilder::for_testing` / `aeron_init_timeouts` | Short driver wait budget in tests |
| `media_driver_probe_with_reason` | CNC probe API |
| `should_skip_entire_suite` / `SS_AERON_REQUIRED` | Soft skip vs hard fail when driver down |
| `preflight_ipc_roundtrip` / `aeron_preflight_smoke` | CNC + IPC wire settle before serial scenarios (script may run smoke test after driver restart) |

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `distributed.aeron-uri` | Valid Aeron URIs | 2 |
| `distributed.aqueduct-stream` | Aqueduct multiplex | 2 |
| `distributed.subscribe-publish` | Pub/sub bundles | 2 |
| `distributed.media-driver-testing` | Test graph skips driver | 2 |
| `stream.control-payload` | Dual buffer streams | 1 |
