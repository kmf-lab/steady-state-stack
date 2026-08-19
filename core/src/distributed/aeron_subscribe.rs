// ss[related distributed.subscribe-publish]
use std::error::Error;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use std::time::{Duration, Instant};
// ss[related distributed.subscribe-publish]
use futures_timer::Delay;
// ss[related philosophy.structural-hierarchy]
use aeron::aeron::Aeron;
// ss[related philosophy.structural-hierarchy]
use aeron::concurrent::atomic_buffer::AtomicBuffer;
// ss[related distributed.subscribe-publish]
use aeron::concurrent::logbuffer::frame_descriptor;
// ss[related philosophy.structural-hierarchy]
use aeron::concurrent::logbuffer::header::Header;
// ss[related philosophy.structural-hierarchy]
use aeron::subscription::Subscription;
// ss[related distributed.subscribe-publish]
use log::{error, warn};
// ss[related philosophy.structural-hierarchy]
use crate::distributed::aeron_channel_structs::Channel;
// ss[related philosophy.structural-hierarchy]
use crate::distributed::aqueduct_stream::{SteadyStreamTx, StreamIngress};
// ss[related distributed.subscribe-publish]
use crate::{SteadyActor, StreamTx};
// ss[related philosophy.structural-hierarchy]
use crate::steady_actor_shadow::SteadyActorShadow;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related distributed.subscribe-publish]
use crate::distributed::polling;
// ss[related philosophy.structural-hierarchy]
use crate::state_management::SteadyState;
// ss[related philosophy.structural-hierarchy]
use crate::yield_now;

/// Steady state for the single-channel Aeron subscriber, tracking the subscription registration ID.
#[derive(Default)]
// ss[related distributed.subscribe-publish]
pub struct AeronSubscribeSteadyState {
    /// The registration ID of the single subscription, None if not yet registered.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) sub_reg_id: Option<i64>,
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
// ss[related distributed.subscribe-publish]
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
        let result =
            internal_behavior(actor, tx, aeron_connect, stream_id, aeron_media_driver, state).await;
        if let Err(ref e) = result {
            eprintln!("AeronSubscribe actor exited with error: {e}");
        }
        result
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
// ss[related distributed.subscribe-publish]
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
    // ss[depends distributed.subscribe-publish]
    if state.sub_reg_id.is_none() {
        let mut aeron_guard = aeron.lock().await;
        // ss[impl distributed.subscribe-publish]
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
                        // ss[depends distributed.subscribe-publish]
                        if actor.is_liveliness_stop_requested() {
                            if !actor.is_running(&mut || tx.mark_closed()) {
                                warn!("shutdown requested while waiting for subscription (ingress closed)");
                                return Ok(());
                            }
                            return Err(
                                "shutdown during subscription registration (ingress not closed)"
                                    .into(),
                            );
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
    // ss[impl distributed.subscribe-publish]
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
// ss[related distributed.subscribe-publish]
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

#[cfg(test)]
// ss[related distributed.subscribe-publish]
mod aeron_subscribe_state_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::AeronSubscribeSteadyState;
    // ss[related philosophy.structural-hierarchy]
    use crate::distributed::polling::PollScheduler;

    /// Preflight wire probe stream id in integration tests (see `PREFLIGHT_PROBE_STREAM_ID`).
    // ss[related distributed.subscribe-publish]
    const PREFLIGHT_PROBE_STREAM_ID: i32 = 80_000;

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_state_default() {
        let state = AeronSubscribeSteadyState::default();
        assert!(state.sub_reg_id.is_none());
    }

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_fresh_stream_ids_disjoint_from_preflight_probe() {
        for salt in 0..u16::MAX {
            let id = 10_000 + i32::from(salt);
            assert_ne!(
                id, PREFLIGHT_PROBE_STREAM_ID,
                "salt {salt} collides with preflight probe stream id"
            );
        }
    }

    #[test]
    // ss[verify distributed.media-driver-testing]
    fn test_poll_scheduler_delay_within_bounds() {
        let mut scheduler = PollScheduler::new();
        let min = 10_000_000u64;
        let max = 2_000_000_000u64;
        scheduler.set_min_delay_ns(min);
        scheduler.set_max_delay_ns(max);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_expected_moment_ns(5_000_000_000);
        for now_ns in [0, 4_000_000_000, 5_000_000_000, 6_000_000_000, 20_000_000_000] {
            let delay = scheduler.compute_next_delay_ns(now_ns);
            assert!(delay >= min, "delay {delay} below min at now={now_ns}");
            assert!(delay <= max, "delay {delay} above max at now={now_ns}");
        }
    }

}

#[cfg(test)]
// ss[related distributed.subscribe-publish]
mod aeron_subscribe_graph_tests {
    // ss[related philosophy.structural-hierarchy]
    use std::time::Duration;

    // ss[related distributed.subscribe-publish]
    use futures_timer::Delay;

    // ss[related philosophy.structural-hierarchy]
    use crate::distributed::aeron_channel_builder::AeronConfig;
    // ss[related distributed.subscribe-publish]
    use crate::distributed::aeron_channel_structs::MediaType;
    // ss[related philosophy.structural-hierarchy]
    use crate::distributed::aqueduct_builder::AqueductBuilder;
    // ss[related philosophy.structural-hierarchy]
    use crate::distributed::aqueduct_stream::StreamIngress;
    // ss[related distributed.subscribe-publish]
    use crate::{AqueTech, GraphBuilder, SoloAct};

    /// Simulated Aeron subscribe actor: graph starts and stops without a live driver.
    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_simulated_graph_stops_cleanly() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(256);
        let (tx, _rx) = cb.build_stream::<StreamIngress>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        tx.build_aqueduct(
            AqueTech::Aeron(channel, 42),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeSim")
                .never_simulate(false),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(15)));
        Delay::new(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(20)).is_ok());
        });
}

    /// Internal subscribe path: when no media driver is attached, shutdown during driver wait exits.
    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_stops_during_driver_wait_without_driver() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        if graph.aeron_media_driver().is_some() {
            eprintln!("SKIP: media driver present — driver-wait stop test needs isolated graph");
            return;
        }
        let cb = graph.channel_builder().with_capacity(256);
        let (tx, _rx) = cb.build_stream::<StreamIngress>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        tx.build_aqueduct(
            AqueTech::Aeron(channel, 43),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeStop")
                .never_simulate(true),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(10)));
        Delay::new(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(25)).is_ok());
        });
}

    /// Internal subscribe path with closed ingress: shutdown during driver wait exits promptly.
    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_internal_closed_ingress_stops_without_driver() {
    crate::core_exec::block_on(async {

        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(256);
        let (tx, _rx) = cb.build_stream::<StreamIngress>(64);
        tx.testing_close();
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        tx.build_aqueduct(
            AqueTech::Aeron(channel, 51),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeClosedIngress")
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

// Live Aeron pub/sub E2E tests live in `core/tests/aeron_integration_*.rs`.