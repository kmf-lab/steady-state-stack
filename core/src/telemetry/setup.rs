
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut, Sub};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, AtomicU64};
use std::time::{Duration, Instant};
use async_ringbuf::traits::Observer;
#[allow(unused_imports)]
use log::*;
use num_traits::Zero;
use crate::{steady_tx_bundle, ActorIdentity, Graph, GraphLivelinessState, ScheduleAs, SendSaturation, MONITOR_NOT, MONITOR_UNKNOWN};
use crate::channel_builder::ChannelBuilder;
use crate::steady_config::*;
use crate::monitor::{find_my_index, ChannelMetaData, RxTel};
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetryRx, SteadyTelemetrySend, SteadyTelemetryTake};
use crate::telemetry::{metrics_collector, metrics_server};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::core_exec;
use crate::steady_actor_shadow::SteadyActorShadow;
use crate::steady_actor_spotlight::SteadyActorSpotlight;
use crate::core_tx::TxCore;

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
    that: &SteadyActorShadow,
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
        that.frame_rate_ms,
    )
        .with_labels(&["steady_state-telemetry"], false)
        .with_no_refresh_window()
        .with_capacity(REAL_CHANNEL_LENGTH_TO_COLLECTOR);

    let rx_tuple: (Option<SteadyTelemetrySend<RX_LEN>>, Option<SteadyTelemetryTake<RX_LEN>>) =
        if 0usize == RX_LEN {
            (None, None)
        } else {
            let (telemetry_send_rx, telemetry_take_rx) = channel_builder.build_channel();
            (
                //TODO: perhaps we should have LazySend...
                Some(SteadyTelemetrySend::new(telemetry_send_rx.clone(), [0; RX_LEN], rx_inverse_local_idx, start_now)),
                Some(SteadyTelemetryTake { rx: telemetry_take_rx.clone(), details: rx_meta_data }),
            )
        };

    let tx_tuple: (Option<SteadyTelemetrySend<TX_LEN>>, Option<SteadyTelemetryTake<TX_LEN>>) =
        if 0usize == TX_LEN {
            (None, None)
        } else {
            let (telemetry_send_tx, telemetry_take_tx) = channel_builder.build_channel();
            (  
                //TODO: may need LazySend..
                Some(SteadyTelemetrySend::new(telemetry_send_tx.clone(), [0; TX_LEN], tx_inverse_local_idx, start_now)),
                Some(SteadyTelemetryTake { rx: telemetry_take_tx.clone(), details: tx_meta_data }),
            )
        };

    let act_tuple = channel_builder.build_channel();
    let det = SteadyTelemetryRx {
        send: tx_tuple.1,
        take: rx_tuple.1,
        actor: Some(act_tuple.1.clone()), //TODO: may need LazySend...
        actor_metadata: that.actor_metadata.clone(),
    };

    let idx: Option<usize> = {
            let guard = that.all_telemetry_rx.read();
            let shared_vec = guard.deref();
            shared_vec.iter().enumerate().find(|(_, x)| x.ident == that.ident).map(|(idx, _)| idx)
        };

    {
        let mut guard = that.all_telemetry_rx.write();
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
    

    let calls: [AtomicU16; 6] = Default::default();
    let telemetry_actor = Some(SteadyTelemetryActorSend {
        tx: act_tuple.0.clone(), //TODO: may need LazySend...
        last_telemetry_error: start_now,
        instant_start: Instant::now(),
        iteration_index_start: 0,
        hot_profile_await_ns_unit: AtomicU64::new(0),
        hot_profile: AtomicU64::new(0),
        hot_profile_concurrent: Default::default(),
        calls,
        regeneration: that.regeneration,
        bool_stop: false,
        bool_blocking: false, //TODO: thread this in from with_blocking?
    });

    (rx_tuple.0, tx_tuple.0, telemetry_actor)
}

