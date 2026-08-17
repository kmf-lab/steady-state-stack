//! Minimal live-driver wire probe for script post-restart settle (Gate C).
//!
//! Run: `cargo test -p steady_state --test aeron_preflight_smoke -- --nocapture`

mod common;

use common::aeron_gate::{should_skip_entire_suite, skip_unless_gate_c};
use common::support::pub_sub_harness::preflight_ipc_roundtrip;

#[test]
// ss[verify distributed.media-driver-testing]
fn aeron_preflight_wire_settle() -> Result<(), Box<dyn std::error::Error>> {
    const NAME: &str = "aeron_preflight_wire_settle";
    if skip_unless_gate_c(NAME) {
        return Ok(());
    }
    if should_skip_entire_suite(NAME) {
        return Ok(());
    }
    preflight_ipc_roundtrip(NAME).map_err(|e| format!("{e}").into())
}
