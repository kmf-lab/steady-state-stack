# steady_state

[![Leaderboard](https://my.kmf-lab.com/leaderboard/static/badge/kmf-lab.svg)](https://my.kmf-lab.com/leaderboard/kmf-lab/steady-state-stack)

Framework for building long-running, low-latency actor-based services on Linux. Isolated actors, non-blocking async ring buffers, Erlang-style supervisors, and built-in visual telemetry.

## Add the dependency

```toml
[dependencies]
steady_state = "0.2"
```

```bash
cargo add steady_state
```

**Default features:** `telemetry_server_builtin`, `prometheus_metrics`, `core_display`, `core_affinity`.

Optional `tokio` puts a current-thread Tokio reactor on SOLO/TROUP OS threads (not a work-stealing pool). Full API: [docs.rs/steady_state](https://docs.rs/steady_state/0.2.13/steady_state/).

## Quick start

1. Follow **[Getting started](https://github.com/kmf-lab/steady-state-stack/blob/main/docs/getting_started.md)** — install, first actor, telemetry.
2. Clone and run **[steady-state-minimum](https://github.com/kmf-lab/steady-state-minimum)** for a single-actor heartbeat.
3. Skim the **[architecture TLDR](https://github.com/kmf-lab/steady-state-stack/blob/main/steady_state_tldr.md)** when you want the mental model (pull-reactor actors, veto shutdown, lazy channels).

## Lesson path

Each lesson is a full runnable project. Work them in order:

| Step | Lesson | What you learn |
|------|--------|----------------|
| 1 | [steady-state-minimum](https://github.com/kmf-lab/steady-state-minimum) | One actor, isolated state, timed shutdown |
| 2 | [steady-state-standard](https://github.com/kmf-lab/steady-state-standard) | Generator → worker → logger, batching, metrics |
| 3 | [steady-state-robust](https://github.com/kmf-lab/steady-state-robust) | Auto-restart, persistent state, peek-before-commit |
| 4 | [steady-state-performant](https://github.com/kmf-lab/steady-state-performant) | Large channels, double-buffering, zero-copy slices |
| 5 | [steady-state-distributed](https://github.com/kmf-lab/steady-state-distributed) | Publisher/subscriber pods over Aeron |

In-tree distributed example (same idea as the lesson): [core/examples/steady-state-distributed](https://github.com/kmf-lab/steady-state-stack/tree/main/core/examples/steady-state-distributed).

## Observability

With telemetry enabled, open:

- Dashboard: `http://127.0.0.1:9900`
- Prometheus: `http://127.0.0.1:9900/metrics`
- Graph DOT: `http://127.0.0.1:9900/graph.dot`

## Specification and verification

Normative requirements live in the monorepo:

- **[docs/spec/README.md](https://github.com/kmf-lab/steady-state-stack/blob/main/docs/spec/README.md)** — learning path (actors, channels, bundles, shutdown, testing, distributed)
- Code and tests link requirements with Tracey `ss[impl …]` / `ss[verify …]` comments

Deep dives in the stack repo: [bundles](https://github.com/kmf-lab/steady-state-stack/blob/main/lesson-on-bundles.md), [testing](https://github.com/kmf-lab/steady-state-stack/blob/main/lesson-on-testing.md), [proptest](https://github.com/kmf-lab/steady-state-stack/blob/main/lesson_on_proptest.md).

## Features at a glance

- **Actors** — `run` dispatcher + `internal_behavior` domain loop; unit tests call `internal_behavior`, not `run()`
- **Channels** — lazy until clone; backpressure; testing helpers `testing_send_all` / `testing_take_all`
- **State** — `SteadyState<S>` survives actor restarts
- **Throughput** — large batch channels, double-buffering, optional zero-copy `peek_slice` / `poke_slice`
- **Distributed** — Aeron IPC/UDP between pods

## Community

[![Dashboard](https://my.kmf-lab.com/leaderboard/static/badge/dashboard/kmf-lab.svg)](https://my.kmf-lab.com/leaderboard/dashboard?account=kmf-lab)
[![Honor board](https://my.kmf-lab.com/leaderboard/static/badge/honor/kmf-lab.svg)](https://my.kmf-lab.com/leaderboard/honor/kmf-lab)

- Monorepo: [kmf-lab/steady-state-stack](https://github.com/kmf-lab/steady-state-stack)
- [Discussions](https://github.com/kmf-lab/steady-state-stack/discussions)
- [Sponsor](https://github.com/sponsors/kmf-lab)

MIT licensed.
