//! Aeron subscription and polling logic for SteadyState actors.
//!
//! This module manages the lifecycle and polling of Aeron subscriptions for
//! SteadyState actors, including dynamic scheduling, connection management,
//! and efficient polling of incoming data streams. It is designed for use
//! in distributed, high-throughput, low-latency streaming systems.

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
use aeron::subscription::Subscription;
use log::{error, trace, warn};
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::distributed_stream::{SteadyStreamTxBundle, StreamIngress};
use crate::{SteadyActor, SteadyState, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait};
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::core_tx::TxCore;
use crate::distributed::polling;
use crate::simulate_edge::IntoSimRunner;
use crate::yield_now;

/// State for Aeron subscription management, tracking registration IDs for each stream.
#[derive(Default)]
pub struct AeronSubscribeSteadyState {
    /// Registration IDs for each Aeron subscription, indexed by stream.
    pub sub_reg_id: Vec<Option<i64>>,
}

/// The default round-robin polling interval for Aeron subscriptions.
/// This is a temporary hack for testing and may be tuned for production.
const ROUND_ROBIN: Option<Duration> = Some(Duration::from_micros(50));

/// Main entry point for running an Aeron subscriber actor.
///
/// This function initializes the actor, ensures the Aeron media driver is available,
/// and then enters either internal or simulated behavior depending on configuration.
/// It manages the lifecycle of subscriptions and polling for incoming data.
///
/// # Arguments
/// - `context`: The actor context.
/// - `tx`: The bundle of stream transmitters for incoming data.
/// - `aeron_connect`: The Aeron channel configuration.
/// - `stream_id`: The base stream ID for subscriptions.
/// - `state`: Shared state for subscription management.
///
/// # Returns
/// Returns `Ok(())` on normal shutdown, or an error if initialization fails.
pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    tx: SteadyStreamTxBundle<StreamIngress, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    // Convert the actor context into a spotlight (active actor) and initialize metadata.
    let mut actor = context.into_spotlight([], tx.payload_meta_data());
    if actor.use_internal_behavior {
        // Wait for Aeron media driver to become available.
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if actor.is_running(&mut || tx.mark_closed()) {
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(());
            }
        }
        // Enter the main internal polling loop.
        internal_behavior(actor, tx, aeron_connect, stream_id, state).await
    } else {
        // If not using internal behavior, run in simulation mode.
        let te: Vec<_> = tx.iter().cloned().collect();
        let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
        actor.simulated_behavior(sims).await
    }
}

