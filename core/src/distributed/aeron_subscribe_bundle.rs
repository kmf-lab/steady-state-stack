//! Aeron subscription and polling logic for SteadyState actors.
//!
//! This module manages the lifecycle and polling of Aeron subscriptions for SteadyState actors,
//! enabling efficient reception of data from distributed streams in high-throughput, low-latency systems.
//! It handles subscription registration, dynamic polling, and connection management, adapting to
//! varying data rates for optimal performance.
//!
//! ## Key Features
//! - **Subscription Registration**: Connects to Aeron streams for each stream in the bundle.
//! - **Dynamic Polling**: Polls subscriptions and adjusts intervals based on data arrival rates.
//! - **Connection Management**: Ensures subscriptions are established and handles disconnections.
//! - **Scalability**: Supports multiple streams (defined by `GIRTH`) in a single actor.
//!
//! ## Usage
//! Used within the SteadyState actor framework to process incoming Aeron streams. The actor initializes
//! via `run`, which delegates to `internal_behavior` for subscription management and polling, or runs
//! a simulated mode for testing.

// ss[related distributed.subscribe-publish]
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
// ss[related distributed.subscribe-publish]
use aeron::concurrent::atomic_buffer::AtomicBuffer;
use aeron::concurrent::logbuffer::frame_descriptor;
use aeron::concurrent::logbuffer::header::Header;
// ss[related distributed.subscribe-publish]
use aeron::subscription::Subscription;
use aeron::aeron::Aeron;
use futures_util::lock::MutexGuard;
// ss[related distributed.subscribe-publish]
use log::*;
use crate::distributed::aeron_channel_structs::Channel;
use crate::distributed::aqueduct_stream::{SteadyStreamTxBundle, StreamIngress};
// ss[related distributed.subscribe-publish]
use crate::{SteadyActor, SteadyStreamTxBundleTrait, StreamTx, StreamTxBundleTrait};
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::core_tx::TxCore;
// ss[related distributed.subscribe-publish]
use crate::distributed::polling;
use crate::simulate_edge::IntoSimRunner;
use crate::state_management::SteadyState;
// ss[related distributed.subscribe-publish]
use crate::yield_now;

/// State for managing Aeron subscriptions within a SteadyState actor.
///
/// Tracks registration IDs assigned by Aeron for each stream in the subscription bundle.
#[derive(Default)]
// ss[related distributed.subscribe-publish]
pub struct AeronSubscribeSteadyState {
    /// Registration IDs for each Aeron subscription, indexed by stream position in the bundle.
    /// Each entry is `None` until the subscription is registered, then holds the Aeron-assigned ID.
    pub sub_reg_id: Vec<Option<i64>>,
}

/// Default round-robin polling interval for Aeron subscriptions.
///
/// Perf tuning only; correctness does not depend on this value (bootstrap poll handles IPC connect).
// ss[related distributed.subscribe-publish]
const ROUND_ROBIN: Option<Duration> = Some(Duration::from_micros(20));

