use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Sub};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use async_ringbuf::traits::Observer;
use log::*;
use num_traits::Zero;
use crate::{abstract_executor, ActorIdentity, steady_config, Graph, GraphLivelinessState, MONITOR_NOT, MONITOR_UNKNOWN, SendSaturation, steady_tx_bundle, SteadyContext, telemetry};
use crate::channel_builder::ChannelBuilder;
use crate::steady_config::*;
use crate::monitor::{ChannelMetaData, find_my_index, LocalMonitor, RxTel};
use crate::steady_telemetry::{SteadyTelemetryActorSend, SteadyTelemetryRx, SteadyTelemetrySend, SteadyTelemetryTake};
use crate::telemetry::{metrics_collector, metrics_server};
use crate::telemetry::metrics_collector::CollectorDetail;

/// Constructs telemetry channels for the given context and metadata.
///
/// # Parameters
/// - `that`: The steady context.
/// - `rx_meta_data`: Metadata for RX channels.
/// - `rx_inverse_local_idx`: Inverse local index for RX.
/// - `tx_meta_data`: Metadata for TX channels.
/// - `tx_inverse_local_idx`: Inverse local index for TX.
///
/// # Returns
/// A tuple containing optional TX send, RX send, and actor send telemetry.
pub(crate) fn construct_telemetry_channels<const RX_LEN: usize, const TX_LEN: usize>(
    that: &SteadyContext,
    rx_meta_data: Vec<Arc<ChannelMetaData>>,
    rx_inverse_local_idx: [usize; RX_LEN],
    tx_meta_data: Vec<Arc<ChannelMetaData>>,
    tx_inverse_local_idx: [usize; TX_LEN],
) -> (
    Option<SteadyTelemetrySend<RX_LEN>>,
    Option<SteadyTelemetrySend<TX_LEN>>,
    Option<SteadyTelemetryActorSend>,
) {
    let start_now = Instant::now().sub(Duration::from_secs(1 + MAX_TELEMETRY_ERROR_RATE_SECONDS as u64));

    let channel_builder = ChannelBuilder::new(
        that.channel_count.clone(),
        that.oneshot_shutdown_vec.clone(),
        start_now,
        that.frame_rate_ms,
    )
        .with_labels(&["steady_state-telemetry"], false)
        .with_compute_refresh_window_bucket_bits(0, 0)
        .with_capacity(steady_config::REAL_CHANNEL_LENGTH_TO_COLLECTOR);

    let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>) =
        if 0usize == RX_LEN {
            (None, None)
        } else {
            let (telemetry_send_rx, telemetry_take_rx) = channel_builder.build();
            (
                Some(SteadyTelemetrySend::new(telemetry_send_rx, [0; RX_LEN], rx_inverse_local_idx, start_now)),
                Some(SteadyTelemetryTake { rx: telemetry_take_rx, details: rx_meta_data }),
            )
        };

    let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>) =
        if 0usize == TX_LEN {
            (None, None)
        } else {
            let (telemetry_send_tx, telemetry_take_tx) = channel_builder.build();
            (
                Some(SteadyTelemetrySend::new(telemetry_send_tx, [0; TX_LEN], tx_inverse_local_idx, start_now)),
                Some(SteadyTelemetryTake { rx: telemetry_take_tx, details: tx_meta_data }),
            )
        };

    let act_tuple = channel_builder.build();
    let det = SteadyTelemetryRx {
        send: tx_tuple.1,
        take: rx_tuple.1,
        actor: Some(act_tuple.1),
        actor_metadata: that.actor_metadata.clone(),
    };

    let idx: Option<usize> = match that.all_telemetry_rx.read() {
        Ok(guard) => {
            let shared_vec = guard.deref();
            shared_vec.iter().enumerate().find(|(_, x)| x.ident == that.ident).map(|(idx, _)| idx)
        }
        Err(_) => {
            error!("internal error: unable to get read lock");
            None
        }
    };

    match that.all_telemetry_rx.write() {
        Ok(mut guard) => {
            let shared_vec = guard.deref_mut();
            if let Some(idx) = idx {
                shared_vec[idx].telemetry_take.push_back(Box::new(det));
            } else {
                let mut telemetry_take: VecDeque<Box<dyn RxTel>> = VecDeque::new();
                telemetry_take.push_back(Box::new(det));
                shared_vec.push(CollectorDetail {
                    ident: that.ident,
                    telemetry_take,
                });
            }
        }
        Err(_) => {
            error!("internal error: failed to write to all_telemetry_rx");
        }
    }

    let calls: [AtomicU16; 6] = Default::default();
    let telemetry_actor = Some(SteadyTelemetryActorSend {
        tx: act_tuple.0,
        last_telemetry_error: start_now,
        instant_start: Instant::now(),
        hot_profile_await_ns_unit: AtomicU64::new(0),
        hot_profile: AtomicU64::new(0),
        hot_profile_concurrent: Default::default(),
        calls,
        instance_id: that.instance_id,
        bool_stop: false,
    });

    (rx_tuple.0, tx_tuple.0, telemetry_actor)
}

