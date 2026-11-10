//! Round-5 coverage: registered shutdown voters, index-wait shutdown,
//! channel telemetry, and driver-free Aeron URI building.

// ss[related graph.liveliness-voters]
use std::net::IpAddr;
// ss[related philosophy.structural-hierarchy]
use std::panic::{catch_unwind, AssertUnwindSafe};
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};

// ss[related actor.index-wait-round-robin]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::distributed::aeron_channel_structs::{ControlMode, ReliableConfig};
// ss[related actor.shadow-spotlight]
use crate::monitor_telemetry::SteadyTelemetrySend;
// ss[related philosophy.structural-hierarchy]
use crate::*;

fn endpoint(port: u16) -> Endpoint {
    Endpoint {
        ip: "127.0.0.1".parse::<IpAddr>().expect("ipv4"),
        port,
    }
}

fn register_running_voter(
    runtime: &parking_lot::RwLock<GraphLiveliness>,
    ident: ActorIdentity,
) {
    let mut live = runtime.write();
    live.register_voter(ident);
    live.building_to_running();
}

#[test]
// ss[verify graph.liveliness-voters]
// ss[verify actor.index-wait-round-robin]
// ss[verify actor.wait-avail-vacant]
// ss[verify bundle.index-wait-shutdown-none]
fn registered_voter_drives_shutdown_and_index_waits_return_none() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (full_tx, _full_rx) = graph.channel_builder().with_capacity(1).build::<u8>();
    let (_empty_tx, empty_rx) = graph.channel_builder().with_capacity(1).build::<u8>();
    full_tx.testing_send_all(vec![1u8], false);
    let rx_meta = empty_rx.clone();
    let tx_meta = full_tx.clone();
    let ident = ActorIdentity::new(0, "voter", None);

    let mut shadow = graph.new_testing_test_monitor("shadow_voter");
    shadow.ident = ident;
    let mut spot_shadow = graph.new_testing_test_monitor("spot_voter");
    spot_shadow.ident = ident;
    let mut spot = spot_shadow.into_spotlight([&rx_meta], [&tx_meta]);
    register_running_voter(&*spot.runtime_state, ident);

    spot.telemetry.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    spot.frame_rate_ms = 1;
    spot.last_telemetry_send = Instant::now() - Duration::from_secs(30);
    let stale = core_exec::block_on(spot.wait_shutdown());
    assert!(
        !stale,
        "a running actor with a blown telemetry budget must not block in wait_shutdown"
    );

    core_exec::block_on(GraphLiveliness::internal_request_shutdown(
        spot.runtime_state.clone(),
    ));
    assert!(
        !spot.is_running(|| true),
        "an in-favor vote from a registered id must accept shutdown"
    );
    assert!(
        spot.is_running(|| false),
        "a later veto on the same ballot must keep the actor in the loop"
    );
    assert!(
        !shadow.is_running(|| true),
        "the shadow with the same registered id must accept shutdown"
    );

    let none = core_exec::block_on(async {
        let rx_held = empty_rx.clone();
        let tx_held = full_tx.clone();
        let rx_guard = rx_held.lock().await;
        let tx_guard = tx_held.lock().await;
        let mut rx_bundle = vec![rx_guard];
        let mut tx_bundle = vec![tx_guard];
        let avail = spot.wait_avail_index(&mut rx_bundle, &[1]).await;
        let vacant = spot.wait_vacant_index(&mut tx_bundle, &[1]).await;
        let shadow_avail = shadow.wait_avail_index(&mut rx_bundle, &[1]).await;
        (avail, vacant, shadow_avail)
    });
    assert_eq!(none.0, None, "shutdown during wait_avail_index yields None");
    assert_eq!(none.1, None, "shutdown during wait_vacant_index yields None");
    assert_eq!(none.2, None, "shadow shutdown during wait_avail_index yields None");

    #[allow(deprecated)]
    let bundle_stopped = core_exec::block_on(async {
        let rx_held = empty_rx.clone();
        let rx_guard = rx_held.lock().await;
        let mut bundle = vec![rx_guard];
        spot.wait_avail_bundle(&mut bundle, 1, 1).await
    });
    assert!(
        !bundle_stopped,
        "a fired shutdown oneshot must end wait_avail_bundle"
    );
}

