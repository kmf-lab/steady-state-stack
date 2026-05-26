# Steady State specification conventions

**Who should read this:** Anyone authoring or annotating requirements, or interpreting Tracey reports.

---

## Requirement language

- **MUST** / **MUST NOT** — normative; Tier 0 failures block release until implemented and verified (or waived in writing).
- **SHOULD** — Tier 1; tracked in Tracey; waivable for one release cycle with a linked issue.
- **MAY** — Tier 2 or documentation-only; integration-only areas may lack unit `ss[verify]` if waived below.

Each requirement is defined with a column-0 marker:

```markdown
ss[domain.requirement-id]

The system MUST ...
```

Requirement IDs use dot-separated segments. Each segment contains only ASCII letters, digits, `-`, and `_`.

---

## Tiers

| Tier | Meaning | Tracey gate |
|------|---------|-------------|
| **0** | Core framework contract for every release | Must have `ss[impl]` and `ss[verify]` (or documented waiver) |
| **1** | Strongly recommended; tooling / ergonomics | Tracked; waivable short-term |
| **2** | Integration, media driver, examples | Requirement exists; verify may be unit-only + **integration waiver** |

Tier is noted in each domain file’s requirement index table.

---

## Annotation verbs (Rust)

| Verb | Comment form | Meaning |
|------|----------------|---------|
| impl | `// ss[impl domain.id]` | Code implements the requirement |
| verify | `// ss[verify domain.id]` | Test proves the requirement |
| depends | `// ss[depends domain.id]` | Re-check if requirement changes |
| related | `// ss[related domain.id]` | Loose link for reviewers |

Prefix is **`ss`**, inferred from requirement markers in spec files (e.g. `ss[actor.lock-first.channels]`). Config: [`.config/tracey/config.styx`](../../.config/tracey/config.styx).

---

## Waiver policy

Document waivers in this file or in the requirement’s **Acceptance** section:

| Waiver type | When allowed | What to record |
|-------------|--------------|----------------|
| **integration** | Tier 2 (Aeron live, media driver) | Req ID, reason, planned CI job |

**Active Tier-2 integration waivers (rust-core):** `distributed.aeron-uri`, `distributed.aqueduct-stream`, `distributed.subscribe-publish`, `distributed.media-driver-testing`, `stream.control-payload`, `platform.aeron-out-of-scope-coverage` — spec present; unit stubs only until media-driver CI.

**Active Tier-1 process waivers:** `verify.process.proptest`, `verify.process.fuzz`, `verify.process.mutants`, `verify.process.llvm-cov`, `verify.process.tracey-gate` — documented in `12-verification-stack.md`; `verify.process.nextest` covered by CI workflow.
| **temporary** | Tier 1 not yet testable | Issue URL, target release |
| **process** | `12-verification-stack` (proptest/fuzz) | Spec defines target; impl follow-on |

Waivers do **not** remove requirements from the spec; they defer `ss[verify]` until infrastructure exists.

---

## Source-of-truth hierarchy

When narrative docs disagree:

1. **`docs/spec/`** (this tree) — normative requirements
2. **CHANGELOG (Unreleased)** — API semantics for in-flight work
3. **Lessons / `docs/*.md`** — teaching; link to spec, do not duplicate MUST text
4. **`steady_state_manifesto.md`** — philosophy; encoded as `philosophy.*` reqs

---

## Implementations

| Impl name | Glob | Crate |
|-----------|------|-------|
| `rust-core` | `core/src/**/*.rs` | `steady_state` |
| `rust-cli` | `cargo-steady-state/src/**/*.rs` | `cargo-steady-state` |

Examples under `core/examples/` are out of scope unless a future `rust-examples` impl is added.

---

## Requirement index (conventions)

| ID | Summary | Tier |
|----|---------|------|
| *(meta)* | This document defines tiers, IDs, waivers | — |
