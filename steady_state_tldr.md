# Steady State TLDR

Short architecture overview. **Normative requirements live in [docs/spec/README.md](docs/spec/README.md).**

---

## Core idea

Steady State is a pull-reactor actor framework: actors sleep until intent and resources align, then run a tight loop with channels locked up front and shutdown negotiated by veto.

---

## Actor shape

- **`run`** — dispatcher: shadow → spotlight, production vs simulation.  
- **`internal_behavior`** — domain logic; lock channels once; `while actor.is_running(|| veto) { await_for_all!(...); work }`.  

See [02-actor](docs/spec/02-actor.md) and [manifesto](steady_state_manifesto.md).

---

## Channels

- Build **lazy** in the graph; **establish on clone** to the actor thread.  
- Backpressure: never drop queued messages.  
- Unit tests use `testing_send_all` / `testing_take_all`; call **`internal_behavior`**, not `run()`.  

See [03-channel](docs/spec/03-channel.md), [06-testing](docs/spec/06-testing-and-simulation.md).

---

## Graph & shutdown

- `GraphBuilder::for_testing()` for tests.  
- Shutdown: actors vote; veto if work remains (`rx.is_closed_and_empty()` && `tx.mark_closed()` pattern).  

See [05-graph-and-shutdown](docs/spec/05-graph-and-shutdown.md).

---

## Bundles & state

- **Bundles:** `const GIRTH`, lazy → clone, index waits pick a ready lane ([04-bundle-and-index](docs/spec/04-bundle-and-index.md)).  
- **State:** `SteadyState<S>` in the graph survives actor restarts ([07-state](docs/spec/07-state.md)).

---

## Next steps

1. [Specification README](docs/spec/README.md) — full requirement index  
2. [Getting started](docs/getting_started.md)  
3. [Actor lifecycle](docs/actor_lifecycle.md) · [Channels](docs/channels.md) · [Testing](docs/testing.md)
