//! Contract tests linking Tracey `verify.process.*` requirements to repository artifacts.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

// ss[verify verify.process.nextest]
#[test]
fn nextest_ci_profile_exists() {
    let path = repo_root().join(".config/nextest.toml");
    let text = std::fs::read_to_string(&path).expect("nextest config");
    assert!(
        text.contains("ci-unit"),
        ".config/nextest.toml must define ci-unit profile for Gate A"
    );
}

// ss[verify verify.process.proptest]
#[test]
fn proptest_support_configured() {
    let support = repo_root().join("core/src/proptest_support/mod.rs");
    assert!(support.is_file(), "proptest_support module must exist");
    let lib = std::fs::read_to_string(repo_root().join("core/src/lib.rs")).expect("lib.rs");
    assert!(
        lib.contains("macro_rules! ss_proptest"),
        "lib.rs must export ss_proptest! macro"
    );
}

// ss[verify verify.process.llvm-cov]
#[test]
fn llvm_cov_release_script_exists() {
    let script = repo_root().join("scripts/run-llvm-cov-release.sh");
    assert!(script.is_file(), "Gate B llvm-cov script must exist");
}

// ss[verify verify.process.tracey-gate]
#[test]
fn tracey_ci_scripts_exist() {
    let root = repo_root();
    assert!(root.join("scripts/tracey-ci-validate.sh").is_file());
    assert!(root.join("scripts/tracey-unmapped-gate.sh").is_file());
}

// ss[verify verify.process.file-size]
#[test]
fn file_size_gate_script_exists() {
    assert!(repo_root().join("scripts/check-file-size.sh").is_file());
}

// ss[verify platform.coverage-merge]
#[test]
fn coverage_merge_documented_in_release_script() {
    let text = std::fs::read_to_string(repo_root().join("scripts/run-llvm-cov-release.sh"))
        .expect("llvm-cov release script");
    assert!(
        text.contains("lcov") || text.contains("LCOV"),
        "release coverage script must document LCOV merge"
    );
}

// ss[verify verify.process.fuzz]
#[test]
fn fuzz_crate_and_script_exist() {
    let root = repo_root();
    assert!(
        root.join("scripts/run-fuzz.sh").is_file(),
        "scripts/run-fuzz.sh must exist"
    );
    assert!(
        root.join("core/fuzz/Cargo.toml").is_file(),
        "core/fuzz/Cargo.toml must exist"
    );
    for target in [
        "aeron_channel_uri",
        "fast_protocol_packed",
        "cgroup_quota",
        "pack_even_split",
    ] {
        let path = root
            .join("core/fuzz/fuzz_targets")
            .join(format!("{target}.rs"));
        assert!(path.is_file(), "missing fuzz target {}", path.display());
    }
}

// ss[related verify.process.mutants]
// ss[related platform.aeron-out-of-scope-coverage]
// Mutants remain a Tier-1 process waiver until a current kill-rate exists.
