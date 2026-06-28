# Telemetry (guide)

Operational guide for metrics and observability. **Requirements:** [docs/spec/09-telemetry-and-observability.md](spec/09-telemetry-and-observability.md).

---

## Prometheus

Enable the `prometheus_metrics` feature on `steady_state`. Channel and actor statistics integrate with Prometheus collectors (`channel_stats`, `actor_stats`, `telemetry/metrics_collector.rs`).

---

## Builtin server

When configured through steady config / graph setup, a builtin HTTP server may expose scraped metrics. See `core/src/telemetry/metrics_server.rs` and `telemetry/setup.rs`.

---

## DOT export

Graph topology can be exported as DOT for documentation (`core/src/dot/` — `build.rs`, `register.rs`, `history.rs`; plus `dot_node.rs`, `dot_edge.rs`, `dot_unify.rs`).

---

## Shutdown visibility

Unclean shutdown records veto reasons and optional backtraces via graph liveliness reporting (`core/src/graph/shutdown.rs`; public path remains `graph_liveliness` re-exports).

---

## See also

- [Specification: telemetry](spec/09-telemetry-and-observability.md)  
- [Getting started](getting_started.md)