/// Builds optional telemetry graph for the given graph.
///
/// # Parameters
/// - `graph`: The graph to build the telemetry for.
pub(crate) fn build_telemetry_metric_features(graph: &mut Graph) {
    #[cfg(any(
        feature = "telemetry_server_builtin",
        feature = "telemetry_server_cdn",
        feature = "prometheus_metrics"
    ))]
    {

        let base = graph.channel_builder().with_no_refresh_window();

        #[cfg(feature = "telemetry_on_telemetry")]
            let base = base
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_type();

        let (tx, rx) = base
            .with_labels(&["steady_state-telemetry"], true)
            .with_capacity(REAL_CHANNEL_LENGTH_TO_FEATURE)
            .build_channel();

        let outgoing = [tx.clone()]; 
        let optional_servers = steady_tx_bundle(outgoing);

        ////////////////////////////////////
        //create collector first to get the lower id for flow debug later.
        ////////////////////////////////////
        let all_tel_rx = graph.all_telemetry_rx.clone();
        let bldr = graph.actor_builder();
        #[cfg(feature = "telemetry_on_telemetry")]
        let bldr = bldr
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_mcpu_avg()
            .with_load_avg();
        bldr.with_name(metrics_collector::NAME).build(move |context| {
            let all_rx = all_tel_rx.clone();
            metrics_collector::run(context, all_rx, optional_servers.clone())
        }, ScheduleAs::SoloAct);
        
        
        ////////////////////////////////////
        let bldr = graph.actor_builder();
        #[cfg(feature = "telemetry_on_telemetry")]
            let bldr = bldr
            .with_compute_refresh_window_floor(Duration::from_secs(1), Duration::from_secs(10))
            .with_mcpu_avg()
            .with_load_avg();
        bldr.with_name(metrics_server::NAME).build(move |context| {
            metrics_server::run(context, rx.clone())
        }, ScheduleAs::SoloAct);

        

    }
}

pub(crate) fn is_empty_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(
    this: &mut SteadyActorSpotlight<RX_LEN, TX_LEN>) -> bool {
    if let Some(ref mut actor_status) = this.telemetry.state {
        if let Some(ref mut lock_guard) = actor_status.tx.try_lock() {
            lock_guard.deref_mut().is_empty()
        } else {
            false
        }
    } else {
        false
    }
}

#[cfg(feature = "core_display")]
pub(crate) fn get_current_cpu() -> i32 {
    #[cfg(unix)]
    unsafe {
        // This returns the CPU number the calling thread is running on
        libc::sched_getcpu()+1 // We add 1 to match one based display used by gnome et. all 
    }
    #[cfg(windows)]
    {
        unsafe { winapi::um::processthreadsapi::GetCurrentProcessorNumber() as usize as i32 }
    }

}



