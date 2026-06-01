//! Reusable Aeron pub/sub wiring for integration tests and examples.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use steady_state::distributed::aeron_channel_builder::AeronConfig;
use steady_state::distributed::aeron_channel_structs::{Channel, ControlMode, Endpoint, MediaType, ReliableConfig};
use steady_state::distributed::aqueduct_builder::AqueductBuilder;
use steady_state::distributed::aqueduct_stream::{
    LazySteadyStreamRxBundle, LazySteadyStreamTxBundle, LazyStreamRx, LazyStreamTx, StreamEgress,
    StreamIngress,
};
use steady_state::{AqueTech, Graph, GraphBuilder, SoloAct};

use super::aeron_test_error::{AeronPhase, AeronResult, AeronTestError};

static NEXT_TEST_SALT: AtomicU16 = AtomicU16::new(0);
/// Scenarios remaining that get extra registration settle after suite preflight (IPC singles group).
static POST_PREFLIGHT_SETTLE_REMAINING: AtomicU16 = AtomicU16::new(0);

/// Called when `preflight_ipc_roundtrip` succeeds (or script smoke already proved the driver).
pub fn mark_suite_preflight_wire_verified() {
    POST_PREFLIGHT_SETTLE_REMAINING.store(6, Ordering::SeqCst);
}

