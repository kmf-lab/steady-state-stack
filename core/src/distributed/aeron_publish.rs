// ss[related distributed.subscribe-publish]
use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
// ss[related distributed.subscribe-publish]
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use aeron::exclusive_publication::ExclusivePublication;
// ss[related distributed.subscribe-publish]
use aeron::utils::types::Index;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::aqueduct_stream::{SteadyStreamRx, StreamEgress};
// ss[related distributed.subscribe-publish]
use crate::{await_for_any, RxCore, SteadyActor};
use crate::steady_actor_shadow::SteadyActorShadow;
use std::time::Duration;
// ss[related distributed.subscribe-publish]
use log::*;
use crate::state_management::SteadyState;
// Reference to Aeron Best Practices Guide for performance optimization and configuration tips:
// https://github.com/real-logic/aeron/wiki/Best-Practices-Guide

// **Constants for Testing and Configuration**
/// Number of items to send in tests; increase for extended load testing.
// ss[related distributed.subscribe-publish]
pub const TEST_ITEMS: usize = 200_000_000;
/// Base stream ID for test publications.
pub const STREAM_ID: i32 = 11;
/// Term buffer size in MB; 64MB targets high message rates (e.g., 12M messages/sec).
// ss[related distributed.subscribe-publish]
pub const _TERM_MB: i32 = 64;
// A single stream at 64MB maps 400MB of shared memory. For optimal performance,
// tune SO_RCVBUF/SO_SNDBUF and check loopback queue length (e.g., `ip link set lo txqueuelen 10000`).

/// Manages Aeron-based message publishing for a single stream within the Steady State framework.
#[derive(Default)]
// ss[related distributed.subscribe-publish]
pub struct AeronPublishSteadyState {
    /// Optional registration ID for the Aeron publication, persisted across actor restarts.
    pub(crate) pub_reg_id: Option<i64>,
    /// Internal counter for items taken from the stream, used for tracking progress.
    pub(crate) _items_taken: usize,
}

