# Streams and distributed

**Who should read this:** Aeron/aqueduct integrators. Many requirements are **Tier 2** (integration waiver).

**See also:** [Distributed stub](../distributed.md), `core/src/distributed/`.

---

ss[distributed.aeron-uri]

Aeron channel builders MUST produce valid URI strings for publish/subscribe configuration documented in crate READMEs.

**Tier:** 2 — **integration waiver** until media-driver CI job exists.

---

ss[distributed.aqueduct-stream]

Aqueduct stream types MUST multiplex control and payload per stream channel semantics.

**Tier:** 2

---

ss[distributed.subscribe-publish]

Subscribe and publish bundles MUST wire lazy bundles through graph establishment like standard channels.

**Tier:** 2

---

ss[distributed.media-driver-testing]

`for_testing` graphs MUST skip or mock media driver requirements when `is_for_testing` is set.

**Tier:** 2

---

ss[stream.control-payload]

Stream ingress/egress MUST keep control items and byte payloads on paired buffers with independent capacity.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `distributed.aeron-uri` | Valid Aeron URIs | 2 |
| `distributed.aqueduct-stream` | Aqueduct multiplex | 2 |
| `distributed.subscribe-publish` | Pub/sub bundles | 2 |
| `distributed.media-driver-testing` | Test graph skips driver | 2 |
| `stream.control-payload` | Dual buffer streams | 1 |
