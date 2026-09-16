# ADR 005: Lambda warm graph (no Tokio)

## Context

AWS Lambda freezes and thaws the whole execution environment between invokes. Steady State already maps actors to OS threads (SOLO/TROUP) that park at 0% CPU on `wait_avail`. That matches freeze/thaw better than a Tokio work-stealing pool: idle waiters resume on the same thread with the same channels and `SteadyState`.

Cargo-lambda templates pull in Tokio for the Runtime API. Steady’s contract forbids using that runtime as the actor executor. Authors need a first-class preset and a host bridge so a warm graph can accept many injects without `testing_send_all` hacks or per-invoke rebuild/shutdown.

Durable state has a hole: lock-first holds `StateGuard` for the whole `internal_behavior`, so persist-on-drop never runs while parked. Freeze or SIGKILL loses dirty state unless actors `persist()` before every park.

## Decision

1. **`GraphBuilder::for_lambda()`** — telemetry off, no fail-fast exit, no test backplane, pack slots from cgroup/`SS_PACK_SLOTS`, smaller stacks, **strict persist** on. Tests use `for_testing().with_strict_persist()` (and `with_pack_slots`) rather than calling `for_lambda` under `cfg(test)`.
2. **Warm graph** — one graph per sandbox; host ingress stays open; no `request_shutdown` per invoke. Dynamic troupes pack at `start` as today.
3. **Host bridge** — `LambdaHost` / ingress / egress with blocking inject and wait-output on the host thread. Not StageManager, not lazy `testing_send_all`.
4. **Persist-before-park** — dirty bit on `StateGuard`; `new_persistent_state_with` for user backends (Dynamo/S3/EFS via blocking I/O in the app). No AWS SDK in core. Strict mode fails dirty-at-park.
5. **Optional `lambda` feature** — blocking HTTP (`ureq`) for Runtime API only. Default tree stays Tokio-free. No `lambda_runtime` crate.
6. **No Suspended liveliness state** — sleep already means power may vanish; a second shutdown machine does not help SIGKILL.

## Consequences

- Same binary can run on 1–2 Lambda vCPUs with packing; extra memory/vCPU helps parallel width only.
- Authors must write event-driven actors (no heartbeats for progress) and call `persist()` before park when durable.
- Cargo-lambda remains optional packaging; Steady-native Runtime API is the supported zero-Tokio path.

## Alternatives rejected

- Per-invoke graph build/shutdown: wastes cold path; fights warm reuse.
- `SuspendRequested` graph state: duplicates shutdown, does not flush held guards.
- Embedding `aws-sdk-*` / `lambda_runtime`: reintroduces Tokio into the core dependency graph.
- Using StageManager / `testing_send_all` as production inject: wrong ownership and establish semantics.