/// Set by `run-aeron-integration.sh` after `aeron_preflight_smoke` succeeds (avoids duplicate in-suite preflight).
pub fn script_preflight_already_ok() -> bool {
    std::env::var("SS_AERON_SCRIPT_PREFLIGHT_OK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn release_profile() -> bool {
    std::env::var("SS_AERON_RELEASE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Brief cooldown after script smoke (script already slept via `SS_AERON_PRE_SUITE_SETTLE_SEC`).
pub fn pre_suite_cooldown_after_script_preflight() {
    eprintln!("NOTE: brief cooldown after in-process suite warmup (script pre-suite settle already ran)");
    scenario_cooldown();
}

fn maybe_settle_after_suite_preflight(scenario: &str) {
    loop {
        let n = POST_PREFLIGHT_SETTLE_REMAINING.load(Ordering::SeqCst);
        if n == 0 {
            return;
        }
        if POST_PREFLIGHT_SETTLE_REMAINING
            .compare_exchange(n, n - 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            settle_after_suite_preflight(scenario);
            return;
        }
    }
}

fn llvm_cov_slow_path() -> bool {
    std::env::var("CARGO_LLVM_COV")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn start_registration_settle() -> Duration {
    if llvm_cov_slow_path() {
        Duration::from_secs(8)
    } else if release_profile() || script_preflight_already_ok() {
        Duration::from_secs(8)
    } else {
        Duration::from_secs(6)
    }
}

fn scenario_wire_ready_timeout() -> Duration {
    let base = if llvm_cov_slow_path() {
        35
    } else if release_profile() {
        35
    } else {
        25
    };
    Duration::from_secs(base)
}

const DEFAULT_BYTES_PER_ITEM: usize = 64;
const DEFAULT_CAPACITY: usize = 4096;
/// Fixed stream id for suite preflight / wire probes only (avoids collision with `fresh_stream_id` salts).
pub const PREFLIGHT_PROBE_STREAM_ID: i32 = 80_000;
const START_TIMEOUT: Duration = Duration::from_secs(30);
const RECV_TIMEOUT: Duration = Duration::from_secs(30);
const BUNDLE_RECV_TIMEOUT: Duration = Duration::from_secs(60);
const BUNDLE_POST_START_WAIT: Duration = Duration::from_millis(1500);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(60);

pub fn channel_uri(channel: &Channel) -> String {
    channel.cstring().into_string().unwrap_or_else(|_| "<invalid uri>".to_string())
}

pub fn stream_id_from_salt(salt: u16) -> i32 {
    10_000 + i32::from(salt)
}

pub fn fresh_stream_id() -> i32 {
    stream_id_from_salt(NEXT_TEST_SALT.fetch_add(1, Ordering::Relaxed))
}

pub fn port_from_salt(salt: u16) -> u16 {
    40_456 + salt
}

pub fn fresh_udp_port() -> u16 {
    port_from_salt(NEXT_TEST_SALT.fetch_add(1, Ordering::Relaxed) + 500)
}

pub fn fresh_multicast_ports() -> (u16, u16) {
    let base = NEXT_TEST_SALT.fetch_add(1, Ordering::Relaxed);
    (port_from_salt(base + 600), port_from_salt(base + 700))
}

pub fn channel_ipc() -> Channel {
    AeronConfig::new()
        .with_media_type(MediaType::Ipc)
        .use_ipc()
        .build()
}

pub fn channel_udp_p2p(port: u16) -> Channel {
    AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .use_point_to_point(Endpoint {
            ip: std::net::IpAddr::from([127, 0, 0, 1]),
            port,
        })
        .with_reliability(ReliableConfig::Reliable)
        .build()
}

pub fn channel_multicast(group_port: u16, control_port: u16) -> Channel {
    AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .use_multicast(
            Endpoint {
                ip: "224.0.1.1".parse().expect("multicast group ip"),
                port: group_port,
            },
            Endpoint {
                ip: "224.0.1.1".parse().expect("multicast control ip"),
                port: control_port,
            },
        )
        .with_control_mode(ControlMode::Dynamic)
        .with_ttl(1)
        .build()
}

pub fn new_graph() -> Graph {
    GraphBuilder::for_testing().build(())
}

/// Ensure the shared Aeron client can be created before scenarios run.
pub fn preflight_aeron_client(scenario: &str) -> AeronResult<()> {
    let graph = new_graph();
    if graph.aeron_media_driver().is_none() {
        return Err(AeronTestError::new(
            AeronPhase::Probe,
            scenario,
            "could not create Aeron client (is aeronmd running? same user as tests? try: restart aeronmd)",
        ));
    }
    Ok(())
}

/// Throwaway IPC pub/sub roundtrip to prove the driver path is ready (used after restart and at suite start).
// ss[verify distributed.subscribe-publish]
pub fn suite_ipc_wire_probe(scenario: &str) -> AeronResult<()> {
    const PREFLIGHT_START_SETTLE: Duration = Duration::from_secs(6);
    let mut graph = new_graph();
    let pipe = SinglePipe::wire(&mut graph, channel_ipc(), PREFLIGHT_PROBE_STREAM_ID);
    start_and_wait_running(scenario, &mut graph)?;
    std::thread::sleep(PREFLIGHT_START_SETTLE);
    let result = wait_for_ipc_roundtrip_ready_with_timeout(scenario, &pipe, PREFLIGHT_WIRE_READY_TIMEOUT);
    if result.is_ok() {
        drain_egress_before_shutdown(&pipe);
    } else {
        pipe.egress_tx.testing_close();
        std::thread::sleep(Duration::from_millis(500));
        let _ = shutdown_graph_lenient(scenario, graph);
        return result;
    }
    shutdown_graph(scenario, graph)?;
    result
}

/// Wire probe only (no client preflight sleep); for bisect and script settle hooks.
// ss[verify distributed.subscribe-publish]
pub fn ipc_wire_probe_only(scenario: &str) -> AeronResult<()> {
    preflight_aeron_client(scenario)?;
    suite_ipc_wire_probe(scenario)
}

/// Ensure the driver is reachable and IPC wire probe succeeds before scenarios run.
// ss[related distributed.media-driver-testing]
pub fn preflight_ipc_roundtrip(scenario: &str) -> AeronResult<()> {
    preflight_aeron_client(scenario)?;
    const ATTEMPTS: u32 = 3;
    const BACKOFF: Duration = Duration::from_secs(3);
    let mut last_err = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(BACKOFF);
        }
        std::thread::sleep(Duration::from_secs(2));
        match suite_ipc_wire_probe(scenario) {
            Ok(()) => {
                mark_suite_preflight_wire_verified();
                return Ok(());
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        AeronTestError::new(
            AeronPhase::Wire,
            scenario,
            "preflight IPC wire probe failed after retries",
        )
    }))
}

/// In-process wire warmup when script smoke already ran in another process (Gate C).
// ss[verify distributed.media-driver-testing]
pub fn suite_in_process_warmup(scenario: &str) -> AeronResult<()> {
    preflight_aeron_client(scenario)?;
    const ATTEMPTS: u32 = 3;
    const BACKOFF: Duration = Duration::from_secs(3);
    let mut last_err = None;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            eprintln!("NOTE [{scenario}]: in-process suite warmup retry {attempt}/{ATTEMPTS}");
            std::thread::sleep(BACKOFF);
        }
        match suite_ipc_wire_probe(scenario) {
            Ok(()) => {
                mark_suite_preflight_wire_verified();
                return Ok(());
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        AeronTestError::new(
            AeronPhase::Wire,
            scenario,
            "in-process suite wire warmup failed after retries",
        )
    }))
}