/// Entry point for running an Aeron subscriber actor.
///
/// Initializes the actor, ensures the Aeron media driver is available, and delegates to either
/// internal subscription/polling logic or a simulated behavior for testing.
///
/// # Arguments
/// - `context`: The actor context, providing framework utilities and identity.
/// - `tx`: Bundle of stream transmitters for forwarding incoming data.
/// - `aeron_connect`: Configuration for connecting to the Aeron media driver.
/// - `stream_id`: Base stream ID; each stream in the bundle offsets from this value.
/// - `state`: Shared state tracking subscription registration IDs across restarts.
///
/// # Returns
/// - `Ok(())` on normal shutdown.
/// - `Err` if initialization fails (e.g., media driver unavailable).
///
/// # Panics
/// Panics if internal invariants (e.g., unwrapping subscriptions) fail, indicating a bug.
// ss[related distributed.subscribe-publish]
pub async fn run<const GIRTH: usize>(
    context: SteadyActorShadow,
    tx: SteadyStreamTxBundle<StreamIngress, GIRTH>,
    aeron_connect: Channel,
    stream_id: i32,
    state: SteadyState<AeronSubscribeSteadyState>,
) -> Result<(), Box<dyn Error>> {
    // Convert context to an active actor with no additional metadata (empty array).
    let mut actor = context.into_spotlight([], tx.payload_meta_data());
    if actor.use_internal_behavior {
        // Wait for the Aeron media driver, retrying every 15 seconds if unavailable.
        while actor.aeron_media_driver().is_none() {
            warn!("unable to find Aeron media driver, will try again in 15 sec");
            let mut tx = tx.lock().await;
            if actor.is_running(&mut || tx.mark_closed()) {
                let _ = actor.wait_periodic(Duration::from_secs(15)).await;
            } else {
                return Ok(()); // Exit if actor is stopped during wait.
            }
        }
        // Delegate to internal behavior for subscription and polling management.
        let result = internal_behavior(actor, tx, aeron_connect, stream_id, state).await;
        if let Err(ref e) = result {
            eprintln!("AeronSubscribeBundle actor exited with error: {e}");
        }
        result
    } else {
        // Run simulated behavior for testing, using transmitter clones.
        let te: Vec<_> = tx.iter().cloned().collect();
        let sims: Vec<_> = te.iter().map(|f| f as &dyn IntoSimRunner<_>).collect();
        actor.simulated_behavior(sims).await
    }
}

