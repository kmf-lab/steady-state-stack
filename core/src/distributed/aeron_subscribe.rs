use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use futures_timer::Delay;
use aeron::aeron::Aeron;
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use log::{error, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::aqueduct_stream::{SteadyStreamTx, StreamIngress};
use crate::{SteadyActor, SteadyState, StreamTx};
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::yield_now;

/// Steady state for the single-channel Aeron subscriber, tracking the subscription registration ID.
#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    /// The registration ID of the single subscription, None if not yet registered.
    sub_reg_id: Option<i64>,
}

/// Main entry point to run the single-channel Aeron subscriber actor.
///
/// This function initializes the actor and delegates to either internal Aeron behavior or simulated behavior
/// based on the configuration. It ensures the Aeron media driver is available before proceeding.
///
/// # Arguments
/// * `context` - The actor shadow context for managing the actor lifecycle.
/// * `tx` - The single stream transmitter for sending received data.
/// * `aeron_connect` - The Aeron channel configuration.
/// * `stream_id` - The stream ID for the subscription.
/// * `state` - The shared steady state for managing subscription registration.
///
/// # Returns
/// * `Result<(), Box<dyn Error>>` - Ok on success, Err on failure.
pub async fn run(
    context: SteadyActorShadow,
    tx: SteadyStreamTx<StreamIngress>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    // Initialize the actor with the single transmitter in the spotlight
    let mut actor = context.into_spotlight([], [&tx]);

    if actor.use_internal_behavior {
        // Wait for the Aeron media driver to become available
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if actor.is_running(&mut || tx.mark_closed()) {
                actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        let aeron_media_driver = actor.aeron_media_driver().expect("media driver");
        // Delegate to internal behavior with the media driver
        internal_behavior(actor, tx, aeron_connect, stream_id, aeron_media_driver, state).await
    } else {
        // Run simulated behavior if internal behavior is not used
        actor.simulated_behavior(vec![&tx]).await
    }
}

/// Internal behavior for managing the single Aeron subscription and polling loop.
///
/// This function handles subscription registration, waits for the subscription to become available,
/// and enters a dynamic polling loop to process incoming data.
///
/// # Arguments
/// * `actor` - The steady actor instance managing the lifecycle.
/// * `tx` - The single stream transmitter for sending received data.
/// * `aeron_channel` - The Aeron channel configuration.
/// * `stream_id` - The stream ID for the subscription.
/// * `aeron` - The shared Aeron instance for managing subscriptions.
/// * `state` - The shared steady state for tracking subscription status.
///
/// # Returns
/// * `Result<(), Box<dyn Error>>` - Ok on success, Err on failure.
async fn internal_behavior<C: SteadyActor>(
    mut actor: C,
    tx: SteadyStreamTx<StreamIngress>,
    aeron_channel: Channel,
    stream_id: i32,
    aeron: Arc<futures_util::lock::Mutex<Aeron>>,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let mut tx = tx.lock().await; // Lock the transmitter once at the start
    let mut state = state.lock(AeronSubscribeSteadyState::default).await;
    let mut sub: Option<Subscription> = None;

    // Register the subscription if not already done
    if state.sub_reg_id.is_none() {
        let mut aeron_guard = aeron.lock().await;
        let reg_id = aeron_guard.add_subscription(aeron_channel.cstring(), stream_id)?;
        warn!("new subscription registered: {}", reg_id);
        state.sub_reg_id = Some(reg_id);
    }

    // Wait for the subscription to become available
    while sub.is_none() {
        if let Some(id) = state.sub_reg_id {
            let mut aeron_guard = aeron.lock().await;
            match aeron_guard.find_subscription(id) {
                Ok(subscription) => {
                    match Arc::try_unwrap(subscription) {
                        Ok(mutex) => {
                            match mutex.into_inner() {
                                Ok(subscription) => sub = Some(subscription),
                                Err(_) => panic!("Failed to unwrap Mutex"),
                            }
                        },
                        Err(_) => panic!("Failed to unwrap Arc. Are there other references?"),
                    }
                }
                Err(e) => {
                    if e.to_string().contains("Awaiting") || e.to_string().contains("not ready") {
                        Delay::new(Duration::from_millis(2)).await;
                        if actor.is_liveliness_stop_requested() {
                            warn!("shutdown requested while waiting for subscription");
                            return Ok(());
                        }
                    } else {
                        return Err(format!("Error finding subscription: {:?}", e).into());
                    }
                }
            }
        } else {
            return Err("Subscription registration ID not set".into());
        }
    }

    let mut sub = sub.expect("internal error");
    error!("running subscriber '{:?}' with subscription in place", actor.identity());

    // Initialize polling schedule
    let mut next_poll_time = Instant::now();

    // Main polling loop with dynamic scheduling
    while actor.is_running(&mut || tx.mark_closed()) {
        let now = Instant::now();
        if now < next_poll_time {
            // Wait until the next scheduled poll time
            actor.wait_periodic(next_poll_time - now).await;
        }

        if !tx.shared_is_full() {
            // Poll the subscription and compute the next delay dynamically
            let delay = poll_aeron_subscription(&mut tx, &mut sub, &mut actor).await;
            next_poll_time = Instant::now() + delay;
        } else {
            // If the transmitter is full, wait based on processing rate
            let fastest_duration = tx.fastest_byte_processing_duration();
            next_poll_time = Instant::now() + if let Some(f) = fastest_duration {
                f * (tx.capacity().0 >> 1) as u32
            } else {
                tx.max_poll_latency
            };
        }
    }

    Ok(())
}

/// Polls the Aeron subscription and processes incoming fragments, returning the next poll delay.
///
/// This function polls the subscription, consumes fragments, updates data rate statistics,
/// and uses a scheduler to determine the next poll delay based on data arrival patterns.
///
/// # Arguments
/// * `tx` - The locked transmitter for processing received fragments.
/// * `sub` - The Aeron subscription to poll.
/// * `actor` - The steady actor instance for lifecycle management.
///
/// # Returns
/// * `Duration` - The computed delay until the next poll.
async fn poll_aeron_subscription<C: SteadyActor>(
    tx: &mut StreamTx<StreamIngress>,
    sub: &mut Subscription,
    actor: &mut C,
) -> Duration {
    let mut input_bytes: u32 = 0;
    let mut input_frags: u32 = 0;
    let now = Instant::now();

    // Capture current vacant capacity for statistics
    let measured_vacant_items = tx.control_channel.shared_vacant_units() as u32;
    let measured_vacant_bytes = tx.payload_channel.shared_vacant_units() as u32;

    // Poll the subscription until no more data or capacity is exhausted
    loop {
        let remaining_poll = tx.defrag_has_room_for();
        if remaining_poll == 0 {
            break;
        }
        if 0 >= sub.poll(
            &mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
                let flags = header.flags();
                let is_begin = (flags & frame_descriptor::BEGIN_FRAG) != 0;
                let is_end = (flags & frame_descriptor::END_FRAG) != 0;
                tx.fragment_consume(
                    header.session_id(),
                    buffer.as_sub_slice(offset, length),
                    is_begin,
                    is_end,
                    now,
                );
                input_bytes += length as u32;
                input_frags += 1;
            },
            remaining_poll as i32,
        ) {
            break; // No more data available
        }
        yield_now().await; // Yield to allow more data processing in this pass
    }

    // Flush any ready messages and update output statistics
    if !tx.ready_msg_session.is_empty() {
        let (now_sent_messages, now_sent_bytes) = tx.fragment_flush_ready(actor);
        let (stored_vacant_items, stored_vacant_bytes) = tx.get_stored_vacant_values();

        if stored_vacant_items > measured_vacant_items as i32 {
            let duration = now.duration_since(tx.last_output_instant);
            tx.store_output_data_rate(
                duration,
                (stored_vacant_items - measured_vacant_items as i32) as u32,
                (stored_vacant_bytes - measured_vacant_bytes as i32) as u32,
            );
            tx.last_output_instant = now;
            tx.set_stored_vacant_values(
                measured_vacant_items as i32 - now_sent_messages as i32,
                measured_vacant_bytes as i32 - now_sent_bytes as i32,
            );
        }
    }

    // Update input data rate if fragments were received
    if input_frags > 0 {
        let duration = now.duration_since(tx.last_input_instant);
        tx.store_input_data_rate(duration, input_frags, input_bytes);
        tx.last_input_instant = now;
    }

    // Compute the next poll delay using the scheduler
    let (avg, std) = tx.guess_duration_between_arrivals();
    let (min, max) = tx.next_poll_bounds();
    let mut scheduler = polling::PollScheduler::new();
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let now_ns = now.duration_since(tx.last_input_instant).as_nanos() as u64;
    Duration::from_nanos(scheduler.compute_next_delay_ns(now_ns))
}

