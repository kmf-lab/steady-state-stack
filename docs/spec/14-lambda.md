# Lambda warm graph (no Tokio)

**Who should read this:** Anyone running Steady State inside an AWS Lambda sandbox (or similar freeze/thaw execution environments).

**See also:** [graph](05-graph-and-shutdown.md), [state](07-state.md), [troupe packing](13-troupe-packing.md), [platform](11-dependencies-and-platform.md), ADR [005-lambda-warm-graph](../arch/005-lambda-warm-graph.md).

**Honesty:** Freeze/thaw is the whole VM. Idle MUST be event-driven (`wait_avail` / `wait_vacant`). `/tmp` JSON does **not** survive a new sandbox. Shutdown on SIGKILL is best-effort. Extra pack slots help **width**, not a sequential pipeline. Cargo-lambda is a **packager**; it MUST NOT be the actor executor.

---

## Glossary

| Term | Meaning |
|------|---------|
| **Warm graph** | One graph per Lambda sandbox; stays `Running` across invokes |
| **Host bridge** | Blocking inject / wait-output between the Runtime API host and graph edges |
| **Persist-before-park** | Actor calls `StateGuard::persist()` on every path back to the wait |
| **Strict persist** | Dirty-at-park is a defect under `for_lambda` / `with_strict_persist` |

---

ss[graph.for-lambda]

`GraphBuilder::for_lambda` MUST configure graphs for Lambda-style sandboxes: telemetry metric features off, no fail-fast process exit, no test StageManager backplane, pack slots detected (cgroup / `SS_PACK_SLOTS`) unless pinned, a smaller default actor stack, and strict persist enabled. It MUST NOT blindly inherit host core counts for packing (same constraints as [13-troupe-packing](13-troupe-packing.md)).

**Tier:** 0

---

ss[graph.lambda.warm-graph]

A lambda-mode graph MUST live for the sandbox lifetime. Invoke handling MUST NOT call `request_shutdown` per request. The host ingress path MUST remain open between invokes so actors can park on `wait_avail` and resume on the next inject.

**Tier:** 0

---

ss[graph.lambda.host-not-executor]

The Lambda Runtime API host thread (or cargo-lambda Tokio worker, if used only as packager host) MUST NOT drive actor futures. Actor futures MUST stay on SOLO/TROUP OS threads. Host I/O for the Runtime API MUST be blocking (`call_blocking` or a dedicated host thread).

**Tier:** 0

---

ss[platform.lambda-no-tokio]

The default build and the optional `lambda` Cargo feature MUST NOT add Tokio to `steady_state`. The `lambda` feature MAY add only blocking HTTP (e.g. `ureq`). Enabling Steady’s `tokio` feature for Lambda is MUST NOT as the supported path.

**Tier:** 0

---

ss[graph.lambda.host-bridge]

The framework MUST provide a first-class host bridge for inject / await-output that is **not** `testing_send_all` and **not** StageManager. The host MUST inject on the host thread; actors MUST establish application channels on their packed OS thread.

**Tier:** 0

---

ss[graph.lambda.event-driven]

Lambda-mode graphs MUST NOT rely on `wait_periodic` or telemetry production ticks for progress. Wall clocks and monotonic timers lie across freeze/thaw. Progress MUST be driven by host inject and channel waits.

**Tier:** 0

---

ss[graph.lambda.runtime-api]

When the optional `lambda` feature is enabled, the crate MUST expose a blocking Runtime API client for `GET /runtime/invocation/next`, `POST …/response`, and `POST …/error`. It MUST NOT depend on the `lambda_runtime` crate.

**Tier:** 0

---

ss[state.persist-before-park]

When `on_persist` is configured, every path that returns to the actor’s wait point after mutating state MUST call `StateGuard::persist()`. Drop-on-exit MUST NOT be treated as sufficient for freeze/SIGKILL safety while the guard is held.

**Tier:** 0

---

ss[state.dirty-at-park]

`StateGuard` MUST track a dirty bit (set on `DerefMut`, cleared on successful `persist()`). Under `for_lambda` / `with_strict_persist`, arriving at the single wake point with a watched persistent guard that is dirty MUST be treated as a defect (debug panic / test failure).

**Tier:** 0

---

ss[state.persist-hooks]

The framework MUST provide `new_persistent_state_with` (or equivalent) that accepts user load/persist callbacks. File-backed `new_persistent_state` MAY remain. Lambda graphs MUST NOT assume process-local files (e.g. `/tmp`) outlive the sandbox.

**Tier:** 0

---

ss[graph.lambda.shutdown-hook]

Lambda graphs SHOULD register a SIGTERM / extension Shutdown path that calls `request_shutdown` with a sub-second drain bound. Clean shutdown while frozen on `/next` is best-effort.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `graph.for-lambda` | Lambda GraphBuilder preset | 0 |
| `graph.lambda.warm-graph` | One graph per sandbox | 0 |
| `graph.lambda.host-not-executor` | Host ≠ actor executor | 0 |
| `platform.lambda-no-tokio` | No Tokio for lambda path | 0 |
| `graph.lambda.host-bridge` | Inject / wait-output bridge | 0 |
| `graph.lambda.event-driven` | No periodic progress | 0 |
| `graph.lambda.runtime-api` | Blocking Runtime API client | 0 |
| `state.persist-before-park` | Persist before await | 0 |
| `state.dirty-at-park` | Dirty bit + strict check | 0 |
| `state.persist-hooks` | User persist callbacks | 0 |
| `graph.lambda.shutdown-hook` | Short SIGTERM drain | 1 |
