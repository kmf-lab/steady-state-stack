# Dynamic troupe packing

Startup schedule: map a **fixed** actor graph onto the OS threads this process actually has,
once, at `Graph::start`.

## What this is not

- Not Tokio work-stealing
- Not live re-pack when a pod’s CPU limit changes
- Not a way to parallelize a 3-actor chain onto 64 cores

Extra slots help **parallel width** (bundles / many workers). Replica count remains Kubernetes HPA.

## Run

```bash
cargo run -p steady_state --example dynamic_troupe -- --slots 5 --beats 8
SS_PACK_SLOTS=5 cargo run -p steady_state --example dynamic_troupe
```

`--slots` pins detected budget before subtracting already-spawned SoloAct threads (VIP + telemetry ≈ 3).
Default `--slots 5` typically leaves two packed OS threads in this example.

Watch the logs for `INGRESS` / `RELAY_*` / `WORKER` thread ids and the packing summary from `Graph::start`.

See also: [lesson-on-dynamic-troupes.md](../../../lesson-on-dynamic-troupes.md),
[docs/spec/13-troupe-packing.md](../../../docs/spec/13-troupe-packing.md).
