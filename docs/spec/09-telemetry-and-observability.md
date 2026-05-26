# Telemetry and observability

**Who should read this:** Operators enabling Prometheus, DOT export, and shutdown metrics.

**See also:** [Telemetry stub](../telemetry.md), `core/src/telemetry/`, `dot.rs`.

---

ss[telemetry.prometheus-metrics]

With `prometheus_metrics` feature, channel and actor stats MUST expose Prometheus-compatible collectors.

**Tier:** 1

---

ss[telemetry.builtin-server]

The framework MAY start a builtin metrics HTTP server when configured via steady config / graph options.

**Tier:** 1

---

ss[telemetry.dot-export]

Graph DOT export MUST represent nodes and edges sufficient to reproduce topology documentation.

**Tier:** 1

---

ss[telemetry.shutdown-complete]

Shutdown telemetry MUST record completion or veto reasons for unclean stops.

**Tier:** 1

---

ss[telemetry.channel-labels]

Prometheus label suffix behavior for channel stats MUST remain stable for dashboards (see channel_stats tests).

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `telemetry.prometheus-metrics` | Prometheus feature | 1 |
| `telemetry.builtin-server` | HTTP metrics server | 1 |
| `telemetry.dot-export` | DOT graphs | 1 |
| `telemetry.shutdown-complete` | Stop telemetry | 1 |
| `telemetry.channel-labels` | Label stability | 1 |
