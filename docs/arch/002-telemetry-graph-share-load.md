# ADR 002: Telemetry graph-share Avg load

## Context

Avg **mCPU** is a local busy fraction (0–1024). Avg **load %** was wrongly computed as local CPU % after a rollup change, then broke further when the collector sent sparse batches (only a subset of actors per message). Consumers expected a hotspot metric: this actor’s share of all graph mCPU.

## Decision (0.3.1)

1. **Pass 1** — `apply_local_mcpu` updates each touched actor’s last-known mCPU.
2. **Pass 2** — `DotState::refresh_actor_loads` sums last-known mCPU across **all** defined nodes and sets  
   `load% = 100 × this_mcpu / Σ graph_mcpu` (sole actor → 100%).
3. Rolling samples for load accumulate only for actors touched in the current batch; load values themselves are recomputed for every node so sparse updates stay correct.

## Consequences

Avg load is a **graph-share** metric, not “how busy is this core.” Docs on `ActorBuilder::with_load_avg` and DOT tests under `build_load_tests.rs` lock that meaning. Do not reintroduce per-message-only totals.
