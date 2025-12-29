use std::error::Error;
use std::sync::Arc;
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::{AlignedBuffer, AtomicBuffer};
use aeron::exclusive_publication::ExclusivePublication;
use aeron::utils::types::Index;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::aqueduct_stream::{SteadyStreamRxBundle, SteadyStreamRxBundleTrait, StreamEgress, StreamRxBundleTrait};
use crate::SteadyActor;
use crate::*;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::simulate_edge::IntoSimRunner;
use crate::state_management::SteadyState;
// Reference to Aeron Best Practices Guide for performance optimization and configuration tips:
// https://github.com/real-logic/aeron/wiki/Best-Practices-Guide

/// Manages Aeron-based message publishing within the Steady State framework.
///
/// This module provides an actor-based implementation for publishing messages from a stream to an Aeron media driver.
/// It leverages Aeron's high-performance messaging capabilities to achieve low-latency, high-throughput communication.
/// Key components include state management for publication registration, efficient message publishing loops, and
/// robust shutdown handling. The module is designed to integrate seamlessly with the Steady State actor system,
/// supporting both real and simulated behaviors for testing.
///
/// For optimal performance, ensure the Aeron media driver is running and configured appropriately (e.g., term buffer
/// size, SO_RCVBUF/SO_SNDBUF settings). See the Aeron Best Practices Guide for details.
///
/// Represents the persistent state for Aeron publishing across actor restarts.
///
/// This struct tracks publication registration IDs and an internal counter for items processed, enabling the actor
/// to resume publishing seamlessly after interruptions.
#[derive(Default)]
pub struct AeronPublishSteadyState {
    /// Vector of optional registration IDs for Aeron publications, one per stream.
    ///
    /// Each entry corresponds to a stream in the bundle, storing the Aeron-assigned ID for its publication.
    /// `None` indicates the publication has not yet been registered.
    pub(crate) pub_reg_id: Vec<Option<i64>>,
    /// Internal counter for items taken from the stream, used for tracking purposes.
    pub(crate) _items_taken: usize,
}

/// Launches an Aeron publishing actor to transmit messages from a stream.
///
/// This asynchronous function initializes and runs the publishing actor, handling both real and simulated behaviors.
/// In real mode, it connects to the Aeron media driver and delegates to `internal_behavior` for publishing logic.
/// In simulation mode, it executes a test-friendly simulated behavior.
///
/// # Parameters
/// - `context`: The actor's context, providing framework utilities and identity.
/// - `rx`: The receiver bundle containing stream data to publish.
/// - `aeron_connect`: Configuration for connecting to the Aeron media driver.
/// - `stream_id`: Base stream ID for assigning publication IDs.
/// - `state`: Persistent state for tracking publication registrations.
///
/// # Returns
/// A `Result` indicating success (`Ok(())`) or an error if the media driver is unavailable or setup fails.
///
/// # Aeron Insights
/// - The function retries every 15 seconds if the media driver is not found, ensuring robustness in distributed setups.
/// - Publications are exclusive (single-writer), aligning with Aeron's recommendation for high-throughput scenarios.
pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    rx: SteadyStreamRxBundle<StreamEgress, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronPublishSteadyState>,
) -> Result<(), Box<dyn Error>> {
    // Initialize the actor with a spotlight, focusing on the stream's payload metadata.
    // The empty array indicates no control metadata is provided here; ensure the receiver end matches this expectation.
    let mut actor = context.into_spotlight(rx.payload_meta_data(), []);

    if actor.use_internal_behavior {
        // Poll for the Aeron media driver, essential for publishing.
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut rx = rx.lock().await;
            if actor.is_running(&mut || rx.is_closed_and_empty()) {
                // Wait periodically to avoid busy-waiting, respecting Aeron's startup time.
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                // Exit gracefully if the actor is stopped before the driver is found.
                return Ok(());
            }
        }
        let aeron_media_driver = actor.aeron_media_driver().expect("media driver");
        // Delegate to the internal logic with the established Aeron connection.
        return internal_behavior(actor, rx, aeron_connect, stream_id, aeron_media_driver, state).await;
    }
    // Simulation mode: Collect stream iterators and run a simulated behavior for testing.
    let te: Vec<_> = rx.iter().cloned().collect();
    let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
    actor.simulated_behavior(sims).await
}

/// Message indicating shutdown during initialization.
const SHUTDOWN_ON_INIT_MESSAGE: &str = "Shutdown requested while waiting";

