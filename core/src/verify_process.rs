//! Tracey impl anchors for CI / release process requirements (`verify.process.*`).
//!
//! Shell scripts under `scripts/` are not in Tracey's rust-core glob; these constants and
//! the contract tests in `core/tests/tracey_process_contract.rs` link requirements to the repo.

#![allow(dead_code)]

// ss[impl verify.process.nextest]
/// Nextest profile used by `.github/workflows/rust.yml` Gate A.
pub(crate) const NEXTEST_CI_PROFILE: &str = "ci-unit";

// ss[impl verify.process.proptest]
/// Default proptest case count for `ss_proptest!` (see `proptest_support::ss_proptest_config`).
pub(crate) const PROPTEST_DEFAULT_CASES: u32 = 2048;

// ss[impl verify.process.llvm-cov]
/// Release coverage merge script (Gate B).
pub(crate) const LLVM_COV_RELEASE_SCRIPT: &str = "scripts/run-llvm-cov-release.sh";

// ss[impl verify.process.tracey-gate]
/// Tracey validate script run in CI.
pub(crate) const TRACEY_VALIDATE_SCRIPT: &str = "scripts/tracey-ci-validate.sh";

// ss[impl verify.process.file-size]
/// Per-file line cap gate for `core/src/`.
pub(crate) const FILE_SIZE_GATE_SCRIPT: &str = "scripts/check-file-size.sh";

// ss[impl platform.coverage-merge]
/// Merged LCOV output from Gate B (`run-llvm-cov-release.sh`).
pub(crate) const MERGED_LCOV_OUTPUT: &str = "merged.lcov";

// ss[impl verify.process.fuzz]
/// Bounded cargo-fuzz campaign script (parse/protocol targets under `core/fuzz`).
pub(crate) const FUZZ_CAMPAIGN_SCRIPT: &str = "scripts/run-fuzz.sh";

// ss[impl verify.process.mutants]
/// Resumable file-scoped cargo-mutants campaign (Stage-1 globs in `core/mutants.toml`).
pub(crate) const MUTANTS_CAMPAIGN_SCRIPT: &str = "scripts/run-mutants-iterate.sh";

// ss[related platform.aeron-out-of-scope-coverage]
// Waiver: docs/spec/00-conventions.md (`platform.aeron-out-of-scope-coverage`)
