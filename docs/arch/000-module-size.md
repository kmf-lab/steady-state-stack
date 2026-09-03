# ADR 000: Module size and split policy

## Context

V2V SubsystemsDesign and `verify.process.file-size` require Rust sources under `core/src/` to stay below **1,200 lines** (soft) and never exceed **1,800** (hard). Large “god files” made reviews and Tracey anchoring harder.

## Decision

1. **Production files** stay under 1,200 lines. Prefer a second `impl` / `#[path]` sibling over new directory nesting.
2. **Embedded `#[cfg(test)]` modules** move to `*_tests.rs` siblings declared with `#[path]` **inside** the parent so `super` still reaches private items.
3. **New directories** only when the type is already a public facade (`actor_builder/`, `steady_actor_spotlight/`).
4. CI runs `bash scripts/check-file-size.sh` on every PR.

## Consequences

- Splits are mechanical (no API change). Trait method bodies that cannot live in a submodule use thin delegates to a `wait.rs`-style helper module, or a module-level second `impl` only when the language allows it.
- Test files count toward the same caps; large suites are topic-split (e.g. `dot/tests/build_*_tests.rs`).