const IPC_WIRE_PROBE: &[u8] = b"ss_probe0";
const PREFLIGHT_WIRE_READY_TIMEOUT: Duration = Duration::from_secs(25);

/// Send one probe frame and wait for ingress before the main payload burst.
// ss[verify distributed.subscribe-publish]
pub fn wait_for_ipc_roundtrip_ready(scenario: &str, pipe: &SinglePipe) -> AeronResult<()> {
    wait_for_ipc_roundtrip_ready_with_timeout(scenario, pipe, scenario_wire_ready_timeout())
}

/// Extra registration settle when suite preflight already proved the driver (new stream still needs time).
fn settle_after_suite_preflight(scenario: &str) {
    let wait = if llvm_cov_slow_path() {
        Duration::from_secs(12)
    } else if release_profile() {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(8)
    };
    eprintln!("NOTE [{scenario}]: extra post-preflight registration settle ({wait:?})");
    std::thread::sleep(wait);
}

pub fn wait_for_ipc_roundtrip_ready_with_timeout(
    scenario: &str,
    pipe: &SinglePipe,
    total_timeout: Duration,
) -> AeronResult<()> {
    const ATTEMPTS: usize = 6;
    let per_attempt = total_timeout / ATTEMPTS as u32;
    let backoff = if release_profile() {
        Duration::from_secs(4)
    } else {
        Duration::from_secs(3)
    };

    for attempt in 0..ATTEMPTS {
        pipe.egress_tx.testing_send_frame(IPC_WIRE_PROBE);
        if pipe.ingress_rx.testing_avail_wait(1, per_attempt) {
            let _ = pipe.ingress_rx.testing_take_all();
            return Ok(());
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(backoff);
        }
    }
    let avail = pipe.ingress_rx.testing_avail_units();
    Err(
        AeronTestError::new(
            AeronPhase::Wire,
            scenario,
            "IPC wire probe did not round-trip (publish/subscribe path not ready)",
        )
        .stream_id(pipe.stream_id)
        .channel_uri(&pipe.channel_uri)
        .recv_counts(1, avail)
        .egress_occupied(0),
    )
}

/// Lane `f` uses `base_stream_id + f` (matches subscribe/publish bundle actors).
pub fn bundle_lane_stream_id(base_stream_id: i32, lane: usize) -> i32 {
    base_stream_id + lane as i32
}

fn bundle_start_settle() -> Duration {
    if release_profile() {
        Duration::from_secs(3)
    } else {
        BUNDLE_POST_START_WAIT
    }
    .max(BUNDLE_POST_START_WAIT)
}