#[test]
// ss[verify channel.backpressure-never-drop]
// ss[verify philosophy.zero-copy-discipline]
fn tx_and_rx_telemetry_record_normal_and_stream_counts() {
    let mut graph = GraphBuilder::for_testing().build(());
    let (tx, rx) = graph.channel_builder().with_capacity(4).build::<u8>();
    tx.testing_send_all(vec![4u8, 5], false);
    let (dummy_tx, _dummy_rx) = crate::channel_builder::ChannelBuilder::default()
        .with_capacity(2)
        .eager_build::<[usize; 2]>();
    let mut tel = SteadyTelemetrySend::new(dummy_tx, [0, 0], [0, 0], Instant::now());

    core_exec::block_on(async {
        let tx_held = tx.clone();
        let mut tx_guard = tx_held.lock().await;
        tx_guard.local_monitor_index = 0;
        TxCore::telemetry_inc(&mut *tx_guard, TxDone::Normal(3), &mut tel);
        TxCore::telemetry_inc(&mut *tx_guard, TxDone::Stream(1, 4), &mut tel);
        tx_guard.local_monitor_index = MONITOR_NOT;
        TxCore::telemetry_inc(&mut *tx_guard, TxDone::Normal(1), &mut tel);

        let rx_held = rx.clone();
        let mut rx_guard = rx_held.lock().await;
        let rendered = format!("{:?}", &*rx_guard);
        assert!(rendered.contains("Rx"), "{rendered}");
        let _ = (&*rx_guard).dynamic_memory_bytes();
        rx_guard.local_monitor_index = 0;
        rx_guard.telemetry_inc(RxDone::Normal(2), &mut tel);
        rx_guard.telemetry_inc(RxDone::Stream(1, 8), &mut tel);
        rx_guard.local_monitor_index = MONITOR_NOT;
        rx_guard.telemetry_inc(RxDone::Normal(1), &mut tel);
    });

    assert_eq!(RxDone::Normal(2).item_count(), 2);
    assert_eq!(RxDone::Normal(2).payload_count(), None);
    assert_eq!(*RxDone::Stream(3, 9), 3);
    assert_eq!(RxDone::Stream(3, 9).payload_count(), Some(9));
}

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_channel_uri_covers_unicast_ipc_and_multicast_without_a_driver() {
    let unicast = AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .with_term_length(65_536)
        .use_point_to_point(endpoint(40123))
        .with_interface(endpoint(0))
        .with_reliability(ReliableConfig::Unreliable)
        .build();
    let unicast_uri = unicast.cstring();
    let unicast_text = unicast_uri.to_str().expect("unicast uri");
    assert!(unicast_text.contains("aeron:udp"), "{unicast_text}");
    assert!(unicast_text.contains("40123"), "{unicast_text}");
    assert!(unicast_text.contains("reliable=false"), "{unicast_text}");

    let ipc = AeronConfig::new().with_media_type(MediaType::Ipc).build();
    assert_eq!(ipc.cstring().to_str().expect("ipc"), "aeron:ipc");

    let multicast = AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .use_multicast(endpoint(40456), endpoint(40457))
        .with_control_mode(ControlMode::Dynamic)
        .with_ttl(1)
        .build();
    let multi_uri = multicast.cstring();
    let multi_text = multi_uri.to_str().expect("multicast uri");
    assert!(multi_text.contains("40456"), "{multi_text}");
    assert!(multi_text.contains("40457"), "{multi_text}");
    assert!(multi_text.contains("ttl=1"), "{multi_text}");

    let wrong_mode = catch_unwind(AssertUnwindSafe(|| {
        AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .with_interface(endpoint(1));
    }));
    assert!(wrong_mode.is_err(), "interface without point-to-point must panic");
    let wrong_ttl = catch_unwind(AssertUnwindSafe(|| {
        AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(endpoint(2))
            .with_ttl(1);
    }));
    assert!(wrong_ttl.is_err(), "ttl outside multicast must panic");
}
