//! Single-process serial Aeron pub/sub integration suite (one media driver session).
//!
//! Run: `cargo test -p steady_state --features exec_async_std --test aeron_integration_suite -- --nocapture`
//!
//! Bisect one scenario: `SS_AERON_SCENARIO=ipc_single_ten cargo test ...`

mod common;

use common::aeron_driver::refresh_media_driver;
use common::aeron_gate::{release_profile, should_skip_entire_suite};
use common::support::pub_sub_harness::{
    channel_ipc, channel_multicast, channel_udp_p2p, fresh_multicast_ports, fresh_stream_id,
    fresh_udp_port, ipc_wire_probe_only, preflight_ipc_roundtrip, run_aqueduct_all_impls,
    run_aqueduct_graph_start_only, run_backpressure_scenario, run_bundle_both_lanes,
    run_bundle_lane_roundtrip,     run_single_roundtrip, run_single_roundtrip_batched,
    run_single_roundtrip_udp, run_stream_id_mismatch, run_uri_live_transport_roundtrip,
    scenario_cooldown, script_preflight_already_ok, suite_in_process_warmup,
};
use common::support::{AeronPhase, AeronResult};

// ss[verify distributed.subscribe-publish]
// ss[verify distributed.aqueduct-stream]
// ss[verify distributed.aeron-uri]

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn multicast_enabled() -> bool {
    env_flag("SS_AERON_MULTICAST")
}

fn udp_enabled() -> bool {
    env_flag("SS_AERON_UDP")
}

fn scenario_filter() -> Option<String> {
    std::env::var("SS_AERON_SCENARIO").ok().filter(|s| !s.is_empty())
}

fn should_run(name: &str) -> bool {
    match scenario_filter().as_deref() {
        Some(want) => want == name,
        None => true,
    }
}