fn wait_for_bundle_lane_ready_with_timeout<const G: usize>(
    scenario: &str,
    pipe: &BundlePipe<G>,
    lane: usize,
    total_timeout: Duration,
) -> AeronResult<()> {
    if G > 1 && lane > 0 {
        eprintln!("NOTE [{scenario}]: warming bundle lane 0 before lane {lane}");
        send_payload_lane(&pipe.egress_tx, 0, &[IPC_WIRE_PROBE]);
        if pipe.ingress_rx[0].testing_avail_wait(1, total_timeout / 3) {
            let _ = pipe.ingress_rx[0].testing_take_all();
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    const ATTEMPTS: usize = 6;
    let per_attempt = total_timeout / ATTEMPTS as u32;
    let backoff = if release_profile() {
        Duration::from_secs(4)
    } else {
        Duration::from_secs(3)
    };

    for attempt in 0..ATTEMPTS {
        send_payload_lane(&pipe.egress_tx, lane, &[IPC_WIRE_PROBE]);
        if pipe.ingress_rx[lane].testing_avail_wait(1, per_attempt) {
            let _ = pipe.ingress_rx[lane].testing_take_all();
            return Ok(());
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(backoff);
        }
    }
    let avail = pipe.ingress_rx[lane].testing_avail_units();
    Err(
        AeronTestError::new(
            AeronPhase::Wire,
            scenario,
            format!("bundle lane {lane} wire probe did not round-trip"),
        )
        .stream_id(pipe.stream_id)
        .channel_uri(&pipe.channel_uri)
        .recv_counts(1, avail),
    )
}

pub fn wait_for_bundle_lane_ready<const G: usize>(
    scenario: &str,
    pipe: &BundlePipe<G>,
    lane: usize,
) -> AeronResult<()> {
    wait_for_bundle_lane_ready_with_timeout(scenario, pipe, lane, scenario_wire_ready_timeout())
}

pub fn scenario_cooldown() {
    std::thread::sleep(Duration::from_secs(3));
}

pub fn wire_aqueduct_subscribe(
    graph: &mut Graph,
    ingress_tx: LazyStreamTx<StreamIngress>,
    tech: AqueTech,
    name: &'static str,
) {
    ingress_tx.build_aqueduct(
        tech,
        &graph.actor_builder().with_name(name).never_simulate(true),
        SoloAct,
    );
}

pub fn wire_aqueduct_publish(
    graph: &mut Graph,
    egress_rx: LazyStreamRx<StreamEgress>,
    tech: AqueTech,
    name: &'static str,
) {
    egress_rx.build_aqueduct(
        tech,
        &graph.actor_builder().with_name(name).never_simulate(true),
        SoloAct,
    );
}

pub struct SinglePipe {
    pub egress_tx: LazyStreamTx<StreamEgress>,
    pub ingress_rx: LazyStreamRx<StreamIngress>,
    pub stream_id: i32,
    pub channel_uri: String,
}

impl SinglePipe {
    pub fn wire(graph: &mut Graph, channel: Channel, stream_id: i32) -> Self {
        let uri = channel_uri(&channel);
        let cb = graph.channel_builder().with_capacity(DEFAULT_CAPACITY);
        let (egress_tx, egress_rx) = cb.build_stream::<StreamEgress>(DEFAULT_BYTES_PER_ITEM);
        let (ingress_tx, ingress_rx) = cb.build_stream::<StreamIngress>(DEFAULT_BYTES_PER_ITEM);
        let tech = AqueTech::Aeron(channel, stream_id);
        wire_aqueduct_subscribe(graph, ingress_tx, tech.clone(), "AeronSubscribe");
        wire_aqueduct_publish(graph, egress_rx, tech, "AeronPublish");
        Self {
            egress_tx,
            ingress_rx,
            stream_id,
            channel_uri: uri,
        }
    }
}

pub struct BundlePipe<const GIRTH: usize> {
    pub egress_tx: LazySteadyStreamTxBundle<StreamEgress, GIRTH>,
    pub ingress_rx: LazySteadyStreamRxBundle<StreamIngress, GIRTH>,
    pub stream_id: i32,
    pub channel_uri: String,
}

impl<const GIRTH: usize> BundlePipe<GIRTH> {
    pub fn wire(graph: &mut Graph, channel: Channel, stream_id: i32) -> Self {
        let uri = channel_uri(&channel);
        let cb = graph.channel_builder().with_capacity(DEFAULT_CAPACITY);
        let (egress_tx, egress_rx) =
            cb.build_stream_bundle::<StreamEgress, GIRTH>(DEFAULT_BYTES_PER_ITEM);
        let (ingress_tx, ingress_rx) =
            cb.build_stream_bundle::<StreamIngress, GIRTH>(DEFAULT_BYTES_PER_ITEM);
        let tech = AqueTech::Aeron(channel, stream_id);
        ingress_tx.build_aqueduct(
            tech.clone(),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeBundle")
                .never_simulate(true),
            SoloAct,
        );
        egress_rx.build_aqueduct(
            tech,
            &graph
                .actor_builder()
                .with_name("AeronPublishBundle")
                .never_simulate(true),
            SoloAct,
        );
        Self {
            egress_tx,
            ingress_rx,
            stream_id,
            channel_uri: uri,
        }
    }
}

pub fn start_and_wait_running(scenario: &str, graph: &mut Graph) -> AeronResult<()> {
    if graph.start_with_timeout(START_TIMEOUT) {
        // Allow subscribe/publish actors to finish exclusive registration with the driver.
        std::thread::sleep(start_registration_settle());
        Ok(())
    } else {
        Err(
            AeronTestError::new(
                AeronPhase::GraphStart,
                scenario,
                "graph did not reach Running within timeout (actors may not have registered)",
            ),
        )
    }
}

pub fn send_payloads_single(egress_tx: &LazyStreamTx<StreamEgress>, payloads: &[&[u8]]) {
    for p in payloads {
        egress_tx.testing_send_frame(p);
    }
    egress_tx.testing_close();
}

pub fn send_payload_lane<const G: usize>(
    egress_tx: &LazySteadyStreamTxBundle<StreamEgress, G>,
    lane: usize,
    payloads: &[&[u8]],
) {
    for p in payloads {
        egress_tx[lane].testing_send_frame(p);
    }
}

pub fn close_all_lanes<const G: usize>(egress_tx: &LazySteadyStreamTxBundle<StreamEgress, G>) {
    for lane in egress_tx.iter() {
        lane.testing_close();
    }
}

pub fn recv_all_single(
    scenario: &str,
    pipe: &SinglePipe,
    expected_count: usize,
    timeout: Duration,
) -> AeronResult<Vec<(StreamIngress, Box<[u8]>)>> {
    let ingress_rx = &pipe.ingress_rx;
    for attempt in 0..3 {
        if ingress_rx.testing_avail_wait(expected_count, timeout) {
            let taken = ingress_rx.testing_take_all();
            if taken.len() == expected_count {
                return Ok(taken);
            }
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(250 * (attempt as u64 + 1)));
        }
    }
    let avail = ingress_rx.testing_avail_units();
    Err(
        AeronTestError::new(
            AeronPhase::Recv,
            scenario,
            "timed out waiting for ingress messages (publish/subscribe path may be stalled)",
        )
        .stream_id(pipe.stream_id)
        .channel_uri(&pipe.channel_uri)
        .recv_counts(expected_count, avail)
        .egress_occupied(0),
    )
}

pub fn recv_lane<const G: usize>(
    scenario: &str,
    pipe: &BundlePipe<G>,
    lane: usize,
    expected_count: usize,
    timeout: Duration,
) -> AeronResult<Vec<(StreamIngress, Box<[u8]>)>> {
    for attempt in 0..3 {
        if pipe.ingress_rx[lane].testing_avail_wait(expected_count, timeout) {
            let taken = pipe.ingress_rx[lane].testing_take_all();
            if taken.len() == expected_count {
                return Ok(taken);
            }
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(250 * (attempt as u64 + 1)));
        }
    }
    let avail = pipe.ingress_rx[lane].testing_avail_units();
    Err(
        AeronTestError::new(
            AeronPhase::Recv,
            scenario,
            format!("timed out on bundle lane {lane}"),
        )
        .stream_id(pipe.stream_id)
        .channel_uri(&pipe.channel_uri)
        .recv_counts(expected_count, avail),
    )
}