/// Core logic for publishing messages to Aeron streams.
///
/// This internal function handles the setup, publishing, and cleanup of Aeron publications. It ensures messages are
/// efficiently transmitted from the stream bundle to the Aeron media driver, managing backpressure and shutdown
/// gracefully.
///
/// # Parameters
/// - `actor`: The actor instance driving the publishing process.
/// - `rx`: The receiver bundle providing messages to publish.
/// - `aeron_channel`: Configuration for the Aeron channel.
/// - `stream_id`: Base stream ID for publication assignments.
/// - `aeron`: Shared Aeron instance for managing publications.
/// - `state`: Persistent state for registration IDs.
///
/// # Returns
/// A `Result` indicating success (`Ok(())`) or an error if publication setup or operation fails.
///
/// # Aeron Insights
/// - Uses exclusive publications for single-writer efficiency, minimizing contention.
/// - Employs `offer_part` for message publishing, with potential for `try_claim` in future optimizations for zero-copy.
/// - Monitors the available window to respect Aeron's flow control, preventing buffer overruns.
async fn internal_behavior<const GIRTH: usize, C: SteadyActor>(
    mut actor: C,
    rx: SteadyStreamRxBundle<StreamEgress, GIRTH>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronPublishSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut rx = rx.lock().await;
    let mut state = state.lock(AeronPublishSteadyState::default).await;

    // Array to store publication results for each stream; initialized with errors until setup completes.
    let mut pubs: [Result<ExclusivePublication, Box<dyn Error>>; GIRTH] =
        std::array::from_fn(|_| Err("Not Found".into()));
    // Tracks the last published position for each stream to ensure all messages are delivered.
    let mut last_position: [i64; GIRTH] = [0; GIRTH];

    // Ensure the state vector matches the number of streams (GIRTH).
    while state.pub_reg_id.len() < GIRTH {
        state.pub_reg_id.push(None);
    }

    {
        let mut aeron = aeron.lock().await; // Lock Aeron briefly to register publications.
        for f in 0..GIRTH {
            if state.pub_reg_id[f].is_none() {
                // Register a new exclusive publication if not already present.
                trace!("adding new pub {} {:?}", f as i32 + stream_id, aeron_channel.cstring());
                match aeron.add_exclusive_publication(aeron_channel.cstring(), f as i32 + stream_id) {
                    Ok(reg_id) => state.pub_reg_id[f] = Some(reg_id),
                    Err(e) => warn!("Unable to add publication: {:?}", e),
                };
            }
        }
    }
    // Brief delay to allow the media driver to process registrations, avoiding immediate polling.
    Delay::new(Duration::from_millis(2)).await;

    // Wait for publications to become available, polling Aeron until ready or shutdown is requested.
    for f in 0..GIRTH {
        if let Some(id) = state.pub_reg_id[f] {
            let mut found = false;
            while actor.is_running(&mut || rx.is_closed_and_empty()) && !found {
                let ex_pub = {
                    let mut aeron = aeron.lock().await;
                    aeron.find_exclusive_publication(id)
                };
                match ex_pub {
                    Err(e) => {
                        if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                            // Back off to avoid overwhelming the driver during setup.
                            Delay::new(Duration::from_millis(4)).await;
                            if actor.is_liveliness_stop_requested() {
                                pubs[f] = Err(SHUTDOWN_ON_INIT_MESSAGE.into());
                                found = true;
                            }
                        } else {
                            warn!("Error finding publication: {:?}", e);
                            pubs[f] = Err(e.into());
                            found = true;
                        }
                    }
                    Ok(publication) => {
                        match Arc::try_unwrap(publication) {
                            Ok(mutex) => match mutex.into_inner() {
                                Ok(publication) => {
                                    pubs[f] = Ok(publication);
                                    found = true;
                                }
                                Err(_) => panic!("Failed to unwrap Mutex"),
                            },
                            Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                        }
                    }
                }
            }
        } else {
            return Err("Check if Media Driver is running.".into());
        }
    }

    info!("running: '{:?}' all publications in place", actor.identity().label);

    // Threshold for waiting on messages; set to 1/16th of capacity to balance latency and throughput.
    let wait_for = rx.capacity() / 16;
    let in_channels = 1;

    while actor.is_running(&mut || i!(rx.is_closed_and_empty())  ) {
        // Wait for either a periodic tick or available messages, optimizing CPU usage.
        let _clean = await_for_any!(
            actor.wait_periodic(Duration::from_millis(500)),
            actor.wait_avail_bundle(&mut rx, wait_for, in_channels)
        );

        for index in 0..GIRTH {
            match &mut pubs[index] {
                Ok(p) => {
                    // Query Aeron's available window to respect flow control.
                    let mut vacant_aeron_bytes = p.available_window().unwrap_or(0);
                    let mut avail_messages = rx[index].avail_units();
                    loop {
                        let mut count_done = 0;
                        let mut count_bytes = 0;

                        if vacant_aeron_bytes > 0 && avail_messages.0 > 0 {
                            // Consume and publish messages within the available window.
                            rx[index].consume_messages(&mut actor, vacant_aeron_bytes as usize, |slice1: &mut [u8], slice2: &mut [u8]| {
                                let msg_len = slice1.len() + slice2.len();
                                assert!(msg_len > 0);

                                let response = if slice2.is_empty() {
                                    p.offer_part(AtomicBuffer::wrap_slice(slice1), 0, msg_len as Index)
                                } else if slice1.is_empty() {
                                        p.offer_part(AtomicBuffer::wrap_slice(slice2), 0, msg_len as Index)
                                    } else {
                                        // Handle fragmented messages by combining into a single buffer.
                                        let a_len = msg_len.min(slice1.len());
                                        let aligned_buffer = AlignedBuffer::with_capacity(msg_len as Index);
                                        let buf = AtomicBuffer::from_aligned(&aligned_buffer);
                                        buf.put_bytes(0, slice1);
                                        buf.put_bytes(a_len as Index, slice2);
                                        p.offer_part(buf, 0, msg_len as Index)
                                    };
                                

                                match response {
                                    Ok(value) => {
                                        if value > 0 {
                                            let dif: i64 = value - last_position[index];
                                            last_position[index] = value;
                                            count_done += 1;
                                            if dif < (msg_len as i64) {
                                                error!("warning not packing the data sent {} to send {}", dif, msg_len);
                                            }
                                            count_bytes += msg_len;
                                            true
                                        } else {
                                            false
                                        }
                                    }
                                    Err(_aeron_error) => false,
                                }
                            });
                        }

                        if count_done == 0 {
                            break; // Exit loop if no more messages were published.
                        }
                        // Refresh window and message availability for the next iteration.
                        vacant_aeron_bytes = p.available_window().unwrap_or(0);
                        avail_messages = rx[index].avail_units();
                    }


                }
                Err(e) => {
                    trace!("{}", e);
                    yield_now().await;
                }
            }
        }
    }

    // Ensure all messages are delivered before closing publications.
    for index in 0..GIRTH {
        if let Ok(p) = &mut pubs[index] {
            while p.position().unwrap_or(0) < last_position[index] || p.is_connected() {
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            p.close(); // Explicitly close to free resources.
        }
    }

    Ok(())
}