/// Builds optional telemetry graph for the given graph.
///
/// # Parameters
/// - `graph`: The graph to build the telemetry for.
pub(crate) fn build_optional_telemetry_graph(graph: &mut Graph) {
    #[cfg(any(
        feature = "telemetry_server_builtin",
        feature = "telemetry_server_cdn",
        feature = "prometheus_metrics"
    ))]
    {
        let base = graph.channel_builder().with_compute_refresh_window_bucket_bits(0, 0);

        #[cfg(feature = "telemetry_on_telemetry")]
            let base = base
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_type();

        let (tx, rx) = base
            .with_labels(&["steady_state-telemetry"], true)
            .with_capacity(steady_config::REAL_CHANNEL_LENGTH_TO_FEATURE)
            .build();

        let outgoing = [tx; 1];
        let optional_servers = steady_tx_bundle(outgoing);

        let bldr = graph.actor_builder();

        #[cfg(feature = "telemetry_on_telemetry")]
            let bldr = bldr
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_avg_mcpu()
            .with_avg_work();

        bldr.with_name(metrics_server::NAME).build_spawn(move |context| {
            telemetry::metrics_server::run(context, rx.clone())
        });

        let all_tel_rx = graph.all_telemetry_rx.clone();

        let bldr = graph.actor_builder();

        #[cfg(feature = "telemetry_on_telemetry")]
            let bldr = bldr
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_avg_mcpu()
            .with_avg_work();

        bldr.with_name(metrics_collector::NAME).build_spawn(move |context| {
            let all_rx = all_tel_rx.clone();
            metrics_collector::run(context, all_rx, optional_servers.clone())
        });
    }
}