/// Unit tests for the single-channel Aeron subscriber.
#[cfg(test)]
pub(crate) mod aeron_media_driver_tests {
    use log::info;
    use super::*;
    use crate::distributed::aeron_channel_structs::{Endpoint, MediaType};
    use crate::distributed::aqueduct_stream::StreamEgress;
    use crate::distributed::aeron_channel_builder::{AeronConfig, AqueTech};
    use crate::distributed::aeron_publish::STREAM_ID;
    use crate::distributed::aqueduct_builder::AqueductBuilder;
    use crate::{GraphBuilder, SoloAct};

    /// Tests the processing of bytes through the single-channel Aeron subscriber.
    #[test]
    fn test_bytes_process() -> Result<(), Box<dyn Error>> {
        if std::env::var("GITHUB_ACTIONS").is_ok() {
            return Ok(()); // Skip in CI environment
        }

        let mut graph = GraphBuilder::for_testing().build(());
        if let Some(md) = graph.aeron_media_driver() {
            if let Some(md) = md.try_lock() {
                info!("Found MediaDriver cnc:{:?}", md.context().cnc_file_name());
            }
        } else {
            info!("aeron test skipped, no media driver present");
            return Ok(());
        }

        let channel_builder = graph.channel_builder();

        // Create single streams instead of a bundle
        let (to_aeron_tx, to_aeron_rx) = channel_builder
            .with_capacity(500)
            .build_stream::<StreamEgress>(6);

        let aeron_config = AeronConfig::new()
            .with_media_type(MediaType::Ipc) // Use IPC for testing
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Invalid IP address"),
                port: 40456,
            })
            .build();

        // Build the publisher aqueduct
        to_aeron_rx.build_aqueduct(
            AqueTech::Aeron(aeron_config.clone(), STREAM_ID),
            &graph.actor_builder().with_name("SenderTest").never_simulate(true),
            SoloAct,
        );

        // Send test frames
        for _i in 0..100 {
            to_aeron_tx.testing_send_frame(&[1, 2, 3, 4, 5]);
            to_aeron_tx.testing_send_frame(&[6, 7, 8, 9, 10]);
        }
        to_aeron_tx.testing_close();

        // Create receiver stream
        let (from_aeron_tx, _from_aeron_rx) = channel_builder
            .with_capacity(500)
            .build_stream::<StreamIngress>(6);

        // Build the subscriber aqueduct
        from_aeron_tx.build_aqueduct(
            AqueTech::Aeron(aeron_config, STREAM_ID),
            &graph.actor_builder().with_name("ReceiverTest").never_simulate(true),
            SoloAct,
        );

        // Run the graph and shut down
        graph.start();
        graph.request_shutdown();
        let _unclean = graph.block_until_stopped(Duration::from_secs(2));

        Ok(())
    }
}