/// Test module for validating Aeron publishing functionality.
///
/// This module includes mock sender and receiver actors, along with a test case to simulate message flow through Aeron.
/// It exercises the publishing logic under controlled conditions, ensuring reliability and performance.
#[cfg(test)]
pub(crate) mod aeron_publish_bundle_tests {
    use super::*;
    use crate::distributed::aqueduct_stream::{SteadyStreamTxBundle, SteadyStreamTxBundleTrait, StreamIngress, StreamTxBundleTrait};
    use crate::distributed::aqueduct_stream::StreamEgress;

    /// Number of items to send in tests; increase for extended load testing.
    pub const TEST_ITEMS: usize = 200_000_000;

    /// Base stream ID for test publications.
    pub const STREAM_ID: i32 = 11;
    /// Term buffer size in MB; 64MB targets high message rates (e.g., 12M messages/sec).
    pub const _TERM_MB: i32 = 64;
    // A single stream at 64MB maps 400MB of shared memory. For optimal performance,
    // tune SO_RCVBUF/SO_SNDBUF and check loopback queue length (e.g., `ip link set lo txqueuelen 10000`).

    #[test]
    fn test_publish_state_init() {
        let state = AeronPublishSteadyState::default();
        assert!(state.pub_reg_id.is_empty());
        assert_eq!(state._items_taken, 0);
    }