pub fn assert_payloads_match(
    scenario: &str,
    received: &[(StreamIngress, Box<[u8]>)],
    expected: &[&[u8]],
) -> AeronResult<()> {
    if received.len() != expected.len() {
        return Err(AeronTestError::new(
            AeronPhase::Assert,
            scenario,
            format!(
                "message count mismatch: expected {} got {}",
                expected.len(),
                received.len()
            ),
        ));
    }
    for (i, (meta, bytes)) in received.iter().enumerate() {
        if meta.length != expected[i].len() as i32 {
            return Err(AeronTestError::new(
                AeronPhase::Assert,
                scenario,
                format!("control length mismatch at index {i}"),
            ));
        }
        if bytes.as_ref() != expected[i] {
            return Err(AeronTestError::new(
                AeronPhase::Assert,
                scenario,
                format!("payload mismatch at index {i}"),
            ));
        }
    }
    Ok(())
}

/// Close egress and brief settle so Aeron publish actors can vote stop before strict shutdown.
pub fn drain_egress_before_shutdown(pipe: &SinglePipe) {
    pipe.egress_tx.testing_close();
    std::thread::sleep(Duration::from_millis(800));
}

pub fn drain_bundle_egress_before_shutdown<const G: usize>(pipe: &BundlePipe<G>) {
    close_all_lanes(&pipe.egress_tx);
    std::thread::sleep(Duration::from_millis(800));
}

pub fn shutdown_graph(scenario: &str, graph: Graph) -> AeronResult<()> {
    shutdown_graph_inner(scenario, graph, false)
}

/// Shutdown when mismatched Aeron streams may leave actors blocked on registration.
pub fn shutdown_graph_lenient(scenario: &str, graph: Graph) -> AeronResult<()> {
    shutdown_graph_inner(scenario, graph, true)
}

