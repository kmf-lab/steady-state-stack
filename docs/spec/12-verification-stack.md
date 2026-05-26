# Verification stack (process)

**Who should read this:** CI maintainers adopting nextest, llvm-cov, proptest, fuzz, and mutants.

**Note:** Process requirements here define targets; full tool adoption is follow-on work per [00-conventions](00-conventions.md) waivers.

---

ss[verify.process.nextest]

CI and release scripts SHOULD run workspace tests via `cargo nextest` for parallelism and clearer failure reports.

**Tier:** 1

---

ss[verify.process.llvm-cov]

Coverage gates SHOULD use `cargo llvm-cov` with merged LCOV per `platform.coverage-merge`.

**Tier:** 1

---

ss[verify.process.proptest]

Property tests SHOULD be added for channel/actor invariants described in lessons; **temporary waiver** until crates are wired.

**Tier:** 1 — process waiver

---

ss[verify.process.fuzz]

Fuzz targets SHOULD cover parsing and protocol edges for distributed builders; **temporary waiver** until `cargo-fuzz` targets exist.

**Tier:** 1 — process waiver

---

ss[verify.process.mutants]

`cargo-mutants` SHOULD run on critical modules listed in release policy; scope MAY remain limited initially.

**Tier:** 1

---

ss[verify.process.tracey-gate]

Pull requests SHOULD run Tracey `validate` and fail on uncovered/untested Tier-0 requirements once annotations are complete.

**Tier:** 1

---

## Requirement index

| ID | Summary | Tier |
|----|---------|------|
| `verify.process.nextest` | nextest in CI | 1 |
| `verify.process.llvm-cov` | llvm-cov merge | 1 |
| `verify.process.proptest` | proptest (deferred) | 1 |
| `verify.process.fuzz` | cargo-fuzz (deferred) | 1 |
| `verify.process.mutants` | mutation testing | 1 |
| `verify.process.tracey-gate` | Tracey on PR | 1 |
