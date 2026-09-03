# ADR 001: Shadow vs spotlight

## Context

Every Steady actor needs a Graph-owned handle (for wiring and restart) and a live execution context (for channels, waits, and telemetry). Collapsing those into one type mixed orchestration with domain logic and broke simulation.

## Decision

- **`SteadyActorShadow`**: Graph handle. Cheap to clone; holds identity, liveliness, and channel blueprints. Used by `run()` and StageManager puppets.
- **`SteadyActorSpotlight`**: Active context from `into_spotlight`. Owns wait/telemetry hot paths. Implements `SteadyActor` for production/`internal_behavior`.
- **`run()` is a dispatcher only**: `use_internal_behavior` → `internal_behavior`; else → `simulated_behavior`. Unit tests call `internal_behavior` directly; they never call `run()`.

## Consequences

Two `SteadyActor` impls (shadow + spotlight) are intentional. Spotlight wait helpers live in `steady_actor_spotlight/wait.rs` so the spotlight module stays under the line budget without changing the trait surface.
