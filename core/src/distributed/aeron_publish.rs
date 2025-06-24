use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use aeron::exclusive_publication::ExclusivePublication;
use aeron::utils::types::Index;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::aqueduct_stream::{SteadyStreamRx, StreamEgress};
use crate::{await_for_any, RxCore, SteadyActor, SteadyState};
use crate::steady_actor_shadow::SteadyActorShadow;
use std::time::Duration;
use log::{warn, error};

// Reference to Aeron Best Practices Guide for performance optimization and configuration tips:
// https://github.com/real-logic/aeron/wiki/Best-Practices-Guide

// **Constants for Testing and Configuration**
/// Number of items to send in tests; increase for extended load testing.
pub const TEST_ITEMS: usize = 200_000_000;
/// Base stream ID for test publications.
pub const STREAM_ID: i32 = 11;
/// Term buffer size in MB; 64MB targets high message rates (e.g., 12M messages/sec).
pub const _TERM_MB: i32 = 64;
// A single stream at 64MB maps 400MB of shared memory. For optimal performance,
// tune SO_RCVBUF/SO_SNDBUF and check loopback queue length (e.g., `ip link set lo txqueuelen 10000`).

/// Manages Aeron-based message publishing for a single stream within the Steady State framework.
#[derive(Default)]
pub struct AeronPublishSteadyState {
    /// Optional registration ID for the Aeron publication, persisted across actor restarts.
    pub(crate) pub_reg_id: Option<i64>,
    /// Internal counter for items taken from the stream, used for tracking progress.
    pub(crate) _items_taken: usize,
}

/// Launches an Aeron publishing actor to transmit messages from a single stream.
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
        internal_behavior(actor, rx, aeron_connect, stream_id, aeron_media_driver, state).await
    } else {
        actor.simulated_behavior(vec![&rx]).await
    }
}