    /// Mock sender actor for testing message transmission to Aeron.
    ///
    /// Sends batches of test data to the stream, simulating a producer.
    pub async fn mock_sender_run<const GIRTH: usize>(
        context: SteadyActorShadow,
        tx: SteadyStreamTxBundle<StreamEgress, GIRTH>,
    ) -> Result<(), Box<dyn Error>> {
        let mut actor = context.into_spotlight([], tx.control_meta_data());
        let mut tx = tx.lock().await;

        let data1 = [1, 2, 3, 4, 5, 6, 7, 8];
        let data2 = [9, 10, 11, 12, 13, 14, 15, 16];

        const BATCH_SIZE: usize = 5000;
        let items: [StreamEgress; BATCH_SIZE] = [StreamEgress { length: 8 }; BATCH_SIZE];
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

            let _clean = await_for_all!(actor.wait_vacant_bundle(&mut tx, (vacant_items, vacant_bytes), 1));

            let mut remaining = TEST_ITEMS;
            let idx: usize = (0 - STREAM_ID) as usize;
            while remaining > 0 && actor.vacant_units(&mut tx[idx].control_channel) >= BATCH_SIZE {
                actor.send_slice(&mut tx[idx].payload_channel, all_bytes.as_ref());
                actor.send_slice(&mut tx[idx].control_channel, items.as_ref());

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
    ///
    /// Consumes messages from the stream, simulating a consumer.
    pub async fn mock_receiver_run<const GIRTH: usize>(
        context: SteadyActorShadow,
        rx: SteadyStreamRxBundle<StreamIngress, GIRTH>,
    ) -> Result<(), Box<dyn Error>> {
        let mut actor = context.into_spotlight(rx.control_meta_data(), []);
        let mut rx = rx.lock().await;

        let _data1 = Box::new([1, 2, 3, 4, 5, 6, 7, 8]);
        let _data2 = Box::new([9, 10, 11, 12, 13, 14, 15, 16]);

        const LEN: usize = 100_000;

        let mut received_count = 0;
        while actor.is_running(&mut || rx.is_closed_and_empty()) {
            let _clean = await_for_all!(actor.wait_avail_bundle(&mut rx, LEN, 1));

            let bytes = actor.avail_units(&mut rx[0].payload_channel);
            actor.advance_take_index(&mut rx[0].payload_channel, bytes);
            let taken = actor.avail_units(&mut rx[0].control_channel);
            actor.advance_take_index(&mut rx[0].control_channel, taken);

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


    // #[test] //TODO: never returns
    // fn test_bytes_process() -> Result<(), Box<dyn Error>> {
    //     crate::core_exec::block_on(async {
    //         if std::env::var("GITHUB_ACTIONS").is_ok() {
    //             return Ok(());
    //         }
    //
    //         let mut graph = GraphBuilder::for_testing()
    //             .with_telemetry_metric_features(true)
    //             .build(());
    //
    //         let aeron_md = graph.aeron_media_driver();
    //         if aeron_md.is_none() {
    //             info!("aeron test skipped, no media driver present");
    //             return Ok(());
    //         }
    //
    //         let channel_builder = graph.channel_builder();
    //
    //         let (to_aeron_tx, to_aeron_rx) = channel_builder
    //             .with_avg_rate()
    //             .with_avg_filled()
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
    //             .with_capacity(4 * 1024 * 1024)
    //             .build_stream_bundle::<StreamEgress, 1>(8);
    //
    //         let (from_aeron_tx, from_aeron_rx) = channel_builder
    //             .with_avg_rate()
    //             .with_avg_filled()
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow)
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p70()), AlertColor::Orange)
    //             .with_filled_trigger(Trigger::AvgAbove(Filled::p90()), AlertColor::Red)
    //             .with_capacity(4 * 1024 * 1024)
    //             .build_stream_bundle::<StreamIngress, 1>(8);
    //
    //         let aeron_config = AeronConfig::new()
    //             .with_media_type(MediaType::Ipc) // IPC for maximum throughput.
    //             .use_point_to_point(Endpoint {
    //                 ip: "127.0.0.1".parse().expect("Invalid IP address"),
    //                 port: 40456,
    //             })
    //             .build();
    //
    //         graph.actor_builder().with_name("MockSender")
    //             .with_thread_info()
    //             .with_mcpu_percentile(Percentile::p96())
    //             .with_mcpu_percentile(Percentile::p25())
    //             .build(
    //                 move |context| mock_sender_run(context, to_aeron_tx.clone()),
    //                 ScheduleAs::SoloAct,
    //             );
    //
    //         let stream_id = 12;
    //
    //         to_aeron_rx.build_aqueduct(
    //             AqueTech::Aeron(aeron_config.clone(), stream_id),
    //             &graph.actor_builder().with_name("SenderTest").never_simulate(true),
    //             ScheduleAs::SoloAct,
    //         );
    //
    //         graph.actor_builder().with_name("MockReceiver")
    //             .with_thread_info()
    //             .with_mcpu_percentile(Percentile::p96())
    //             .with_mcpu_percentile(Percentile::p25())
    //             .build(
    //                 move |context| mock_receiver_run(context, from_aeron_rx.clone()),
    //                 ScheduleAs::SoloAct,
    //             );
    //
    //         let from_aeron_tx = from_aeron_tx;
    //         from_aeron_tx.build_aqueduct(
    //             AqueTech::Aeron(aeron_config.clone(), stream_id),
    //             &graph.actor_builder().with_name("ReceiverTest").never_simulate(true),
    //             ScheduleAs::SoloAct,
    //         );
    //
    //         graph.start();
    //         graph.block_until_stopped(Duration::from_secs(21))
    //     })
    // }
}