/// Polls a single Aeron subscription for new data and processes fragments.
///
/// Reads available fragments from the subscription, updates data rate statistics, and computes
/// the next poll interval based on observed traffic patterns.
///
/// # Arguments
/// - `tx_item`: Stream transmitter for handling incoming fragments.
/// - `sub`: The Aeron subscription to poll.
/// - `actor`: Actor instance for telemetry and shutdown checks.
/// - `now`: Current time, used for timing and statistics.
///
/// # Returns
/// Duration to wait before the next poll, calculated dynamically.
///
/// # Notes
/// - Processes fragments up to the channel’s capacity, respecting defragmentation limits.
/// - Uses a `PollScheduler` to adapt polling intervals based on data arrival rates.
// ss[related distributed.subscribe-publish]
async fn poll_aeron_subscription<C: SteadyActor>(
    tx_item: &mut StreamTx<StreamIngress>,
    sub: &mut Subscription,
    actor: &mut C,
    now: Instant,
) -> Duration {
    let mut count_down = 4; // Limits polling iterations to prevent infinite loops.
    loop {
        let mut input_bytes: u32 = 0;
        let mut input_frags: u32 = 0;

        loop {
            let remaining_poll = tx_item.defrag_has_room_for();
            if remaining_poll == 0 {
                break; // Stop polling if no room for more fragments.
            }

            // Poll subscription and process each fragment.
            let got_count = sub.poll(
                &mut |buffer: &AtomicBuffer, offset: i32, length: i32, header: &Header| {
                    debug_assert!(
                        length <= tx_item.payload_channel.capacity() as i32,
                        "Internal error, slice is too large"
                    );

                    let flags = header.flags();
                    let is_begin = 0 != (flags & frame_descriptor::BEGIN_FRAG);
                    let is_end = 0 != (flags & frame_descriptor::END_FRAG);

                    // Forward fragment to the transmitter for processing.
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
                break; // Exit if no more fragments or limit reached.
            }
            yield_now().await; // Yield to allow other tasks to run.
        }

        // Flush ready messages and update output stats.
        if !tx_item.ready_msg_session.is_empty() {
            let (now_sent_messages, now_sent_bytes) = tx_item.fragment_flush_ready(actor);
            let current_vacant_items = tx_item.control_channel.shared_vacant_units() as i32;
            let current_vacant_bytes = tx_item.payload_channel.shared_vacant_units() as i32;

            let duration = now.duration_since(tx_item.last_output_instant);
            tx_item.store_output_data_rate(duration, now_sent_messages, now_sent_bytes);
            tx_item.last_output_instant = now;
            tx_item.set_stored_vacant_values(current_vacant_items, current_vacant_bytes);
        }

        // Update input stats if fragments were received.
        if input_frags > 0 {
            let duration = now.duration_since(tx_item.last_input_instant);
            tx_item.store_input_data_rate(duration, input_frags, input_bytes);
            tx_item.last_input_instant = now;
        } else {
            break; // Exit if no new fragments.
        }
        count_down -= 1;
        if count_down == 0 {
            break;
        }
    }

    // Calculate next poll interval using observed data rates.
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

/// Manages the lifecycle and polling of Aeron subscriptions for the actor.
///
/// Registers subscriptions, waits for them to connect, and runs a dynamic polling loop
/// to process incoming data from all streams in the bundle.
///
/// # Arguments
/// - `actor`: The actor instance driving the process.
/// - `tx`: Bundle of stream transmitters for incoming data.
/// - `aeron_channel`: Configuration for Aeron subscriptions.
/// - `stream_id`: Base stream ID; each stream offsets from this.
/// - `state`: Shared state tracking subscription IDs.
///
/// # Returns
/// - `Ok(())` on normal shutdown.
/// - `Err` if initialization fails (e.g., media driver issues).
///
/// # Notes
/// - Uses a round-robin or dynamic scheduling approach, configurable via `ROUND_ROBIN`.
/// - Periodically rechecks connection status to handle network issues.
// ss[related distributed.subscribe-publish]
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

    // Ensure state has enough slots for all streams in the bundle.
    while state.sub_reg_id.len() < GIRTH {
        state.sub_reg_id.push(None);
    }
    info!("waiting on media driver");
    let aeron = actor.aeron_media_driver().expect("media driver available");
    info!("register subscriptions A");
    let mut lock: MutexGuard<'_, Aeron> = aeron.lock().await;
    info!("register subscriptions B");

    // Register subscriptions if not already present.
    for f in 0..GIRTH {
        if state.sub_reg_id[f].is_none() {
            let connection_string = aeron_channel.cstring();
            let stream_id = f as i32 + stream_id;

            match lock.add_subscription(connection_string, stream_id) {
                Ok(reg_id) => {
                    // ss[impl distributed.subscribe-publish]
                    trace!("got this id {} for channel idx {}", reg_id, f);
                    state.sub_reg_id[f] = Some(reg_id);
                }
                Err(e) => {
                    // ss[impl distributed.subscribe-publish]
                    if actor.is_liveliness_stop_requested() {
                        let mut tx_guards = tx_bundle.lock().await;
                        if !actor.is_running(&mut || tx_guards.mark_closed()) {
                            return Ok(());
                        }
                        return Err(format!(
                            "shutdown during bundle subscription registration lane={f}"
                        )
                        .into());
                    }
                    return Err(format!(
                        "Failed to add subscription lane={f} stream_id={stream_id}: {e:?}"
                    )
                    .into());
                }
            };
        }
    }
    drop(lock);

    // Wait for all subscriptions to be found and connected.
    actor.wait(Duration::from_millis(13)).await;
    info!("confirm subscriptions");
    let mut tx_guards = tx_bundle.lock().await;
    for f in 0..GIRTH {
        if let Some(id) = state.sub_reg_id[f] {
            let mut found = false;
            while actor.is_running(&mut || tx_guards.mark_closed()) && !found {
                trace!("looking for subscription {}", id);
                let sub = { 
                    let mut lock: MutexGuard<'_, Aeron> = aeron.lock().await;
                    lock.find_subscription(id) 
                };
                match sub {
                    Err(e) => {
                        if actor.is_liveliness_stop_requested() {
                            trace!("stop detected before finding subscription");
                            subs[f] = Err("Shutdown requested while waiting".into());
                            found = true;
                        }
                        if !e.to_string().contains("NotReady") {
                            error!("error {:?} while looking for subscription {}", e, id);
                        }
                        actor.wait(Duration::from_millis(13)).await;
                        actor.relay_stats();
                    }
                    Ok(subscription) => {
                        trace!("found subscription {}", id);
                        match Arc::<std::sync::Mutex<Subscription>>::try_unwrap(subscription) {
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

    info!("running: '{:?}' all subscriptions in place", actor.identity().label);
    let mut assume_connected = [false; GIRTH];

    // Log initial connection status for all subscriptions.
    for i in 0..GIRTH {
        match &subs[i] {
            Ok(subscription) => {
                let ref_images = subscription.images();
                assume_connected[i] = subscription.is_connected();
                trace!(
                    "{:?} connected: {:?} status: {:?} images: {:?}",
                    i, assume_connected[i], subscription.channel_status(), ref_images.len()
                );
            }
            Err(e) => {
                warn!("{:?} {:?}", i, e);
            }
        }
    }

    // Bootstrap: poll all lanes until connected or deadline (IPC needs poll to establish images).
    const BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(10);
    let bootstrap_start = Instant::now();
    while actor.is_running(&mut || tx_guards.mark_closed())
        && bootstrap_start.elapsed() < BOOTSTRAP_DEADLINE
    {
        let mut all_connected = true;
        for i in 0..GIRTH {
            match &mut subs[i] {
                Ok(subscription) => {
                    if !subscription.is_connected() {
                        all_connected = false;
                        let tx_stream = &mut tx_guards[i];
                        let _ =
                            poll_aeron_subscription(tx_stream, subscription, &mut actor, Instant::now())
                                .await;
                        assume_connected[i] = subscription.is_connected();
                    }
                }
                Err(_) => all_connected = false,
            }
        }
        if all_connected {
            break;
        }
        actor.wait(Duration::from_millis(5)).await;
    }

    // Main polling loop: dynamically schedule and poll subscriptions.
    let mut now = Instant::now();
    let mut next_times = [now; GIRTH];
    let mut iteration = 0;

    while actor.is_running(&mut || tx_guards.mark_closed()) {
        // Find the subscription scheduled to poll next.
        let mut earliest_idx = 0;
        let mut earliest_time = next_times[0];
        for i in 1..GIRTH {
            if next_times[i] < earliest_time {
                earliest_time = next_times[i];
                earliest_idx = i;
            }
        }

        // Wait until the scheduled poll time.
        if earliest_time > now {
            let time_to_wait = earliest_time - now;
            let tx_stream = &mut tx_guards[earliest_idx];
            if time_to_wait > tx_stream.max_poll_latency {
                trace!("time to wait exceeds max latency {:?}", time_to_wait);
            }
            if time_to_wait > Duration::from_millis(5) {
                error!("long wait {:?} for subscription {:?}", time_to_wait, earliest_idx);
            }
            actor.wait_periodic(time_to_wait).await;
        }
        now = Instant::now();

        // Poll the subscription and schedule the next poll.
        {
            let tx_stream = &mut tx_guards[earliest_idx];
            // Periodically recheck connection status
            if iteration & ((1 << 16) - 1) == 0 {
                for i in 0..GIRTH {
                    match &mut subs[i] {
                        Ok(subscription) => {
                            let do_periodic_check = iteration & ((1 << 18) - 1) == 0;
                            if !assume_connected[i] || do_periodic_check  {
                                if !assume_connected[i] {
                                    warn!("not connected, rechecking for subscription {}", i);
                                }
                                assume_connected[i] = subscription.is_connected();
                            }
                        }
                        Err(e) => {
                            warn!("{:?} {:?}", i, e);
                        }
                    }
                }
            }

            // Poll even when not yet connected so IPC images can establish when publication offers.
            let dynamic = match &mut subs[earliest_idx] {
                Ok(subscription) => {
                    let delay =
                        poll_aeron_subscription(tx_stream, subscription, &mut actor, now).await;
                    if !assume_connected[earliest_idx] {
                        assume_connected[earliest_idx] = subscription.is_connected();
                    }
                    if assume_connected[earliest_idx] {
                        delay
                    } else {
                        Duration::from_millis(20)
                    }
                }
                Err(e) => {
                    error!("Internal error, subscription missing: {:?}", e);
                    Duration::from_secs(i32::MAX as u64) // Effectively halt polling.
                }
            };

            // Schedule the next poll using either fixed or dynamic interval.
            next_times[earliest_idx] = now + if let Some(fixed) = ROUND_ROBIN { fixed } else { dynamic };
        }
        iteration += 1;
    }
    info!("Subscribe shutting down.");
    Ok(())
}

#[cfg(test)]
// ss[related distributed.subscribe-publish]
mod aeron_subscribe_bundle_tests {
    use super::*;

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_state_init() {
        let state = AeronSubscribeSteadyState::default();
        assert!(state.sub_reg_id.is_empty());
    }

    #[test]
    // ss[verify distributed.subscribe-publish]
    fn test_subscribe_state_resize() {
        let mut state = AeronSubscribeSteadyState::default();
        let girth = 5;
        while state.sub_reg_id.len() < girth {
            state.sub_reg_id.push(None);
        }
        assert_eq!(state.sub_reg_id.len(), 5);
    }
}

#[cfg(test)]
mod aeron_subscribe_bundle_graph_tests {
    use std::time::Duration;

    use async_std::task;

    use crate::distributed::aeron_channel_builder::AeronConfig;
    use crate::distributed::aeron_channel_structs::MediaType;
    use crate::distributed::aqueduct_builder::AqueductBuilder;
    use crate::distributed::aqueduct_stream::StreamIngress;
    use crate::{AqueTech, GraphBuilder, SoloAct};

    /// Simulated Aeron subscribe bundle: graph starts and stops without a live driver.
    #[async_std::test]
    // ss[verify distributed.subscribe-publish]
    async fn test_subscribe_bundle_simulated_graph_stops_cleanly() {
        const GIRTH: usize = 1;
        let mut graph = GraphBuilder::for_testing().build(());
        let cb = graph.channel_builder().with_capacity(256);
        let (lazy_tx, _rx) = cb.build_stream_bundle::<StreamIngress, GIRTH>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        lazy_tx.build_aqueduct(
            AqueTech::Aeron(channel, 48),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeBundleSim")
                .never_simulate(false),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(15)));
        task::sleep(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(20)).is_ok());
    }

    /// Internal subscribe bundle path: shutdown during driver wait when no media driver is attached.
    #[async_std::test]
    // ss[verify distributed.subscribe-publish]
    async fn test_subscribe_bundle_stops_during_driver_wait_without_driver() {
        let mut graph = GraphBuilder::for_testing().build(());
        if graph.aeron_media_driver().is_some() {
            eprintln!("SKIP: media driver present — driver-wait stop test needs isolated graph");
            return;
        }
        const GIRTH: usize = 1;
        let cb = graph.channel_builder().with_capacity(256);
        let (lazy_tx, _rx) = cb.build_stream_bundle::<StreamIngress, GIRTH>(64);
        let channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();
        lazy_tx.build_aqueduct(
            AqueTech::Aeron(channel, 49),
            &graph
                .actor_builder()
                .with_name("AeronSubscribeBundleStop")
                .never_simulate(true),
            SoloAct,
        );
        assert!(graph.start_with_timeout(Duration::from_secs(10)));
        task::sleep(Duration::from_millis(100)).await;
        graph.request_shutdown();
        assert!(graph.block_until_stopped(Duration::from_secs(25)).is_ok());
    }
}