/// Tries to send all local telemetry for the given monitor.
///
/// # Parameters
/// - `this`: The local monitor to send telemetry for.
#[inline]
pub(crate) fn try_send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(
    this: &mut LocalMonitor<RX_LEN, TX_LEN>,
) {
    let last_elapsed = this.last_telemetry_send.elapsed();
    if last_elapsed.as_micros() * (CONSUMED_MESSAGES_BY_COLLECTOR as u128) >= (1000 * this.frame_rate_ms) as u128 {
        if this.is_in_graph() {
            if let Some(ref mut actor_status) = this.telemetry.state {
                let clear_status = {
                    if let Some(ref mut lock_guard) = actor_status.tx.try_lock() {
                        let tx = lock_guard.deref_mut();
                        let rx_version = tx.rx_version.load(Ordering::SeqCst);
                        if ChannelBuilder::UNSET == rx_version {
                            let now = Instant::now();
                            let dif = now.duration_since(actor_status.last_telemetry_error);
                            if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                actor_status.last_telemetry_error = now;
                            }
                            return;
                        }

                        let capacity = tx.capacity();
                        let vacant_units = tx.vacant_units();
                        if vacant_units >= (capacity >> 1) {
                        } else {
                            let scale = calculate_exponential_channel_backoff(capacity, vacant_units);
                            if last_elapsed.as_micros() < scale as u128 * this.frame_rate_ms as u128 {
                                if scale >= 128 {
                                    if let Ok(guard) = this.runtime_state.read() {
                                        let state = guard.deref();
                                        if !state.is_in_state(&[
                                            GraphLivelinessState::StopRequested,
                                            GraphLivelinessState::Stopped,
                                            GraphLivelinessState::StoppedUncleanly,
                                        ]) {
                                            error!(
                                                "{:?} EXIT hard delay on actor status: scale {} empty {} of {}",
                                                this.ident, scale, vacant_units, capacity
                                            );
                                            error!("assume metrics_collector has died and is not consuming messages");
                                            std::process::exit(-1);
                                        }
                                    }
                                }
                                return;
                            }
                        }

                        let msg = actor_status.status_message();
                        match tx.shared_try_send(msg) {
                            Ok(_) => {
                                if let Some(ref mut send_tx) = this.telemetry.send_tx {
                                    if tx.local_index.lt(&MONITOR_NOT) {
                                        send_tx.count[tx.local_index] += 1;
                                        if last_elapsed.as_millis() >= this.frame_rate_ms as u128 {
                                            let now = Instant::now();
                                            let dif = now.duration_since(actor_status.last_telemetry_error);
                                            if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                                warn!("{:?} consider shortening period of relay_stats_periodic or adding relay_stats_all() in your work loop,\n it is called too infrequently at {}ms which is larger than your frame rate of {}ms",
                                                    this.ident, last_elapsed.as_millis(), this.frame_rate_ms);
                                                actor_status.last_telemetry_error = now;
                                            }
                                        }
                                    } else if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                        tx.local_index = find_my_index(send_tx, tx.channel_meta_data.id);
                                        if tx.local_index.lt(&MONITOR_NOT) {
                                            send_tx.count[tx.local_index] += 1;
                                        }
                                    }
                                }
                                true
                            }
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(actor_status.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    if let Ok(guard) = this.runtime_state.read() {
                                        let state = guard.deref();
                                        if !state.is_in_state(&[
                                            GraphLivelinessState::StopRequested,
                                            GraphLivelinessState::Stopped,
                                            GraphLivelinessState::StoppedUncleanly,
                                        ]) {
                                            warn!("full telemetry state channel detected from {:?} value:{:?} full:{:?} capacity: {:?}",
                                                this.ident, a, tx.is_full(), tx.tx.capacity());
                                            actor_status.last_telemetry_error = now;
                                        }
                                    }
                                }
                                false
                            }
                        }
                    } else {
                        warn!("unable to get lock");
                        false
                    }
                };
                if clear_status {
                    actor_status.status_reset();
                }
            }
            if let Some(ref mut send_tx) = this.telemetry.send_tx {
                if let Some(ref mut lock_guard) = send_tx.tx.try_lock() {
                    if send_tx.count.iter().any(|x| !x.is_zero()) {
                        let tx = lock_guard.deref_mut();
                        match tx.shared_try_send(send_tx.count) {
                            Ok(_) => {
                                send_tx.count.fill(0);
                                if tx.local_index.lt(&MONITOR_NOT) {
                                    send_tx.count[tx.local_index] = 1;
                                } else if tx.local_index.eq(&MONITOR_UNKNOWN) {
                                    tx.local_index = find_my_index(send_tx, tx.channel_meta_data.id);
                                    if tx.local_index.lt(&MONITOR_NOT) {
                                        send_tx.count[tx.local_index] = 1;
                                    }
                                }
                            }
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(send_tx.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    warn!("full telemetry tx channel detected from {:?} value:{:?} full:{:?} capacity: {:?}",
                                        this.ident, a, tx.is_full(), tx.tx.capacity());
                                    send_tx.last_telemetry_error = now;
                                }
                            }
                        }
                    }
                } else {
                    warn!("unable to get lock");
                }
            }
            if let Some(ref mut send_rx) = this.telemetry.send_rx {
                if let Some(ref mut lock_guard) = send_rx.tx.try_lock() {
                    if send_rx.count.iter().any(|x| !x.is_zero()) {
                        let rx = lock_guard.deref_mut();
                        match rx.shared_try_send(send_rx.count) {
                            Ok(_) => {
                                send_rx.count.fill(0);
                                if rx.local_index.lt(&MONITOR_NOT) {
                                    send_rx.count[rx.local_index] = 1;
                                } else if rx.local_index.eq(&MONITOR_UNKNOWN) {
                                    rx.local_index = find_my_index(send_rx, rx.channel_meta_data.id);
                                    if rx.local_index.lt(&MONITOR_NOT) {
                                        send_rx.count[rx.local_index] = 1;
                                    }
                                }
                            }
                            Err(a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(send_rx.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    warn!("full telemetry rx channel detected from {:?} value:{:?} full:{:?} capacity: {:?}",
                                        this.ident, a, rx.is_full(), rx.tx.capacity());
                                    send_rx.last_telemetry_error = now;
                                }
                            }
                        }
                    }
                } else {
                    warn!("unable to get lock");
                }
            }
        } else {
            if let Some(ref mut actor_status) = this.telemetry.state {
                actor_status.status_reset();
            }
            if let Some(ref mut send_rx) = this.telemetry.send_rx {
                send_rx.count.fill(0);
            }
            if let Some(ref mut send_tx) = this.telemetry.send_tx {
                send_tx.count.fill(0);
            }
        }
        this.last_telemetry_send = Instant::now();
    }
}

