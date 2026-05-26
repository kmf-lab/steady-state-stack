# Dependencies and platform

**Who should read this:** Release engineers and platform porters.

**See also:** `core/Cargo.toml`, [Migration guide](../migration_guide.md).

---

ss[platform.ringbuf-pin]

`ringbuf` MUST stay on 0.4.x alongside `async-ringbuf` 0.3.5; bumping one without the other is MUST NOT in a single change.

**Tier:** 0

---

ss[platform.executor-features]

Exactly one primary executor feature (`proactor_nuclei`, `proactor_tokio`, or `exec_async_std`) MUST be enabled per build graph.

**Tier:** 0

---

ss[platform.windows-async-std]

Windows builds MUST use `exec_async_std` because io_uring is unavailable.

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
| `platform.executor-features` | One executor feature | 0 |
| `platform.windows-async-std` | Windows backend | 0 |
| `platform.coverage-merge` | Merged LCOV | 1 |
| `platform.aeron-out-of-scope-coverage` | Coverage exclusions | 2 |
