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

// ss[related verify.process.fuzz]
// ss[related verify.process.mutants]
// ss[related platform.aeron-out-of-scope-coverage]
// Waivers: docs/spec/00-conventions.md and docs/spec/12-verification-stack.md