/// Calculates exponential backoff for telemetry channels.
///
/// # Parameters
/// - `capacity`: The capacity of the channel.
/// - `vacant_units`: The number of vacant units in the channel.
///
/// # Returns
/// The calculated backoff value.
pub(crate) fn calculate_exponential_channel_backoff(capacity: usize, vacant_units: usize) -> u32 {
    let bits_count = (capacity as f64).log2().ceil() as u32;
    let bit_to_represent_vacant_count = 32 - (vacant_units as u32).leading_zeros();
    (1 + bits_count - bit_to_represent_vacant_count).pow(3)
}

/// Sends all local telemetry asynchronously.
///
/// # Parameters
/// - `ident`: The actor identity.
/// - `telemetry_state`: The telemetry state.
/// - `telemetry_send_tx`: The TX send telemetry.
/// - `telemetry_send_rx`: The RX send telemetry.
pub(crate) fn send_all_local_telemetry_async<const RX_LEN: usize, const TX_LEN: usize>(
    ident: ActorIdentity,
    telemetry_state: Option<SteadyTelemetryActorSend>,
    telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
) {
    abstract_executor::block_on(async move {
        if let Some(actor_status) = telemetry_state {
            let mut status = actor_status.status_message();
            if let Some(mut tx) = actor_status.tx.try_lock() {
                let needs_to_be_closed = tx.make_closed.is_some();
                if needs_to_be_closed {
                    status.bool_stop = true;
                    let _ = tx.shared_send_async(status, ident, SendSaturation::IgnoreInRelease).await;
                    tx.mark_closed();
                    tx.wait_empty().await;
                }
            } else {
                error!("unable to get lock");
            }
        }

        if let Some(ref send_tx) = telemetry_send_tx {
            let mut tx = send_tx.tx.lock().await;
            if tx.make_closed.is_none() {
                if send_tx.count.iter().any(|x| !x.is_zero()) {
                    let _ = tx.shared_send_async(send_tx.count, ident, SendSaturation::IgnoreInRelease).await;
                }
                tx.mark_closed();
                tx.wait_empty().await;
            }
        }

        if let Some(ref send_rx) = telemetry_send_rx {
            let mut rx = send_rx.tx.lock().await;
            if rx.make_closed.is_none() {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                    let _ = rx.shared_send_async(send_rx.count, ident, SendSaturation::IgnoreInRelease).await;
                }
                rx.mark_closed();
                rx.wait_empty().await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_scale_up_delay() {
        let capacity = 128;

        for vacant_units in (0..capacity).rev() {
            let backoff = calculate_exponential_channel_backoff(capacity, vacant_units);
            match vacant_units {
                64..=127 => assert_eq!(backoff, 1),
                32..=63 => assert_eq!(backoff, 8),
                16..=31 => assert_eq!(backoff, 27),
                8..=15 => assert_eq!(backoff, 64),
                4..=7 => assert_eq!(backoff, 125),
                2..=3 => assert_eq!(backoff, 216),
                1 => assert_eq!(backoff, 343),
                0 => assert_eq!(backoff, 512),
                _ => {}
            }
        }
    }
}