fn shutdown_graph_inner(scenario: &str, graph: Graph, lenient: bool) -> AeronResult<()> {
    let mut graph = graph;
    graph.request_shutdown();
    if let Err(e) = graph.block_until_stopped(SHUTDOWN_TIMEOUT) {
        if lenient {
            eprintln!(
                "WARN [{scenario}]: graph stop timed out or unclean ({e}); continuing (RUST_LOG=steady_state::graph_liveliness=debug for voters)"
            );
        } else {
            return Err(AeronTestError::new(
                AeronPhase::Shutdown,
                scenario,
                format!("graph did not stop cleanly within timeout: {e}"),
            ));
        }
    }
    scenario_cooldown();
    if lenient {
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

// ss[verify distributed.subscribe-publish]
pub fn run_single_roundtrip(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
) -> AeronResult<()> {
    run_single_roundtrip_with_timeout(scenario, channel, stream_id, payloads, RECV_TIMEOUT, Duration::ZERO)
}

pub fn run_single_roundtrip_with_timeout(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
    recv_timeout: Duration,
    post_start_wait: Duration,
) -> AeronResult<()> {
    let mut graph = new_graph();
    let pipe = SinglePipe::wire(&mut graph, channel, stream_id);
    start_and_wait_running(scenario, &mut graph)?;
    if !post_start_wait.is_zero() {
        std::thread::sleep(post_start_wait);
    }
    maybe_settle_after_suite_preflight(scenario);
    wait_for_ipc_roundtrip_ready(scenario, &pipe)?;
    send_payloads_single(&pipe.egress_tx, payloads);
    let received = recv_all_single(scenario, &pipe, payloads.len(), recv_timeout)?;
    assert_payloads_match(scenario, &received, payloads)?;
    shutdown_graph(scenario, graph)
}

pub const UDP_POST_START_WAIT: Duration = Duration::from_secs(3);
pub const UDP_RECV_TIMEOUT: Duration = Duration::from_secs(90);

pub fn run_single_roundtrip_udp(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
) -> AeronResult<()> {
    run_single_roundtrip_with_timeout(
        scenario,
        channel,
        stream_id,
        payloads,
        UDP_RECV_TIMEOUT,
        UDP_POST_START_WAIT,
    )
}

/// Send and receive in batches so large runs do not overrun the Aeron publication window.
// ss[verify distributed.subscribe-publish]
pub fn run_single_roundtrip_batched(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
    batch_size: usize,
) -> AeronResult<()> {
    run_single_roundtrip_batched_with_wait(scenario, channel, stream_id, payloads, batch_size, Duration::ZERO)
}

pub fn run_single_roundtrip_batched_with_wait(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
    batch_size: usize,
    post_start_wait: Duration,
) -> AeronResult<()> {
    run_single_roundtrip_batched_with_timeouts(
        scenario,
        channel,
        stream_id,
        payloads,
        batch_size,
        post_start_wait,
        RECV_TIMEOUT,
    )
}

pub fn run_single_roundtrip_batched_with_timeouts(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    payloads: &[&[u8]],
    batch_size: usize,
    post_start_wait: Duration,
    recv_timeout: Duration,
) -> AeronResult<()> {
    let mut graph = new_graph();
    let pipe = SinglePipe::wire(&mut graph, channel, stream_id);
    start_and_wait_running(scenario, &mut graph)?;
    if !post_start_wait.is_zero() {
        std::thread::sleep(post_start_wait);
    }
    maybe_settle_after_suite_preflight(scenario);
    wait_for_ipc_roundtrip_ready(scenario, &pipe)?;
    let mut all_received: Vec<(StreamIngress, Box<[u8]>)> = Vec::with_capacity(payloads.len());
    for batch in payloads.chunks(batch_size.max(1)) {
        for p in batch {
            pipe.egress_tx.testing_send_frame(p);
        }
        let received = recv_all_single(scenario, &pipe, batch.len(), recv_timeout)?;
        all_received.extend(received);
        std::thread::sleep(Duration::from_millis(300));
    }
    drain_egress_before_shutdown(&pipe);
    assert_payloads_match(scenario, &all_received, payloads)?;
    shutdown_graph(scenario, graph)
}

// ss[verify distributed.subscribe-publish]
// ss[verify distributed.aqueduct-stream]
pub fn run_bundle_lane_roundtrip<const G: usize>(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
    lane: usize,
    payloads: &[&[u8]],
) -> AeronResult<()> {
    let mut graph = new_graph();
    let pipe = BundlePipe::<G>::wire(&mut graph, channel, stream_id);
    start_and_wait_running(scenario, &mut graph)?;
    std::thread::sleep(bundle_start_settle());
    maybe_settle_after_suite_preflight(scenario);
    wait_for_bundle_lane_ready(scenario, &pipe, lane)?;
    send_payload_lane(&pipe.egress_tx, lane, payloads);
    close_all_lanes(&pipe.egress_tx);
    let received = recv_lane(scenario, &pipe, lane, payloads.len(), BUNDLE_RECV_TIMEOUT)?;
    assert_payloads_match(scenario, &received, payloads)?;
    drain_bundle_egress_before_shutdown(&pipe);
    shutdown_graph(scenario, graph)
}

pub fn run_stream_id_mismatch(scenario: &str, channel: Channel) -> AeronResult<()> {
    let subscribe_id = fresh_stream_id();
    let publish_id = fresh_stream_id();
    let uri = channel_uri(&channel);
    let mut graph = new_graph();
    let cb = graph.channel_builder().with_capacity(DEFAULT_CAPACITY);
    let (egress_tx, egress_rx) = cb.build_stream::<StreamEgress>(DEFAULT_BYTES_PER_ITEM);
    let (ingress_tx, ingress_rx) = cb.build_stream::<StreamIngress>(DEFAULT_BYTES_PER_ITEM);
    wire_aqueduct_subscribe(
        &mut graph,
        ingress_tx,
        AqueTech::Aeron(channel.clone(), subscribe_id),
        "AeronSubscribe",
    );
    wire_aqueduct_publish(
        &mut graph,
        egress_rx,
        AqueTech::Aeron(channel, publish_id),
        "AeronPublish",
    );
    start_and_wait_running(scenario, &mut graph)?;
    send_payloads_single(&egress_tx, &[b"should-not-arrive"]);
    let saw_data = ingress_rx.testing_avail_wait(1, Duration::from_secs(2));
    if saw_data {
        return Err(
            AeronTestError::new(
                AeronPhase::Recv,
                scenario,
                "ingress received data despite mismatched stream_id (isolation failure)",
            )
            .stream_id(subscribe_id)
            .channel_uri(&uri),
        );
    }
    std::thread::sleep(Duration::from_millis(500));
    shutdown_graph_lenient(scenario, graph)
}

/// Backpressure-style send burst using SinglePipe + default ring capacity.
// ss[verify distributed.subscribe-publish]
pub fn run_backpressure_scenario(scenario: &str) -> AeronResult<()> {
    const COUNT: usize = 16;
    const PAYLOAD: [u8; 8] = *b"bp000000";

    let stream_id = fresh_stream_id();
    let channel = channel_ipc();
    let mut graph = new_graph();
    let pipe = SinglePipe::wire(&mut graph, channel, stream_id);
    start_and_wait_running(scenario, &mut graph)?;
    wait_for_ipc_roundtrip_ready(scenario, &pipe)?;

    for i in 0..COUNT {
        pipe.egress_tx.testing_send_frame(&PAYLOAD);
        if i + 1 < COUNT {
            let _ = pipe.ingress_rx.testing_avail_wait(1, Duration::from_millis(250));
        }
    }
    pipe.egress_tx.testing_close();

    let expected: Vec<&[u8]> = (0..COUNT).map(|_| PAYLOAD.as_slice()).collect();
    let received = recv_all_single(scenario, &pipe, COUNT, RECV_TIMEOUT)?;
    assert_payloads_match(scenario, &received, &expected)?;
    drain_egress_before_shutdown(&pipe);
    shutdown_graph(scenario, graph)
}

// ss[verify distributed.subscribe-publish]
// ss[verify distributed.aqueduct-stream]
pub fn run_bundle_both_lanes<const G: usize>(
    scenario: &str,
    channel: Channel,
    stream_id: i32,
) -> AeronResult<()> {
    let mut graph = new_graph();
    let pipe = BundlePipe::<G>::wire(&mut graph, channel, stream_id);
    start_and_wait_running(scenario, &mut graph)?;
    std::thread::sleep(BUNDLE_POST_START_WAIT);
    wait_for_bundle_lane_ready(scenario, &pipe, 0)?;
    send_payload_lane(&pipe.egress_tx, 0, &[b"lane0"]);
    std::thread::sleep(Duration::from_millis(500));
    send_payload_lane(&pipe.egress_tx, 1, &[b"lane1-msg"]);
    close_all_lanes(&pipe.egress_tx);
    let r0 = recv_lane(scenario, &pipe, 0, 1, BUNDLE_RECV_TIMEOUT)?;
    let r1 = recv_lane(scenario, &pipe, 1, 1, BUNDLE_RECV_TIMEOUT)?;
    assert_payloads_match(scenario, &r0, &[b"lane0"])?;
    assert_payloads_match(scenario, &r1, &[b"lane1-msg"])?;
    drain_bundle_egress_before_shutdown(&pipe);
    shutdown_graph(scenario, graph)
}

// ss[verify distributed.aqueduct-stream]
// ss[verify distributed.subscribe-publish]
pub fn run_aqueduct_all_impls(scenario: &str) -> AeronResult<()> {
    let mut graph = new_graph();
    let cb = graph.channel_builder().with_capacity(512);
    let channel = channel_ipc();
    let stream_id = fresh_stream_id();
    let tech = AqueTech::Aeron(channel, stream_id);

    let (_tx1, rx1) = cb.build_stream::<StreamEgress>(8);
    let (tx2, _rx2) = cb.build_stream::<StreamIngress>(8);
    let (_tx3, rx3) = cb.build_stream_bundle::<StreamEgress, 1>(8);
    let (tx4, _rx4) = cb.build_stream_bundle::<StreamIngress, 1>(8);

    wire_aqueduct_publish(&mut graph, rx1, tech.clone(), "PubSingle");
    wire_aqueduct_subscribe(&mut graph, tx2, tech.clone(), "SubSingle");
    rx3.build_aqueduct(
        tech.clone(),
        &graph.actor_builder().with_name("PubBundle").never_simulate(true),
        SoloAct,
    );
    tx4.build_aqueduct(
        tech,
        &graph.actor_builder().with_name("SubBundle").never_simulate(true),
        SoloAct,
    );

    start_and_wait_running(scenario, &mut graph)?;
    shutdown_graph_lenient(scenario, graph)
}

/// Live driver roundtrip using `AeronConfig`-built channels (IPC always; UDP when enabled in suite).
// ss[verify distributed.aeron-uri]
// ss[verify distributed.subscribe-publish]
pub fn run_uri_live_transport_roundtrip(scenario: &str, include_udp: bool) -> AeronResult<()> {
    use steady_state::distributed::aeron_channel_builder::AeronConfig;
    use steady_state::distributed::aeron_channel_structs::{Endpoint, MediaType, ReliableConfig};

    let ipc = AeronConfig::new()
        .with_media_type(MediaType::Ipc)
        .use_ipc()
        .build();
    run_single_roundtrip(scenario, ipc, fresh_stream_id(), &[b"uri-live-ipc"])?;
    if include_udp {
        let port = fresh_udp_port();
        let udp = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: std::net::IpAddr::from([127, 0, 0, 1]),
                port,
            })
            .with_reliability(ReliableConfig::Reliable)
            .build();
        run_single_roundtrip_udp(
            &format!("{scenario}_udp"),
            udp,
            fresh_stream_id(),
            &[b"uri-live-udp"],
        )?;
    }
    Ok(())
}