/// Polls a single Aeron subscription for new data and processes incoming fragments.
///
/// This function reads from the Aeron subscription, processes available fragments,
/// updates data rate statistics, and schedules the next poll based on observed traffic.
///
/// # Arguments
/// - `tx_item`: The stream transmitter for the current subscription.
/// - `sub`: The Aeron subscription to poll.
/// - `actor`: The actor instance for telemetry and shutdown checks.
/// - `now`: The current time instant.
///
/// # Returns
/// Returns the recommended duration to wait before the next poll.
async fn poll_aeron_subscription<C: SteadyActor>(
    tx_item: &mut StreamTx<StreamIngress>,
    sub: &mut Subscription,
    actor: &mut C,
    now: Instant,
) -> Duration {
    // Poll the subscription for available fragments, up to the channel's capacity.
    let mut count_down = 1;
    loop {
        let mut input_bytes: u32 = 0;
        let mut input_frags: u32 = 0;

        loop {
            let remaining_poll = tx_item.defrag_has_room_for();
            if remaining_poll == 0 {
                break;
            }

            let got_count = sub.poll(
                &mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
                    debug_assert!(
                        length <= tx_item.payload_channel.capacity() as i32,
                        "Internal error, slice is too large"
                    );

                    let flags = header.flags();
                    let is_begin = 0 != (flags & frame_descriptor::BEGIN_FRAG);
                    let is_end = 0 != (flags & frame_descriptor::END_FRAG);

                    tx_item.fragment_consume(
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
            );

            if got_count <= 0 || got_count == remaining_poll as i32 {
                break;
            }
            yield_now().await;
        }

        // Flush any ready messages and update output statistics.
        if !tx_item.ready_msg_session.is_empty() {
            let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(actor);

            let current_vacant_items = tx_item.control_channel.shared_vacant_units() as i32;
            let current_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as i32;

            let duration = now.duration_since(tx_item.last_output_instant);
            tx_item.store_output_data_rate(duration, now_sent_messages, now_sent_bytes);
            tx_item.last_output_instant = now;

            tx_item.set_stored_vacant_values(current_vacant_items, current_vacant_bytes);
        }
        // Update input data rate statistics if fragments were received.
        if input_frags > 0 {
            let duration = now.duration_since(tx_item.last_input_instant);
            tx_item.store_input_data_rate(duration, input_frags, input_bytes);
            tx_item.last_input_instant = now;
        } else {
            break;
        }
        count_down -= 1;
        if 0 == count_down {
            break;
        }
    }

    // Compute the next poll interval based on observed traffic and scheduling policy.
    let (avg, std) = tx_item.guess_duration_between_arrivals();
    let (min, max) = tx_item.next_poll_bounds();

    let mut scheduler = polling::PollScheduler::new();
    scheduler.set_max_delay_ns(max.as_nanos() as u64);
    scheduler.set_min_delay_ns(min.as_nanos() as u64);
    scheduler.set_std_dev_ns(std.as_nanos() as u64);
    scheduler.set_expected_moment_ns(avg.as_nanos() as u64);
    let waited_ns = now.duration_since(tx_item.last_input_instant).as_nanos() as u64;
    let ns = scheduler.compute_next_delay_ns(waited_ns);
    trace!("end of poll method {} ", ns);
    Duration::from_nanos(ns)
}

/// Main internal polling loop for Aeron subscriptions.
///
/// This function manages the lifecycle of all Aeron subscriptions for the actor,
/// including registration, connection, polling, and dynamic scheduling. It ensures
/// that all subscriptions are established, polls each for new data, and adapts
/// polling intervals based on observed traffic patterns.
///
/// # Arguments
/// - `actor`: The actor instance.
/// - `tx`: The bundle of stream transmitters for incoming data.
/// - `aeron_channel`: The Aeron channel configuration.
/// - `stream_id`: The base stream ID for subscriptions.
/// - `state`: Shared state for subscription management.
///
/// # Returns
/// Returns `Ok(())` on normal shutdown, or an error if initialization fails.
async fn internal_behavior<const GIRTH: usize, C: SteadyActor>(
    mut actor: C,
    tx: SteadyStreamTxBundle<StreamIngress, GIRTH>,
    aeron_channel: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    let tx_bundle = tx;
    let mut state = state.lock(AeronSubscribeSteadyState::default).await;
    let mut subs: [Result<Subscription, Box<dyn Error>>; GIRTH] = std::array::from_fn(|_| Err("Not Found".into()));

    // Ensure state has a registration ID for each stream.
    while state.sub_reg_id.len() < GIRTH {
        state.sub_reg_id.push(None);
    }

    let aeron = actor.aeron_media_driver().expect("media driver");

    // Register subscriptions for each stream if not already present.
    for f in 0..GIRTH {
        if state.sub_reg_id[f].is_none() {
            let connection_string = aeron_channel.cstring();
            let stream_id = f as i32 + stream_id;

            match aeron.lock().await.add_subscription(connection_string, stream_id) {
                Ok(reg_id) => {
                    trace!("got this id {} for channel idx {}", reg_id, f);
                    state.sub_reg_id[f] = Some(reg_id);
                }
                Err(e) => {
                    warn!("Unable to register subscription: {:?}", e);
                }
            };
        }
    }

    // Wait for all subscriptions to become available and connected.
    let mut tx_guards = tx_bundle.lock().await;
    for f in 0..GIRTH {
        if let Some(id) = state.sub_reg_id[f] {
            let mut found = false;
            while actor.is_running(&mut || tx_guards.mark_closed()) && !found {
                error!("looking for subscription {}", id);
                let sub = { aeron.lock().await.find_subscription(id) };
                match sub {
                    Err(e) => {
                        if actor.is_liveliness_stop_requested() {
                            trace!("stop detected before finding publication");
                            subs[f] = Err("Shutdown requested while waiting".into());
                            found = true;
                        }
                        error!("error {:?} while looking for subscription {}", e, id);
                        actor.wait(Duration::from_millis(13)).await;
                        actor.relay_stats();
                    }
                    Ok(subscription) => {
                        error!("found subscription {}", id);
                        match Arc::try_unwrap(subscription) {
                            Ok(mutex) => match mutex.into_inner() {
                                Ok(subscription) => {
                                    subs[f] = Ok(subscription);
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

    error!("running subscriber '{:?}' all subscriptions in place", actor.identity());
    let mut assume_connected = [false; GIRTH];

    // Log connection status for all subscriptions.
    for i in 0..GIRTH {
        match &subs[i] {
            Ok(subscription) => {
                let ref_images = subscription.images();
                assume_connected[i] = subscription.is_connected();
                warn!(
                    "{:?} connected: {:?} statuss: {:?} images: {:?}",
                    i, assume_connected[i], subscription.channel_status(), ref_images.len()
                );
            }
            Err(e) => {
                warn!("{:?} {:?}", i, e);
            }
        }
    }

    // Main polling loop: schedule and poll each subscription as needed.
    let mut now = Instant::now();
    let mut next_times = [now; GIRTH];
    let mut iteration = 0;

    while actor.is_running(&mut || tx_guards.mark_closed()) {
        // Find the subscription with the earliest scheduled poll time.
        let mut earliest_idx = 0;
        let mut earliest_time = next_times[0];
        for i in 1..GIRTH {
            if next_times[i] < earliest_time {
                earliest_time = next_times[i];
                earliest_idx = i;
            }
        }
        // Wait until the next scheduled poll time.
        if earliest_time > now {
            let time_to_wait = earliest_time - now;
            let tx_stream = &mut tx_guards[earliest_idx];
            if time_to_wait > tx_stream.max_poll_latency {
                trace!("time to wait is outside of the expected max {:?}", time_to_wait);
            }
            if time_to_wait > Duration::from_millis(5) {
                error!("big wait {:?} for {:?}", time_to_wait, earliest_idx);
            }
            actor.wait_periodic(time_to_wait).await;
        }
        now = Instant::now();
        {
            let tx_stream = &mut tx_guards[earliest_idx];
            if 0 == iteration & ((1 << 14) - 1) {
                for i in 0..GIRTH {
                    match &mut subs[i] {
                        Ok(subscription) => {
                            if !assume_connected[earliest_idx] || 0 == iteration & ((1 << 15) - 1) {
                                warn!("expensive rechecking connection for {}", i);
                                assume_connected[i] = subscription.is_connected();
                            }
                        }
                        Err(e) => {
                            warn!("{:?} {:?}", i, e);
                        }
                    }
                }
            }

            // Poll the selected subscription for new data, or wait if not connected.
            let dynamic = match &mut subs[earliest_idx] {
                Ok(subscription) => {
                    if assume_connected[earliest_idx] {
                        poll_aeron_subscription(tx_stream, subscription, &mut actor, now).await
                    } else {
                        Duration::from_millis(20)
                    }
                }
                Err(e) => {
                    error!("Internal error, the subscription should be present: {:?}", e);
                    Duration::from_secs(i32::MAX as u64)
                }
            };

            // Schedule the next poll for this subscription.
            next_times[earliest_idx] = now + if let Some(fixed) = ROUND_ROBIN { fixed } else { dynamic };
        }
        iteration += 1;
    }
    Ok(())
}