/// Core logic for publishing messages to a single Aeron stream.
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

    if state.pub_reg_id.is_none() {
        let mut aeron = aeron.lock().await;
        warn!("Adding new publication: stream_id={}, channel={:?}", stream_id, aeron_channel.cstring());
        match aeron.add_exclusive_publication(aeron_channel.cstring(), stream_id) {
            Ok(reg_id) => state.pub_reg_id = Some(reg_id),
            Err(e) => warn!("Failed to add publication: {:?}", e),
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
    } else {
        return Err("No publication registered. Check if Media Driver is running.".into());
    }

    warn!("Running publish for actor '{:?}' with publication in place", actor.identity());

    let capacity: usize = rx.capacity();
    let wait_for = (512 * 1024).min(capacity);

    let mut last_position = 0;
    let mut stream_flushed = false;
    while actor.is_running(&mut || rx.is_closed_and_empty() && stream_flushed) {
        let _clean = await_for_any!(
            actor.wait_periodic(Duration::from_millis(10)),
            actor.wait_avail(&mut rx, wait_for)
        );

        match &mut my_pub {
            Ok(p) => {
                if rx.is_closed_and_empty() && p.position().unwrap_or(0) >= last_position && !p.is_connected() {
                    stream_flushed = true;
                } else {
                    let vacant_aeron_bytes = p.available_window().unwrap_or(0);
                    if vacant_aeron_bytes > 0 {
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

/// Test module for validating Aeron publishing functionality for a single channel.
#[cfg(test)]
pub(crate) mod aeron_tests {
    use log::info;
    use super::*;
    use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
    use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
    use crate::distributed::aqueduct_builder::AqueductBuilder;
    use crate::distributed::aqueduct_stream::{SteadyStreamTx, StreamIngress};
    use crate::{await_for_all, AlertColor, Filled, GraphBuilder, Percentile, RxCore, ScheduleAs, Trigger};

    /// Mock sender actor for testing message transmission to Aeron.
    pub async fn mock_sender_run(
        context: SteadyActorShadow,
        tx: SteadyStreamTx<StreamEgress>,
    ) -> Result<(), Box<dyn Error>> {
        let mut actor = context.into_spotlight([], [&tx]);
        let mut tx = tx.lock().await;

        let data1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let data2 = [9, 10, 11, 12, 13, 14, 15, 16];

        const BATCH_SIZE: usize = 5000;
        let items: [StreamEgress; BATCH_SIZE] = [StreamEgress::new(8); BATCH_SIZE];
        let mut data: [[u8; 8]; BATCH_SIZE] = [data1; BATCH_SIZE];
        for i in 0..BATCH_SIZE {
            if i % 2 == 0 {
                data[i] = data1;
            } else {
                data[i] = data2;
            }
        }
        let all_bytes: Vec<u8> = data.iter().flatten().copied().collect();

        let mut sent_count = 0;
        while actor.is_running(&mut || tx.mark_closed()) {
            let vacant_items = 200000;
            let data_size = 8;
            let vacant_bytes = vacant_items * data_size;

            let _clean = await_for_all!(actor.wait_vacant(&mut tx, (vacant_items, vacant_bytes)));

            let mut remaining = TEST_ITEMS;
            while remaining > 0 && actor.vacant_units(&mut tx.control_channel) >= BATCH_SIZE {
                actor.send_slice(&mut tx.payload_channel, all_bytes.as_ref());
                actor.send_slice(&mut tx.control_channel, items.as_ref());

                sent_count += BATCH_SIZE;
                remaining -= BATCH_SIZE;
            }

            if sent_count >= TEST_ITEMS {
                tx.mark_closed();
                error!("sender is done");
                return Ok(());
            }
        }
        Ok(())
    }

    /// Mock receiver actor for testing message consumption from Aeron.
    pub async fn mock_receiver_run(
        context: SteadyActorShadow,
        rx: SteadyStreamRx<StreamIngress>,
    ) -> Result<(), Box<dyn Error>> {
        let mut actor = context.into_spotlight([&rx], []);
        let mut rx = rx.lock().await;

        let _data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
        let _data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

        const LEN: usize = 100_000;

        let mut received_count = 0;
        while actor.is_running(&mut || rx.is_closed_and_empty()) {
            let _clean = await_for_all!(actor.wait_avail(&mut rx, LEN));

            let bytes = actor.avail_units(&mut rx.payload_channel);
            actor.advance_take_index(&mut rx.payload_channel, bytes);
            let taken = actor.avail_units(&mut rx.control_channel);
            actor.advance_take_index(&mut rx.control_channel, taken);

            received_count += taken;
            if received_count >= (TEST_ITEMS - taken) {
                error!("stop requested");
                actor.request_shutdown().await;
                return Ok(());
            }
        }
        error!("receiver is done");
        Ok(())
    }

    /// Tests the end-to-end byte processing through Aeron for a single channel.
    #[async_std::test]
    async fn test_bytes_process() -> Result<(), Box<dyn Error>> {
        if true {
            return Ok(()); // Skip test by default.
        }
        if std::env::var("GITHUB_ACTIONS").is_ok() {
            return Ok(());
        }

        let mut graph = GraphBuilder::for_testing()
            .with_telemetry_metric_features(true)
            .build(());

        let aeron_md = graph.aeron_media_driver();
        if aeron_md.is_none() {
            info!("aeron test skipped, no media driver present");
            return Ok(());
        }

        let channel_builder = graph.channel_builder();

        let (to_aeron_tx, to_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .with_capacity(4 * 1024 * 1024)
            .build_stream::<StreamEgress>(8);

        let (from_aeron_tx, from_aeron_rx) = channel_builder
            .with_avg_rate()
            .with_avg_filled()
            .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
            .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
            .with_capacity(4 * 1024 * 1024)
            .build_stream::<StreamIngress>(8);

        let aeron_config = AeronConfig::new()
            .with_media_type(MediaType::Ipc) // IPC for maximum throughput.
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            .build();

        graph.actor_builder().with_name("MockSender")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())
            .build(
                move |context| mock_sender_run(context, to_aeron_tx.clone()),
                ScheduleAs::SoloAct,
            );

        let stream_id = 12;

        to_aeron_rx.build_aqueduct(
            AqueTech::Aeron(aeron_config.clone(), stream_id),
            &graph.actor_builder().with_name("SenderTest").never_simulate(true),
            ScheduleAs::SoloAct,
        );

        graph.actor_builder().with_name("MockReceiver")
            .with_thread_info()
            .with_mcpu_percentile(Percentile::p96())
            .with_mcpu_percentile(Percentile::p25())
            .build(
                move |context| mock_receiver_run(context, from_aeron_rx.clone()),
                ScheduleAs::SoloAct,
            );

        from_aeron_tx.build_aqueduct(
            AqueTech::Aeron(aeron_config.clone(), stream_id),
            &graph.actor_builder().with_name("ReceiverTest").never_simulate(true),
            ScheduleAs::SoloAct,
        );

        graph.start();
        graph.block_until_stopped(Duration::from_secs(21))
    }
}