/// Tries to send all local telemetry for the given monitor.
///
/// # Parameters
/// - `this`: The local monitor to send telemetry for.
#[inline]
pub(crate) fn try_send_all_local_telemetry<const RX_LEN: usize, const TX_LEN: usize>(
    this: &mut SteadyActorSpotlight<RX_LEN, TX_LEN>, elapsed_micros: Option<u64>
) {

            // trace!("try_send_all_local_telemetry for {:?} !!!!TOP TOP TOP!!!!!!!!!!!!!!!!!!!!!!!", this.ident);
            if let Some(ref mut actor_status) = this.telemetry.state {
                // trace!("!!! only sending actor since telemtry.state is supported !!!");

                let clear_status = {
                    if let Some(ref mut lock_guard) = actor_status.tx.try_lock() {

                        let tx = lock_guard.deref_mut();                       
                        let capacity = tx.capacity();
                        let vacant_units = tx.vacant_units();
                        //trace!("lock send {:?} !!!!CRITIAL ACTOR SEND!!!!!!!!!!!!!!!!!!!!!!!!!!!{:?} vacant: {:?}", this.ident,tx,vacant_units);

                        if vacant_units >= (capacity >> 1) {
                        } else {
                            let scale = calculate_exponential_channel_backoff(capacity, vacant_units);
                            if let Some(last_elapsed) = elapsed_micros {
                                if last_elapsed < scale as u64 * this.frame_rate_ms {
                                    if scale >= 128 {
                                        let guard = this.runtime_state.read();
                                        let state = guard.deref();
                                        if !state.is_in_state(&[
                                            GraphLivelinessState::StopRequested,
                                            GraphLivelinessState::Stopped,
                                            GraphLivelinessState::StoppedUncleanly,
                                        ]) {
                                            error!(
                                                "Actor '{:?}' hard delay on actor status: scale {} empty {} of {}\nassume metrics_collector is not consuming messages !",
                                                this.ident, scale, vacant_units, capacity
                                            );
                                        }
                                      
                                    }
                                    return;
                                }
                            }
                        }


                        let msg = actor_status.status_message(this.is_running_iteration_count);
                        // trace!("message to send {:?} for {:?}",msg, this.ident);
                        match tx.shared_try_send(msg) {
                            Ok(_) => {
                                if let Some(ref mut send_tx) = this.telemetry.send_tx {
                                    if tx.local_monitor_index.lt(&MONITOR_NOT) {
                                        //happy path
                                        send_tx.count[tx.local_monitor_index] += 1;
                                        if let Some(last_elapsed) = elapsed_micros {
                                            if last_elapsed >= this.frame_rate_ms {
                                                let now = Instant::now();
                                                let dif = now.duration_since(actor_status.last_telemetry_error);
                                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                                    warn!("{:?} consider shortening period of relay_stats_periodic or adding relay_stats_all() in your work loop,\n it is called too infrequently at {}ms which is larger than your frame rate of {}ms",
                                                        this.ident, last_elapsed, this.frame_rate_ms);
                                                    actor_status.last_telemetry_error = now;
                                                }
                                            }
                                        }
                                    } else if tx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                        //happy path
                                        tx.local_monitor_index = find_my_index(send_tx, tx.channel_meta_data.meta_data.id);
                                        if tx.local_monitor_index.lt(&MONITOR_NOT) {
                                            send_tx.count[tx.local_monitor_index] += 1;
                                        } else {
                                            //we did not find our index, should not happen unless there is a code error in in shutdown
                                            if tx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                                trace!("MONITOR_UNKNOWN try send telemetry rx for  {:?} this {:?} with local index {}",this.ident,send_tx.count, tx.local_monitor_index);
                                            } else {
                                                trace!("MONITOR_NOT try send telemetry rx for  {:?} this {:?} with local index {}",this.ident,send_tx.count, tx.local_monitor_index);
                                            }

                                        }
                                    }
                                }
                                true
                            }
                            Err(_a) => {
                                let now = Instant::now();
                                let dif = now.duration_since(actor_status.last_telemetry_error);
                                if dif.as_secs() > MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                                    let guard = this.runtime_state.read();
                                    
                                        let state = guard.deref();
                                        if !state.is_in_state(&[
                                            GraphLivelinessState::StopRequested,
                                            GraphLivelinessState::Stopped,
                                            GraphLivelinessState::StoppedUncleanly,
                                        ]) {
                                            // trace!("full telemetry state channel detected from {:?} value:{:?} full:{:?} capacity: {:?}",
                                            //             this.ident, a, tx.is_full(), tx.tx.capacity());
                                            actor_status.last_telemetry_error = now;
                                        }
                                   
                                }
                                false
                            }
                        }
                    } else {
                        warn!("unable to get ACTOR DATA lock !!!!!!!!!!!!!!!!!");
                        false
                    }
                };
                if clear_status {
                    actor_status.status_reset(this.is_running_iteration_count);
                }
            } else {
                trace!("try_send_all_local_telemetry skipped actor for {:?}", this.ident);
            }
            if let Some(ref mut send_tx) = this.telemetry.send_tx {
                if let Some(ref mut lock_guard) = send_tx.tx.try_lock() {

                    //trace!("lock send {:?} tx!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!{:?} ", this.ident,send_tx.count);

                    if send_tx.count.iter().any(|x| !x.is_zero()) {
                        let tx = lock_guard.deref_mut();
                        //trace!("try send telemetry rx for  {:?} this {:?} with local index {}",this.ident,send_tx.count, tx.local_index);
                        match tx.shared_try_send(send_tx.count) {
                            Ok(_) => {
                                send_tx.count.fill(0);
                                if tx.local_monitor_index.lt(&MONITOR_NOT) {
                                    //happy path
                                    send_tx.count[tx.local_monitor_index] = 1;
                                } else if tx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                    tx.local_monitor_index = find_my_index(send_tx, tx.channel_meta_data.meta_data.id);
                                    if tx.local_monitor_index.lt(&MONITOR_NOT) {
                                        //happy path
                                        send_tx.count[tx.local_monitor_index] = 1;
                                    } else {
                                        //we did not find our index, should not happen often unless we have code error
                                        if tx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                            trace!("MONITOR_UNKNOWN try send telemetry tx for  {:?} this {:?} with local index {}",this.ident,send_tx.count, tx.local_monitor_index);
                                        } else {
                                            trace!("MONITOR_NOT try send telemetry tx for  {:?} this {:?} with local index {}",this.ident,send_tx.count, tx.local_monitor_index);
                                        }

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
            } else {
                trace!("try_send_all_local_telemetry skipped tx for {:?}", this.ident);
            }

            if let Some(ref mut send_rx) = this.telemetry.send_rx {
                if let Some(ref mut lock_guard) = send_rx.tx.try_lock() {

                    //trace!("lock send {:?} rx!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!{:?} ", this.ident,send_rx.count);

                    if send_rx.count.iter().any(|x| !x.is_zero()) {
                        let rx = lock_guard.deref_mut();
                        match rx.shared_try_send(send_rx.count) {
                            Ok(_) => {
                                send_rx.count.fill(0);
                                // above MONITOR_NOT is the MONITOR_UNKNOWN and below are actual index values
                                if rx.local_monitor_index.lt(&MONITOR_NOT) {
                                    //happy path
                                    send_rx.count[rx.local_monitor_index] = 1;
                                } else if rx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                    rx.local_monitor_index = find_my_index(send_rx, rx.channel_meta_data.meta_data.id);
                                    if rx.local_monitor_index.lt(&MONITOR_NOT) {
                                        //happy path
                                        send_rx.count[rx.local_monitor_index] = 1;
                                    } else {
                                        //we did not find our index, should not happen unless we have a code error.
                                        if rx.local_monitor_index.eq(&MONITOR_UNKNOWN) {
                                            trace!("MONITOR_UNKNOWN try send telemetry rx for  {:?} this {:?} with local index {}",this.ident,send_rx.count, rx.local_monitor_index);
                                        } else {
                                            trace!("MONITOR_NOT try send telemetry rx for  {:?} this {:?} with local index {}",this.ident,send_rx.count, rx.local_monitor_index);
                                        }
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
            } else {
                trace!("try_send_all_local_telemetry skipped rx for {:?}", this.ident);
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
    iteration_count: u64,
    telemetry_state: Option<SteadyTelemetryActorSend>,
    telemetry_send_tx: Option<SteadyTelemetrySend<TX_LEN>>,
    telemetry_send_rx: Option<SteadyTelemetrySend<RX_LEN>>,
) {
    core_exec::block_on(async move {
        if let Some(actor_status) = telemetry_state {

            // let thread_info = if this.show_thread_info {
            //     let current_thread = thread::current(); //WARN: we trust this thread is ours.
            //     Some(ThreadInfo{
            //         thread_id: current_thread.id(),
            //         #[cfg(feature = "core_display")]
            //         core: get_current_cpu(),
            //     })
            // } else {
            //     error!("do not show thread info, strange we saw it once");
            //     None
            // };

            let mut status = actor_status.status_message(iteration_count);
            if let Some(mut tx) = actor_status.tx.try_lock() {
                let needs_to_be_closed = tx.make_closed.is_some();
                if needs_to_be_closed {
                    status.bool_stop = true;
                    let _ = tx.shared_send_async(status, ident, SendSaturation::DebugWarnThenAwait).await;
                    if let Some(c) = tx.make_closed.take() {
                        let _ = c.send(());
                        //ignore any failure since this may already be closed which is ok here
                    }; 
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
                    let _ = tx.shared_send_async(send_tx.count, ident, SendSaturation::DebugWarnThenAwait).await;
                }
                tx.mark_closed();
                tx.wait_empty().await;
            }
        }

        if let Some(ref send_rx) = telemetry_send_rx {
            let mut rx = send_rx.tx.lock().await;
            if rx.make_closed.is_none() {
                if send_rx.count.iter().any(|x| !x.is_zero()) {
                    let _ = rx.shared_send_async(send_rx.count, ident, SendSaturation::DebugWarnThenAwait).await;
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

    use std::time::Duration;
    pub struct TelemetrySetup {
        channel_meta_data: Arc<ChannelMetaData>,
        refresh_rate: Duration,
        window_size: Duration,
    }

    impl TelemetrySetup {
        pub fn new(refresh_rate: Duration, window_size: Duration) -> Self {
            TelemetrySetup {
                channel_meta_data: Arc::new(ChannelMetaData::default()),
                refresh_rate,
                window_size,
            }
        }

        pub fn configure(&self) {
            // Configuration logic here
        }

        pub fn validate(&self) -> Result<(), String> {
            if self.refresh_rate.as_secs() == 0 {
                return Err("Refresh rate must be greater than zero.".to_string());
            }
            if self.window_size.as_secs() == 0 {
                return Err("Window size must be greater than zero.".to_string());
            }
            Ok(())
        }
    }

    #[test]
    fn test_new_telemetry_setup() {
        let refresh_rate = Duration::from_secs(1);
        let window_size = Duration::from_secs(10);
        let telemetry_setup = TelemetrySetup::new(refresh_rate, window_size);

        assert_eq!(telemetry_setup.refresh_rate, refresh_rate);
        assert_eq!(telemetry_setup.window_size, window_size);
        assert!(Arc::strong_count(&telemetry_setup.channel_meta_data) == 1);
    }

    #[test]
    fn test_validate_success() {
        let refresh_rate = Duration::from_secs(1);
        let window_size = Duration::from_secs(10);
        let telemetry_setup = TelemetrySetup::new(refresh_rate, window_size);

        assert!(telemetry_setup.validate().is_ok());
    }

    #[test]
    fn test_validate_failure_refresh_rate() {
        let refresh_rate = Duration::from_secs(0);
        let window_size = Duration::from_secs(10);
        let telemetry_setup = TelemetrySetup::new(refresh_rate, window_size);

        let result = telemetry_setup.validate();
        assert!(result.is_err());
        assert_eq!(result.expect_err("iternal error"), "Refresh rate must be greater than zero.");
    }

    #[test]
    fn test_validate_failure_window_size() {
        let refresh_rate = Duration::from_secs(1);
        let window_size = Duration::from_secs(0);
        let telemetry_setup = TelemetrySetup::new(refresh_rate, window_size);

        let result = telemetry_setup.validate();
        assert!(result.is_err());
        assert_eq!(result.expect_err("iternal error"), "Window size must be greater than zero.");
    }

    #[test]
    fn test_configure() {
        let refresh_rate = Duration::from_secs(1);
        let window_size = Duration::from_secs(10);
        let telemetry_setup = TelemetrySetup::new(refresh_rate, window_size);

        // Assuming configure does some internal setup, but has no return value.
        // This test ensures no panics or errors during configuration.
        telemetry_setup.configure();

        // Since `configure` has no output, the main check is that no panic occurs.
        assert!(!telemetry_setup.channel_meta_data.connects_sidecar);
    }
}