// ss[verify distributed.aqueduct-stream]
pub fn run_aqueduct_graph_start_only(scenario: &str, bundle: bool) -> AeronResult<()> {
    let mut graph = new_graph();
    let channel = channel_ipc();
    let stream_id = fresh_stream_id();
    if bundle {
        let _pipe = BundlePipe::<2>::wire(&mut graph, channel, stream_id);
    } else {
        let _pipe = SinglePipe::wire(&mut graph, channel, stream_id);
    }
    start_and_wait_running(scenario, &mut graph)?;
    shutdown_graph_lenient(scenario, graph)
}

#[cfg(test)]
mod harness_contract_tests {
    use super::*;

    #[test]
    fn fresh_stream_ids_avoid_preflight_probe_stream() {
        for salt in 0..u16::MAX {
            let id = stream_id_from_salt(salt);
            assert_ne!(
                id, PREFLIGHT_PROBE_STREAM_ID,
                "salt {salt} collides with preflight probe stream id"
            );
        }
    }

    #[test]
    fn bundle_lane_stream_id_matches_actor_convention() {
        let base = 10_000;
        assert_eq!(bundle_lane_stream_id(base, 0), 10_000);
        assert_eq!(bundle_lane_stream_id(base, 1), 10_001);
        assert_eq!(bundle_lane_stream_id(base, 2), 10_002);
    }
}
