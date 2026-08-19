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

When `TELEMETRY_SERVER_PORT` is unset, the default port (9900) MAY auto-increment by one on `Address already in use`, up to 256 ports walked and below port 32768; the process MUST log the port used for that run. When `TELEMETRY_SERVER_PORT` is explicitly set, binding MUST NOT scan to alternate ports.

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

ss[telemetry.live-title]

The builtin telemetry viewer title MUST start as a loading placeholder. It MUST show **Live Telemetry** only after a successful recent `/graph.dot` pull that produced SVG. A failed pull (HTTP error, network failure, or render error) MUST set the title to **Snapshot** with no occurrence of the word Live, and MUST NOT replace the last successfully rendered diagram.

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
| `telemetry.live-title` | Live vs Snapshot title | 1 |