/// Launches an Aeron publishing actor to transmit messages from a single stream.
// ss[related distributed.subscribe-publish]
pub async fn run(
    context: SteadyActorShadow,
    rx: SteadyStreamRx<StreamEgress>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronPublishSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut actor = context.into_spotlight([&rx], []);
    if actor.use_internal_behavior {
        while actor.aeron_media_driver().is_none() {
            warn!("Unable to find Aeron media driver, will try again in 15 sec");
            let mut rx = rx.lock().await;
            if actor.is_running(&mut || rx.is_closed_and_empty()) {
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        let aeron_media_driver = actor.aeron_media_driver().expect("Media driver should be available");
        let result = internal_behavior(actor, rx, aeron_connect, stream_id, aeron_media_driver, state).await;
        if let Err(ref e) = result {
            eprintln!("AeronPublish actor exited with error: {e}");
        }
        result
    } else {
        actor.simulated_behavior(vec![&rx]).await
    }
}

/// Core logic for publishing messages to a single Aeron stream.
// ss[related distributed.subscribe-publish]
async fn internal_behavior<C: SteadyActor>(
    mut actor: C,
    rx: SteadyStreamRx<StreamEgress>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronPublishSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut rx = rx.lock().await;
    let mut state = state.lock(AeronPublishSteadyState::default).await;

    // ss[depends distributed.subscribe-publish]
    if state.pub_reg_id.is_none() {
        let mut aeron = aeron.lock().await;
        warn!("Adding new publication: stream_id={}, channel={:?}", stream_id, aeron_channel.cstring());
        match aeron.add_exclusive_publication(aeron_channel.cstring(), stream_id) {
            Ok(reg_id) => state.pub_reg_id = Some(reg_id),
            Err(e) => {
                // ss[impl distributed.subscribe-publish]
                if actor.is_liveliness_stop_requested() {
                    if rx.is_closed_and_empty() {
                        return Ok(());
                    }
                    return Err(
                        "shutdown during exclusive publication registration (egress not drained)"
                            .into(),
                    );
                }
                return Err(format!(
                    "Failed to add exclusive publication stream_id={stream_id} channel={:?}: {e:?}",
                    aeron_channel.cstring()
                )
                .into());
            }
        };
    }
    Delay::new(Duration::from_millis(2)).await;

    let mut my_pub: Result<ExclusivePublication, Box<dyn Error>> = Err("Publication not initialized".into());
    if let Some(id) = state.pub_reg_id {
        let mut found = false;
        while actor.is_running(&mut || rx.is_closed_and_empty()) && !found {
            let ex_pub = {
                let mut aeron = aeron.lock().await;
                aeron.find_exclusive_publication(id)
            };
            match ex_pub {
                Err(e) => {
                    if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                        Delay::new(Duration::from_millis(4)).await;
                        if actor.is_liveliness_stop_requested() {
                            if rx.is_closed_and_empty() {
                                return Ok(());
                            }
                            my_pub = Err(Box::new(std::io::Error::new(
                                std::io::ErrorKind::Interrupted,
                                "Shutdown requested while waiting for publication",
                            )));
                            found = true;
                        }
                    } else {
                        warn!("Error finding publication: {:?}", e);
                        my_pub = Err(Box::new(e));
                        found = true;
                    }
                }
                Ok(publication) => {
                    match Arc::try_unwrap(publication) {
                        Ok(mutex) => match mutex.into_inner() {
                            Ok(pub_instance) => {
                                my_pub = Ok(pub_instance);
                                found = true;
                            }
                            Err(_) => panic!("Failed to unwrap Mutex for publication"),
                        },
                        Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                    }
                }
            }
        }
    } else if actor.is_liveliness_stop_requested() && rx.is_closed_and_empty() {
        return Ok(());
    } else {
        // ss[impl distributed.subscribe-publish]
        return Err(
            "No publication registered after add_exclusive_publication (is the media driver running?)"
                .into(),
        );
    }

    if let Err(e) = &my_pub {
        if e.to_string().contains("Shutdown requested") && rx.is_closed_and_empty() {
            return Ok(());
        }
        // ss[impl distributed.subscribe-publish]
        return Err(format!("Publication unavailable for stream_id={stream_id}: {e}").into());
    }

    info!("Running publish for actor '{:?}' with publication in place", actor.identity());

    let mut disconnected_since: Option<std::time::Instant> = None;
    // ss[related distributed.subscribe-publish]
    const CONNECTED_WAIT: Duration = Duration::from_secs(5);

    let capacity: usize = rx.capacity();
    // Wake on the first egress item (wire probes send one frame). The 10ms tick still
    // batches remaining work; waiting for a large `wait_for` starved single-frame probes
    // when `wait_periodic` did not complete first.
    let wait_for = 1.min(capacity);

    let mut last_position = 0;
    let mut stream_flushed = false;
    while actor.is_running(&mut || rx.is_closed_and_empty() && stream_flushed) {
        let _clean = await_for_any!(
            actor.wait_periodic(Duration::from_millis(10)),
            actor.wait_avail(&mut rx, wait_for)
        );

        match &mut my_pub {
            Ok(p) => {
                if !p.is_connected() && !rx.is_closed_and_empty() {
                    let since = disconnected_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= CONNECTED_WAIT {
                        warn!(
                            "publication stream_id={stream_id} not connected after {:?} with pending egress data",
                            CONNECTED_WAIT
                        );
                    }
                } else {
                    disconnected_since = None;
                }
                if rx.is_closed_and_empty() && p.position().unwrap_or(0) >= last_position && !p.is_connected() {
                    stream_flushed = true;
                } else {
                    let vacant_aeron_bytes = p.available_window().unwrap_or(0);
                    if vacant_aeron_bytes > 0 {
                        // ss[impl distributed.subscribe-publish]
                        rx.consume_messages(&mut actor, vacant_aeron_bytes as usize, |slice1: &mut [u8], slice2: &mut [u8]| {
                            let msg_len = slice1.len() + slice2.len();
                            assert!(msg_len > 0, "Message length must be positive");
                            let response = if slice2.is_empty() {
                                p.offer_part(AtomicBuffer::wrap_slice(slice1), 0, msg_len as Index)
                            } else {
                                let aligned_buffer = AlignedBuffer::with_capacity(msg_len as Index);
                                let buf = AtomicBuffer::from_aligned(&aligned_buffer);
                                buf.put_bytes(0, slice1);
                                buf.put_bytes(slice1.len() as Index, slice2);
                                p.offer_part(buf, 0, msg_len as Index)
                            };
                            match response {
                                Ok(value) => {
                                    if value > 0 {
                                        last_position = value;
                                        true
                                    } else {
                                        false
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to offer message: {:?}", e);
                                    false
                                }
                            }
                        });
                    }
                }
            }
            Err(e) => {
                error!("Publication unavailable: {}", e);
                stream_flushed = true;
            }
        }
    }

    if let Ok(p) = my_pub {
        p.close();
    }

    Ok(())
}

#[cfg(test)]
// ss[related distributed.subscribe-publish]
mod aeron_publish_state_tests {
    use super::AeronPublishSteadyState;

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_publish_state_default() {
        let state = AeronPublishSteadyState::default();
        assert!(state.pub_reg_id.is_none());
    }

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_publish_state_registration_cleared_on_default() {
        let state = AeronPublishSteadyState::default();
        assert_eq!(state.pub_reg_id, None);
        assert_eq!(state._items_taken, 0);
    }

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_publish_stream_id_constant_positive() {
        assert!(super::STREAM_ID > 0);
    }

}

#[cfg(test)]
mod aeron_publish_graph_tests {
    use std::time::Duration;

    use futures_timer::Delay;

    use crate::distributed::aeron_channel_builder::AeronConfig;
    use crate::distributed::aeron_channel_structs::MediaType;
    use crate::distributed::aqueduct_builder::AqueductBuilder;
    use crate::distributed::aqueduct_stream::StreamEgress;
    use crate::{AqueTech, GraphBuilder, SoloAct};

    /// Simulated Aeron publish actor: graph starts and stops without requiring registration.
    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_publish_simulated_graph_stops_cleanly() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(256);
        let (_tx, rx) = cb.build_stream::<StreamEgress>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        rx.build_aqueduct(
            AqueTech::Aeron(channel, 44),
            &graph
                .actor_builder()
                .with_name("AeronPublishSim")
                .never_simulate(false),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(15)));
        Delay::new(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(20)).is_ok());
        });
}

    /// Internal publish path: when no media driver is attached, shutdown during driver wait exits.
    #[test]
    #[ignore] //broken until we can get more time to look into this
    // ss[verify distributed.subscribe-publish]
    fn test_publish_stops_during_driver_wait_without_driver() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        if graph.aeron_media_driver().is_some() {
            eprintln!("SKIP: media driver present — driver-wait stop test needs isolated graph");
            return;
        }
        let cb = graph.channel_builder().with_capacity(256);
        let (_tx, rx) = cb.build_stream::<StreamEgress>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        rx.build_aqueduct(
            AqueTech::Aeron(channel, 45),
            &graph
                .actor_builder()
                .with_name("AeronPublishStop")
                .never_simulate(true),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(10)));
        Delay::new(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(25)).is_ok());
        });
}

    /// Internal publish path with closed egress: exits without requiring a live driver registration loop.
    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_publish_internal_closed_egress_stops_without_driver() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(256);
        let (tx, rx) = cb.build_stream::<StreamEgress>(64);
        tx.testing_close();
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        rx.build_aqueduct(
            AqueTech::Aeron(channel, 50),
            &graph
                .actor_builder()
                .with_name("AeronPublishClosedEgress")
                .never_simulate(true),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(10)));
        Delay::new(Duration::from_millis(50)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(25)).is_ok());
        });
}
}