fn refresh_minimal() -> bool {
    std::env::var("SS_AERON_REFRESH_MINIMAL")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn refresh_after_scenario_index(scenario_index: u32) -> bool {
    if release_profile() {
        // Script restarts at start; refresh after ten (2), bundle group (6), shutdown_bundle (9); skip post-hundred (3).
        matches!(scenario_index, 2 | 6 | 9 | 10)
    } else {
        matches!(scenario_index, 3 | 6 | 10)
    }
}

fn maybe_refresh_driver_after(scenario_index: u32) -> Result<(), Box<dyn std::error::Error>> {
    if refresh_minimal() {
        scenario_cooldown();
        return Ok(());
    }
    let refresh = refresh_after_scenario_index(scenario_index);
    if refresh {
        refresh_media_driver().map_err(|e| format!("driver refresh failed: {e}"))?;
        if release_profile() {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        scenario_cooldown();
    }
    Ok(())
}

fn scenario_retry_enabled() -> bool {
    std::env::var("SS_AERON_SCENARIO_RETRY")
        .map(|v| v != "0")
        .unwrap_or(true)
}

fn run_scenario(
    name: &str,
    scenario_index: &mut u32,
    mut f: impl FnMut() -> AeronResult<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !should_run(name) {
        eprintln!("SKIP [{name}] (SS_AERON_SCENARIO filter active)");
        return Ok(());
    }
    *scenario_index += 1;
    let mut result = f();
    if let Err(ref e) = result {
        if scenario_retry_enabled() && matches!(e.phase, AeronPhase::Wire | AeronPhase::Recv) {
            eprintln!("RETRY [{name}] after transient {} — refreshing media driver", e.phase);
            refresh_media_driver().map_err(|r| format!("driver refresh failed: {r}"))?;
            scenario_cooldown();
            std::thread::sleep(std::time::Duration::from_secs(5));
            result = f();
        }
    }
    match result {
        Ok(()) => {
            eprintln!("PASS [{name}]");
            scenario_cooldown();
            maybe_refresh_driver_after(*scenario_index)?;
            Ok(())
        }
        Err(e) => Err(format!("FAIL [{name}]: {e}").into()),
    }
}

fn run_all_scenarios() -> Result<(), Box<dyn std::error::Error>> {
    let mut idx = 0u32;

    if let Some(ref want) = scenario_filter() {
        eprintln!("SS_AERON_SCENARIO={want}: running matching scenarios only");
        if want == "ipc_wire_only" {
            return run_scenario("ipc_wire_only", &mut idx, || ipc_wire_probe_only("ipc_wire_only"));
        }
    }

    // Group A: IPC singles (heavy hundred after one/ten; refresh after group)
    // ss[verify distributed.subscribe-publish]
    // ss[verify distributed.aeron-uri]
    // ss[verify distributed.subscribe-publish]
    run_scenario("ipc_single_one", &mut idx, || {
        run_single_roundtrip("ipc_single_one", channel_ipc(), fresh_stream_id(), &[b"frame-1"])
    })?;

    // ss[verify distributed.subscribe-publish]
    run_scenario("ipc_single_ten", &mut idx, || {
        let fixed: [&[u8]; 10] = [
            b"ipc-00", b"ipc-01", b"ipc-02", b"ipc-03", b"ipc-04", b"ipc-05", b"ipc-06", b"ipc-07",
            b"ipc-08", b"ipc-09",
        ];
        let payloads: Vec<Vec<u8>> = fixed.iter().map(|p| p.to_vec()).collect();
        let refs: Vec<&[u8]> = payloads.iter().map(|v| v.as_slice()).collect();
        run_single_roundtrip_batched("ipc_single_ten", channel_ipc(), fresh_stream_id(), &refs, 2)
    })?;

    // ss[verify distributed.subscribe-publish]
    run_scenario("ipc_single_hundred", &mut idx, || {
        let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(100);
        for i in 0..100 {
            payloads.push(format!("ipc-hundred-{i:03}").into_bytes());
        }
        let refs: Vec<&[u8]> = payloads.iter().map(|v| v.as_slice()).collect();
        run_single_roundtrip_batched(
            "ipc_single_hundred",
            channel_ipc(),
            fresh_stream_id(),
            &refs,
            10,
        )
    })?;

    // Group B: bundle
    // ss[verify distributed.subscribe-publish]
    // ss[verify distributed.aqueduct-stream]
    // ss[verify distributed.subscribe-publish]
    // ss[verify distributed.aqueduct-stream]
    run_scenario("ipc_bundle_lane0", &mut idx, || {
        run_bundle_lane_roundtrip::<2>(
            "ipc_bundle_lane0",
            channel_ipc(),
            fresh_stream_id(),
            0,
            &[b"bundle-lane-0-a", b"bundle-lane-0-b"],
        )
    })?;

    // ss[verify distributed.subscribe-publish]
    // ss[verify distributed.aqueduct-stream]
    run_scenario("ipc_bundle_lane1", &mut idx, || {
        run_bundle_lane_roundtrip::<2>(
            "ipc_bundle_lane1",
            channel_ipc(),
            fresh_stream_id(),
            1,
            &[b"bundle-lane-1-x", b"bundle-lane-1-y", b"bundle-lane-1-z"],
        )
    })?;

    run_scenario("ipc_bundle_both_lanes", &mut idx, || {
        run_bundle_both_lanes::<2>("ipc_bundle_both_lanes", channel_ipc(), fresh_stream_id())
    })?;

    // Group C: isolation / shutdown / backpressure
    // ss[verify distributed.subscribe-publish]
    run_scenario("stream_id_mismatch", &mut idx, || {
        run_stream_id_mismatch("stream_id_mismatch", channel_ipc())
    })?;

    // ss[verify distributed.subscribe-publish]
    run_scenario("shutdown_single", &mut idx, || {
        run_single_roundtrip(
            "shutdown_single",
            channel_ipc(),
            fresh_stream_id(),
            &[b"shutdown-single"],
        )
    })?;

    // ss[verify distributed.subscribe-publish]
    run_scenario("shutdown_bundle", &mut idx, || {
        run_bundle_lane_roundtrip::<2>(
            "shutdown_bundle",
            channel_ipc(),
            fresh_stream_id(),
            0,
            &[b"shutdown-bundle"],
        )
    })?;

    // ss[verify distributed.subscribe-publish]
    eprintln!("COOLDOWN: pre-backpressure settle");
    std::thread::sleep(std::time::Duration::from_secs(5));
    run_scenario("backpressure_ipc", &mut idx, || run_backpressure_scenario("backpressure_ipc"))?;

    // Group D: aqueduct
    // ss[verify distributed.aqueduct-stream]
    // ss[verify distributed.aqueduct-stream]
    run_scenario("aqueduct_single_start", &mut idx, || {
        run_aqueduct_graph_start_only("aqueduct_single_start", false)
    })?;

    // ss[verify distributed.aqueduct-stream]
    run_scenario("aqueduct_bundle_start", &mut idx, || {
        run_aqueduct_graph_start_only("aqueduct_bundle_start", true)
    })?;

    // ss[verify distributed.aqueduct-stream]
    run_scenario("aqueduct_all_impls", &mut idx, || run_aqueduct_all_impls("aqueduct_all_impls"))?;

    // ss[verify distributed.aqueduct-stream]
    // ss[verify distributed.subscribe-publish]
    run_scenario("aqueduct_roundtrip", &mut idx, || {
        run_single_roundtrip(
            "aqueduct_roundtrip",
            channel_ipc(),
            fresh_stream_id(),
            &[b"aqueduct-roundtrip"],
        )
    })?;

    // Live URI + driver: AeronConfig-built IPC (and UDP when enabled)
    // ss[verify distributed.aeron-uri]
    // ss[verify distributed.subscribe-publish]
    run_scenario("uri_live_transports", &mut idx, || {
        run_uri_live_transport_roundtrip("uri_live_transports", udp_enabled())
    })?;

    // Group E: UDP (refresh before block when running full suite)
    if udp_enabled() {
        refresh_media_driver().map_err(|e| format!("pre-UDP driver refresh failed: {e}"))?;
        scenario_cooldown();
        eprintln!("--- UDP scenarios (SS_AERON_UDP=1) ---");
        std::thread::sleep(std::time::Duration::from_secs(3));

        // ss[verify distributed.subscribe-publish]
        // ss[verify distributed.aeron-uri]
        run_scenario("udp_p2p_roundtrip", &mut idx, || {
            run_single_roundtrip_udp(
                "udp_p2p_roundtrip",
                channel_udp_p2p(fresh_udp_port()),
                fresh_stream_id(),
                &[b"udp-p2p-one", b"udp-p2p-two", b"udp-p2p-three"],
            )
        })?;

        // ss[verify distributed.subscribe-publish]
        // ss[verify distributed.aeron-uri]
        run_scenario("udp_p2p_many_small", &mut idx, || {
            use common::support::pub_sub_harness::{
                run_single_roundtrip_batched_with_timeouts, UDP_POST_START_WAIT, UDP_RECV_TIMEOUT,
            };

            let mut payloads: Vec<Vec<u8>> = Vec::with_capacity(30);
            for i in 0..30 {
                payloads.push(vec![i as u8; 8]);
            }
            let refs: Vec<&[u8]> = payloads.iter().map(|v| v.as_slice()).collect();
            run_single_roundtrip_batched_with_timeouts(
                "udp_p2p_many_small",
                channel_udp_p2p(fresh_udp_port()),
                fresh_stream_id(),
                &refs,
                5,
                UDP_POST_START_WAIT,
                UDP_RECV_TIMEOUT,
            )
        })?;
    } else {
        eprintln!("SKIP [udp_p2p_roundtrip] (set SS_AERON_UDP=1 to enable UDP integration)");
        eprintln!("SKIP [udp_p2p_many_small] (set SS_AERON_UDP=1 to enable UDP integration)");
    }

    // Group F: multicast
    if multicast_enabled() {
        let (group, control) = fresh_multicast_ports();
        // ss[verify distributed.subscribe-publish]
        // ss[verify distributed.aeron-uri]
        run_scenario("multicast_roundtrip", &mut idx, || {
            run_single_roundtrip(
                "multicast_roundtrip",
                channel_multicast(group, control),
                fresh_stream_id(),
                &[b"mcast-a", b"mcast-b"],
            )
        })?;
    } else {
        eprintln!("SKIP [multicast_roundtrip] (set SS_AERON_MULTICAST=1 to enable)");
    }

    Ok(())
}

#[test]
// ss[verify distributed.media-driver-testing]
fn aeron_integration_serial_suite() -> Result<(), Box<dyn std::error::Error>> {
    const SUITE: &str = "aeron_integration_serial_suite";
    if common::aeron_gate::skip_unless_gate_c(SUITE) {
        return Ok(());
    }
    if should_skip_entire_suite(SUITE) {
        return Ok(());
    }
    if scenario_filter().is_some() {
        eprintln!("SS_AERON_SCENARIO set: skipping suite preflight (scenario runs its own wire probe)");
    } else if script_preflight_already_ok() {
        eprintln!("Script smoke OK (SS_AERON_SCRIPT_PREFLIGHT_OK=1); running in-process suite wire warmup …");
        suite_in_process_warmup(SUITE).map_err(|e| format!("{e}"))?;
        common::support::pub_sub_harness::pre_suite_cooldown_after_script_preflight();
    } else if let Err(e) = preflight_ipc_roundtrip(SUITE) {
        let required = std::env::var("SS_AERON_REQUIRED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if required {
            return Err(format!("{e}").into());
        }
        eprintln!("======================================================================");
        eprintln!("SKIP [{SUITE}]");
        eprintln!("  Reason: preflight failed ({e})");
        eprintln!("  Hint: docker restart aeronmd; same OS user as tests; see core/tests/README.md");
        eprintln!("  Note: soft skip (set SS_AERON_REQUIRED=1 to fail).");
        eprintln!("======================================================================");
        return Ok(());
    } else {
        scenario_cooldown();
        let secs = if std::env::var("SS_AERON_RELEASE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            12
        } else {
            8
        };
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
    run_all_scenarios()
}
