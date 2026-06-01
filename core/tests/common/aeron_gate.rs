#![allow(dead_code)]

use std::time::Duration;

use steady_state::{media_driver_probe_with_reason, MediaDriverProbeError};

const INSTALL_DOC: &str = "core/routing_service/aeron/README_linux.md";

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn aeron_required() -> bool {
    env_flag("SS_AERON_REQUIRED")
}

/// Release sign-off profile (`SS_AERON_RELEASE=1`).
pub fn release_profile() -> bool {
    env_flag("SS_AERON_RELEASE")
}

/// Live driver suites are Gate C only — must not run under Gate A (`cargo nextest`, pre-publish).
// ss[verify distributed.media-driver-testing]
pub fn gate_c_live_tests_enabled() -> bool {
    env_flag("SS_AERON_GATE_C")
        || env_flag("SS_AERON_REQUIRED")
        || env_flag("SS_AERON_RUN_LIVE_AERON")
        || env_flag("SS_AERON_RELEASE")
}

fn print_gate_a_skip(test_name: &str) {
    eprintln!("======================================================================");
    eprintln!("SKIP [{test_name}]");
    eprintln!("  Reason: live Aeron tests are Gate C only (not Gate A / pre-publish nextest)");
    eprintln!("  Run: ./scripts/run-aeron-integration.sh");
    eprintln!("  Or:  SS_AERON_GATE_C=1 cargo test -p steady_state --test aeron_integration_suite");
    eprintln!("======================================================================");
}

/// Returns true when this test should not run (Gate A path — instant skip, no driver I/O).
pub fn skip_unless_gate_c(test_name: &str) -> bool {
    if gate_c_live_tests_enabled() {
        return false;
    }
    print_gate_a_skip(test_name);
    true
}

/// Returns true when the Aeron media driver responds to a CNC probe.
// ss[verify distributed.media-driver-testing]
pub fn media_driver_available() -> bool {
    media_driver_probe_with_reason(Duration::from_secs(5)).is_ok()
}

/// Probe with a custom timeout budget.
pub fn media_driver_available_within(max_wait: Duration) -> bool {
    media_driver_probe_with_reason(max_wait).is_ok()
}

fn print_skip_block(test_name: &str, err: &MediaDriverProbeError) {
    eprintln!("======================================================================");
    eprintln!("SKIP [{test_name}]");
    eprintln!("  Reason: {err}");
    eprintln!("  Hint: {}", err.hint());
    eprintln!("  Install: {INSTALL_DOC}");
    // ss[related platform.aeron-out-of-scope-coverage]
    eprintln!("  Note: Test passes without running (soft skip). Use SS_AERON_REQUIRED=1 to fail instead.");
    eprintln!("======================================================================");
}

/// Call at the start of integration tests. Returns false when the driver is unavailable (soft skip).
pub fn require_aeron_or_skip(test_name: &str) -> bool {
    match media_driver_probe_with_reason(Duration::from_secs(5)) {
        Ok(()) => true,
        Err(e) => {
            if aeron_required() {
                panic!(
                    "Aeron media driver required (SS_AERON_REQUIRED=1) but unavailable for [{test_name}]: {e}\n  Hint: {}",
                    e.hint()
                );
            }
            print_skip_block(test_name, &e);
            false
        }
    }
}

/// Like `require_aeron_or_skip` but returns `Err` for suite wrappers (fails when SS_AERON_REQUIRED=1).
pub fn require_aeron_or_skip_result(test_name: &str) -> Result<(), MediaDriverProbeError> {
    match media_driver_probe_with_reason(Duration::from_secs(5)) {
        Ok(()) => Ok(()),
        Err(e) => {
            if aeron_required() {
                return Err(e);
            }
            print_skip_block(test_name, &e);
            Err(e)
        }
    }
}

/// True when the whole serial suite should skip (driver down and not required).
// ss[verify distributed.media-driver-testing]
pub fn should_skip_entire_suite(suite_name: &str) -> bool {
    match media_driver_probe_with_reason(Duration::from_secs(5)) {
        Ok(()) => false,
        Err(e) => {
            if aeron_required() {
                panic!(
                    "Aeron media driver required (SS_AERON_REQUIRED=1) for [{suite_name}]: {e}\n  Hint: {}",
                    e.hint()
                );
            }
            print_skip_block(suite_name, &e);
            true
        }
    }
}
