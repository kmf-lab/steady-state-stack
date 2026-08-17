# Dependencies and platform

**Who should read this:** Release engineers and platform porters.

**See also:** `core/Cargo.toml`, [Migration guide](../migration_guide.md).

---

ss[platform.ringbuf-pin]

`ringbuf` MUST stay on 0.4.x alongside `async-ringbuf` 0.3.5; bumping one without the other is MUST NOT in a single change.

**Tier:** 0

---

ss[platform.executor-features]

The default build MUST drive actor futures with nestable `futures_lite::future::block_on` on the SOLO/TROUP OS thread. The optional `tokio` Cargo feature MUST only install a **current-thread** Tokio runtime inside that same `block_on` (I/O reactor on the pinned thread). It MUST NOT spawn actors onto a Tokio work-stealing pool and MUST NOT require `Send` on actor futures. Default `cargo tree -p steady_state` MUST NOT include `tokio`.

**Tier:** 0

---

ss[platform.coverage-merge]

Release coverage interpretation MUST merge the same feature-set LCOV runs as `pre-publish.sh`, not a single default-feature run alone.

**Tier:** 1

---

ss[platform.aeron-out-of-scope-coverage]

Aeron/aqueduct integration and `simulate_edge` MAY remain below strict coverage thresholds until dedicated CI (CHANGELOG policy).

**Tier:** 2

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `platform.ringbuf-pin` | ringbuf 0.4 pin | 0 |
| `platform.executor-features` | Bare-metal block_on; optional tokio reactor | 0 |
| `platform.coverage-merge` | Merged LCOV | 1 |
| `platform.aeron-out-of-scope-coverage` | Coverage exclusions | 2 |
