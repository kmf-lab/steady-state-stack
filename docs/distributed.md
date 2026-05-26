# Distributed systems (guide)

Overview of Aeron and aqueduct integration. **Requirements:** [docs/spec/08-streams-and-distributed.md](spec/08-streams-and-distributed.md) (mostly Tier 2).

---

## When to use

Use distributed features when actors must communicate across processes via Aeron media driver and stream channels. Local graphs and unit tests typically use `GraphBuilder::for_testing()` without a live driver.

---

## Components

| Area | Location |
|------|----------|
| Aeron publish/subscribe | `core/src/distributed/aeron_*.rs` |
| Aqueduct streams | `core/src/distributed/aqueduct_*.rs` |
| Stream channels | `build_stream` in channel builder |

---

## Testing policy

Tier-2 requirements may use **integration waivers** until CI runs a media driver job. See [00-conventions](spec/00-conventions.md).

---

## See also

- Crate READMEs under `core/` for Aeron examples  
- [Specification: distributed](spec/08-streams-and-distributed.md)  
- [Channels guide](channels.md) — stream dual buffers
