use std::any::type_name;
use std::ops::*;
use std::time::{Duration, Instant};
use std::sync::Arc;
use parking_lot::RwLock;
use futures::lock::Mutex;
use log::*; // allowed for all modules
use num_traits::{One, Zero};
use futures::future::select_all;
use futures_timer::Delay;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::thread::ThreadId;
use futures::channel::oneshot;
use futures_util::{FutureExt, select};
use crate::*;
use crate::actor_builder::{MCPU, Percentile, Work};
use crate::channel_builder::{Filled, Rate};
use crate::commander::SteadyCommander;
use crate::graph_liveliness::{ActorIdentity, GraphLiveliness};
use crate::graph_testing::{SideChannelResponder};
use crate::steady_config::{CONSUMED_MESSAGES_BY_COLLECTOR, REAL_CHANNEL_LENGTH_TO_COLLECTOR};
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetrySend};
use crate::telemetry::setup::send_all_local_telemetry_async;
use crate::yield_now::yield_now;

/// Represents the status of an actor.
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub struct ActorStatus {
    pub(crate) total_count_restarts: u32,
    pub(crate) iteration_start: u64,
    pub(crate) iteration_sum: u64,
    pub(crate) bool_stop: bool,
    pub(crate) await_total_ns: u64,
    pub(crate) unit_total_ns: u64,
    pub(crate) thread_info: Option<ThreadInfo>,
    pub(crate) calls: [u16; 6],
}

/// All the thread data to show for this actor
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThreadInfo {
    pub(crate) thread_id: ThreadId,
    pub(crate) team_id:   usize,
    #[cfg(feature = "core_display")]
    pub(crate) core: i32,
}

pub(crate) const CALL_SINGLE_READ: usize = 0;
pub(crate) const CALL_BATCH_READ: usize = 1;
pub(crate) const CALL_SINGLE_WRITE: usize = 2;
pub(crate) const CALL_BATCH_WRITE: usize = 3;
pub(crate) const CALL_OTHER: usize = 4;
pub(crate) const CALL_WAIT: usize = 5;

/// Metadata for an actor.
///
/// The `ActorMetaData` struct contains information and configurations related to an actor within the
/// Steady State framework. This metadata is used to monitor and manage the performance and behavior of the actor.
#[derive(Clone, Default, Debug)]
pub struct ActorMetaData {
    /// The unique identifier for the actor.
    ///
    pub(crate) ident: ActorIdentity,


    /// Indicates whether the average microcontroller processing unit (MCPU) usage is monitored.
    ///
    /// If `true`, the average MCPU usage is tracked for this actor.
    pub(crate) avg_mcpu: bool,

    /// Indicates whether the average work performed by the actor is monitored.
    ///
    /// If `true`, the average work is tracked for this actor.
    pub(crate) avg_work: bool,

    /// A list of percentiles for the MCPU usage.
    ///
    /// This list defines various percentiles to be tracked for the MCPU usage of the actor.
    pub percentiles_mcpu: Vec<Percentile>,

    /// A list of percentiles for the work performed by the actor.
    ///
    /// This list defines various percentiles to be tracked for the work metrics of the actor.
    pub percentiles_work: Vec<Percentile>,

    /// A list of standard deviations for the MCPU usage.
    ///
    /// This list defines various standard deviation metrics to be tracked for the MCPU usage of the actor.
    pub std_dev_mcpu: Vec<StdDev>,

    /// A list of standard deviations for the work performed by the actor.
    ///
    /// This list defines various standard deviation metrics to be tracked for the work metrics of the actor.
    pub std_dev_work: Vec<StdDev>,

    /// A list of triggers for the MCPU usage with associated alert colors.
    ///
    /// This list defines conditions (triggers) for the MCPU usage that, when met, will raise alerts of specific colors.
    pub trigger_mcpu: Vec<(Trigger<MCPU>, AlertColor)>,

    /// A list of triggers for the work performed by the actor with associated alert colors.
    ///
    /// This list defines conditions (triggers) for the work metrics that, when met, will raise alerts of specific colors.
    pub trigger_work: Vec<(Trigger<Work>, AlertColor)>,

    /// The refresh rate for monitoring data, expressed in bits.
    ///
    /// This field defines how frequently the monitoring data should be refreshed.
    pub refresh_rate_in_bits: u8,

    /// The size of the window bucket for metrics, expressed in bits.
    ///
    /// This field defines the size of the window bucket used for metrics aggregation.
    pub window_bucket_in_bits: u8,

    /// Indicates whether usage review is enabled for the actor.
    ///
    /// If `true`, the actor's usage is periodically reviewed.
    pub usage_review: bool,
    
}


/// Metadata for a channel, which is immutable once built.
#[derive(Clone, Default, Debug)]
pub struct ChannelMetaData {
    pub(crate) id: usize,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) capacity: usize,
    pub(crate) display_labels: bool,
    pub(crate) line_expansion: f32,
    pub(crate) show_type: Option<&'static str>,
    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,
    pub(crate) percentiles_filled: Vec<Percentile>,
    pub(crate) percentiles_rate: Vec<Percentile>,
    pub(crate) percentiles_latency: Vec<Percentile>,
    pub(crate) std_dev_inflight: Vec<StdDev>,
    pub(crate) std_dev_consumed: Vec<StdDev>,
    pub(crate) std_dev_latency: Vec<StdDev>,
    pub(crate) trigger_rate: Vec<(Trigger<Rate>, AlertColor)>,
    pub(crate) trigger_filled: Vec<(Trigger<Filled>, AlertColor)>,
    pub(crate) trigger_latency: Vec<(Trigger<Duration>, AlertColor)>,
    pub(crate) avg_filled: bool,
    pub(crate) avg_rate: bool,
    pub(crate) avg_latency: bool,
    pub(crate) min_filled: bool,
    pub(crate) max_filled: bool,
    pub(crate) connects_sidecar: bool,
    pub(crate) type_byte_count: usize,
    pub(crate) show_total: bool,
}

/// Metadata for a transmitter channel.
#[derive(Debug)]
pub struct TxMetaData(pub(crate) Arc<ChannelMetaData>);
/// supports the macro as an easy way to get the metadata
impl TxMetaData {
    pub fn meta_data(self: Self) -> TxMetaData {self}
}

/// Metadata for a receiver channel.
#[derive(Debug)]
pub struct RxMetaData(pub(crate) Arc<ChannelMetaData>);
/// supports the macro as an easy way to get the metadata
impl RxMetaData {
    pub fn meta_data(self: Self) -> RxMetaData {self}
}


/// Trait for telemetry receiver.
pub trait RxTel: Send + Sync {
    /// Returns a vector of channel metadata for transmitter channels.
    fn tx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Returns a vector of channel metadata for receiver channels.
    fn rx_channel_id_vec(&self) -> Vec<Arc<ChannelMetaData>>;

    /// Consumes actor status and returns it.
    fn consume_actor(&self) -> Option<ActorStatus>;

    /// Returns the metadata of the actor.
    fn actor_metadata(&self) -> Arc<ActorMetaData>;

    /// Consumes take data into the provided vectors.
    fn consume_take_into(&self, take_send_source: &mut Vec<(i64, i64)>, future_take: &mut Vec<i64>, future_send: &mut Vec<i64>) -> bool;

    /// Consumes send data into the provided vectors.
    fn consume_send_into(&self, take_send_source: &mut Vec<(i64, i64)>, future_send: &mut Vec<i64>) -> bool;

    /// Returns an actor receiver definition for the specified version.
    fn actor_rx(&self, version: u32) -> Option<Box<SteadyRx<ActorStatus>>>;

    /// Checks if the telemetry is empty and closed.
    fn is_empty_and_closed(&self) -> bool;
}



/// Finds the index of a given goal in the telemetry inverse local index.
///
/// # Parameters
/// - `telemetry`: A reference to a `SteadyTelemetrySend` instance.
/// - `goal`: The goal index to find.
///
/// # Returns
/// The index of the goal if found, otherwise returns `MONITOR_NOT`.
pub(crate) fn find_my_index<const LEN: usize>(telemetry: &SteadyTelemetrySend<LEN>, goal: usize) -> usize {
    let (idx, _) = telemetry.inverse_local_index
        .iter()
        .enumerate()
        .find(|(_, &value)| value == goal)
        .unwrap_or((MONITOR_NOT, &MONITOR_NOT));
    idx
}

/// Automatically sends the last telemetry data when a `LocalMonitor` instance is dropped.
impl<const RXL: usize, const TXL: usize> Drop for LocalMonitor<RXL, TXL> {
    fn drop(&mut self) {
        if self.is_in_graph {
            // finish sending the last telemetry if we are in a graph & have monitoring
            let tel = &mut self.telemetry;
            if tel.state.is_some() || tel.send_tx.is_some() || tel.send_rx.is_some() {
                send_all_local_telemetry_async(
                    self.ident,
                    self.is_running_iteration_count,
                    tel.state.take(),
                    tel.send_tx.take(),
                    tel.send_rx.take(),
                );
            }

        }
    }
}

/// Represents a local monitor that handles telemetry for an actor or channel.
///
/// # Type Parameters
/// - `RX_LEN`: The length of the receiver array.
/// - `TX_LEN`: The length of the transmitter array.
pub struct LocalMonitor<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) ident: ActorIdentity,
    pub(crate) is_in_graph: bool,
    pub(crate) telemetry: SteadyTelemetry<RX_LEN, TX_LEN>,
    pub(crate) last_telemetry_send: Instant, // NOTE: we use mutable for counts so no need for Atomic here
    pub(crate) last_periodic_wait: AtomicU64,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown: Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) actor_start_time: Instant, // never changed from context
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    pub(crate) frame_rate_ms: u64,
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    pub(crate) is_running_iteration_count: u64,
    pub(crate) show_thread_info: bool,
    pub(crate) team_id: usize,
}

pub(crate) struct FinallyRollupProfileGuard<'a> {
    st: &'a SteadyTelemetryActorSend,
    start: Instant,
}

impl Drop for FinallyRollupProfileGuard<'_> {
    fn drop(&mut self) {
        // this is ALWAYS run so we need to wait until our concurrent count is back down to zero
        if self.st.hot_profile_concurrent.fetch_sub(1, Ordering::SeqCst).is_one() {
            let p = self.st.hot_profile.load(Ordering::Relaxed);
            let _ = self.st.hot_profile_await_ns_unit.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |f| Some((f + self.start.elapsed().as_nanos() as u64).saturating_sub(p)),
            );
        }
    }
}

/// Implementation of `LocalMonitor`.
impl<const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {



    /// Marks the start of a high-activity profile period for telemetry monitoring.
    ///
    /// # Parameters
    /// - `x`: The index representing the type of call being monitored.
    pub(crate) fn start_profile(&self, x: usize) -> Option<FinallyRollupProfileGuard> {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[x].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
            // if you are the first then store otherwise we leave it as the oldest start
            if st.hot_profile_concurrent.fetch_add(1, Ordering::SeqCst).is_zero() {
                st.hot_profile.store(self.actor_start_time.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
            Some(FinallyRollupProfileGuard { st, start: self.actor_start_time })
        } else {
            None
        }
    }

    //TODO: check usage and add to the SteadyState
    pub(crate) fn validate_capacity_rx<T>(this: &mut Rx<T>, count: usize) -> usize {
        if count <= this.capacity() {
            count
        } else {
            let capacity = this.capacity();
            if this.last_error_send.elapsed().as_secs() > steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                warn!("wait_*: count {} exceeds capacity {}, reduced to capacity", count, capacity);
                this.last_error_send = Instant::now();
            }
            capacity
        }
    }

    //TODO: check usage and add to the SteadyState
    pub(crate) fn validate_capacity_tx<T>(this: &mut Tx<T>, count: usize) -> usize {
        if count <= this.capacity() {
            count
        } else {
            let capacity = this.capacity();
            if this.last_error_send.elapsed().as_secs() > steady_config::MAX_TELEMETRY_ERROR_RATE_SECONDS as u64 {
                warn!("wait_*: count {} exceeds capacity {}, reduced to capacity", count, capacity);
                this.last_error_send = Instant::now();
            }
            capacity
        }
    }

    //TODO: check usage and add to the SteadyState
    pub(crate) async fn internal_wait_shutdown(&self) -> bool {
        let one_shot = &self.oneshot_shutdown;
        let mut guard = one_shot.lock().await;
        if !guard.is_terminated() {
            let _ = guard.deref_mut().await;
        }
        true
    }

    //TODO: check usage and add to the SteadyState
    pub(crate) fn dynamic_event_count(&mut self
                                      , local_index: usize //this.local_index
                                      , id: usize //this.channel_meta_data.id
                                      , count: isize) -> usize {
        if let Some(ref mut tel) = self.telemetry.send_rx {
            tel.process_event(local_index, id, count)
        } else {
            MONITOR_NOT
        }
    }

}

impl<const RX_LEN: usize, const TX_LEN: usize> SteadyCommander for LocalMonitor<RX_LEN, TX_LEN> {


    //TODO: future feature to optimize threading, not yet implemented
    //monitor.chain_channels([rx],tx); //any of the left channels may produce output on the right


    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    ///
    /// This method holds the data if it is called more frequently than the collector can consume the data.
    /// It is designed for use in tight loops where telemetry data is collected frequently.
    fn relay_stats_smartly(&mut self) {
        let last_elapsed = self.last_telemetry_send.elapsed();
        if last_elapsed.as_micros() as u64 * (REAL_CHANNEL_LENGTH_TO_COLLECTOR as u64) >= (1000u64 * self.frame_rate_ms) {
            setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
            self.last_telemetry_send = Instant::now();
        } else {
            //if this is our first iteration flush to get initial usage
            //if the telemetry has no data flush to ensure we dump stale data
            if 0==self.is_running_iteration_count || setup::is_empty_local_telemetry(self) {
                setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
                self.last_telemetry_send = Instant::now();
            }
        }
    }

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    ///
    /// This method ignores the last telemetry send time and may overload the telemetry if called too frequently.
    /// It is designed for use in low-frequency telemetry collection scenarios, specifically cases
    /// when we know the actor will do a blocking wait for a long time and we want relay what we have before that.
    fn relay_stats(&mut self) {
        let last_elapsed = self.last_telemetry_send.elapsed();
        setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
        self.last_telemetry_send = Instant::now();
    }


    /// Periodically relays telemetry data at a specified rate.
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Returns
    /// `true` if the full waiting duration was completed without interruption.
    /// `false` if a shutdown signal was detected during the waiting period.
    ///
    /// # Asynchronous
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        let result = self.wait_periodic(duration_rate).await;
        self.relay_stats_smartly();
        result
    }
    
    /// Checks if the liveliness state matches any of the target states.
    ///
    /// # Parameters
    /// - `target`: A slice of `GraphLivelinessState`.
    ///
    /// # Returns
    /// `true` if the liveliness state matches any target state, otherwise `false`.
    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool {
        let liveliness = self.runtime_state.read();
        liveliness.is_in_state(target)
    }

    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_building(&self) -> bool {
        self.is_liveliness_in(&vec![ GraphLivelinessState::Building ])
    }
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_running(&self) -> bool {
        self.is_liveliness_in(&vec![ GraphLivelinessState::Running ])
    }
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_stop_requested(&self) -> bool {
        self.is_liveliness_in(&vec![ GraphLivelinessState::StopRequested ])
    }
    
    /// Waits while the actor is running.
    ///
    /// # Returns
    /// A future that resolves to `Ok(())` if the monitor stops, otherwise `Err(())`.
    fn wait_while_running(&self) -> impl Future<Output = Result<(), ()>> {
        WaitWhileRunningFuture::new(self.runtime_state.clone())
    }

    /// Waits until a specified number of units are available in the Rx channel bundle.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `RxBundle<T>` instance.
    /// - `avail_count`: The number of units to wait for availability.
    /// - `ready_channels`: The number of ready channels to wait for.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Send` and `Sync`.
    ///
    /// # Asynchronous
    async fn wait_shutdown_or_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
        where
            T: Send + Sync,
    {
        let _guard = self.start_profile(CALL_OTHER);

        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let futures = this.iter_mut().map(|rx| {
            let local_r = result.clone();
            async move {
                let bool_result = rx.shared_wait_shutdown_or_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }
                .boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        //this adds one extra feature as the last one
        futures.push( async move {
            let guard: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
            if !guard.is_terminated() {
                let _ = guard.deref_mut().await;
            }
        }.boxed());

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, index, remaining) = select_all(futures).await;
            if remaining.len() == index { //we had the last one finish
                result.store(false, Ordering::Relaxed);
                break;
            }
            futures = remaining;
            count_down -= 1;
            if count_down <= 1 {
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }

    /// Waits until a specified number of units are available in the Rx channel bundle.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `RxBundle<T>` instance.
    /// - `avail_count`: The number of units to wait for availability.
    /// - `ready_channels`: The number of ready channels to wait for.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Send` and `Sync`.
    ///
    /// # Asynchronous
    async fn wait_closed_or_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    {
        let _guard = self.start_profile(CALL_OTHER);

        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let futures = this.iter_mut().map(|rx| {
            let local_r = result.clone();
            async move {
                let bool_result = rx.shared_wait_closed_or_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }
                .boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        //this adds one extra feature as the last one
        futures.push( async move {
            let guard: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
            if !guard.is_terminated() {
                let _ = guard.deref_mut().await;
            }
        }.boxed());

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, index, remaining) = select_all(futures).await;
            if remaining.len() == index { //we had the last one finish
                result.store(false, Ordering::Relaxed);
                break;
            }
            futures = remaining;
            count_down -= 1;
            if count_down <= 1 {
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }


    /// Waits until a specified number of units are available in the Rx channel bundle.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `RxBundle<T>` instance.
    /// - `avail_count`: The number of units to wait for availability.
    /// - `ready_channels`: The number of ready channels to wait for.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Send` and `Sync`.
    ///
    /// # Asynchronous
    async fn wait_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    {
        let _guard = self.start_profile(CALL_OTHER);

        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let futures = this.iter_mut().map(|rx| {
            let local_r = result.clone();
            async move {
                let bool_result = rx.shared_wait_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }
                .boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        //this adds one extra feature as the last one
        futures.push( async move {
            let guard: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
            if !guard.is_terminated() {
                let _ = guard.deref_mut().await;
            }
        }.boxed());

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, index, remaining) = select_all(futures).await;
            if remaining.len() == index { //we had the last one finish
                result.store(false, Ordering::Relaxed);
                break;
            }
            futures = remaining;
            count_down -= 1;
            if count_down <= 1 {
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }


    /// Waits until a specified number of units are vacant in the Tx channel bundle.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `TxBundle<T>` instance.
    /// - `avail_count`: The number of vacant units to wait for.
    /// - `ready_channels`: The number of ready channels to wait for.
    ///
    /// # Returns
    /// `true` if the units are vacant, otherwise `false`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Send` and `Sync`.
    ///
    /// # Asynchronous
    async fn wait_shutdown_or_vacant_units_bundle<T>(&self, this: &mut TxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
        where
            T: Send,
    {
        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let _guard = self.start_profile(CALL_OTHER);

        let futures = this.iter_mut().map(|tx| {
            let local_r = result.clone();
            async move {
                let bool_result = tx.shared_wait_shutdown_or_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }.boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        //this adds one extra feature as the last one
        futures.push( async move {
            let guard: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
            if !guard.is_terminated() {
                let _ = guard.deref_mut().await;
            }
        }.boxed());

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, index, remaining) = select_all(futures).await;
            if remaining.len() == index { //we had the last one finish
                result.store(false, Ordering::Relaxed);
                break;
            }
            futures = remaining;
            count_down -= 1;
            if count_down <= 1 { //we may have 1 left for the shutdown
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }

    /// Waits until a specified number of units are vacant in the Tx channel bundle.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `TxBundle<T>` instance.
    /// - `avail_count`: The number of vacant units to wait for.
    /// - `ready_channels`: The number of ready channels to wait for.
    ///
    /// # Returns
    /// `true` if the units are vacant, otherwise `false`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Send` and `Sync`.
    ///
    /// # Asynchronous
    async fn wait_vacant_units_bundle<T>(&self, this: &mut TxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send,
    {
        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let _guard = self.start_profile(CALL_OTHER);

        let futures = this.iter_mut().map(|tx| {
            let local_r = result.clone();
            async move {
                let bool_result = tx.shared_wait_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }.boxed() // Box the future to make them the same type
        });

        let mut futures: Vec<_> = futures.collect();

        //this adds one extra feature as the last one
        futures.push( async move {
            let guard: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
            if !guard.is_terminated() {
                let _ = guard.deref_mut().await;
            }
        }.boxed());

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, index, remaining) = select_all(futures).await;
            if remaining.len() == index { //we had the last one finish
                result.store(false, Ordering::Relaxed);
                break;
            }
            futures = remaining;
            count_down -= 1;
            if count_down <= 1 { //we may have 1 left for the shutdown
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }


    /// Attempts to peek at a slice of messages without removing them from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance for peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    fn try_peek_slice<T>(&self, this: &mut Rx<T>, elems: &mut [T]) -> usize
    where
        T: Copy
    {
        this.shared_try_peek_slice(elems)
    }


    /// Asynchronously peeks at a slice of messages, waiting for a specified count to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages peeked and stored in `elems`. this can be less than wait_for_count.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    ///
    /// # Asynchronous
    async fn peek_async_slice<T>(&self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
    where
        T: Copy
    {
        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0)) //immediate timeout
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        this.shared_peek_async_slice_timeout(wait_for_count, elems, timeout).await
    }

    /// Retrieves and removes a slice of messages from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `slice`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `slice`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    fn take_slice<T>(&mut self, this: &mut Rx<T>, slice: &mut [T]) -> usize
        where
            T: Copy,
    {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_take_slice(slice);
        this.local_index = self.dynamic_event_count(
            this.local_index,
            this.channel_meta_data.id,
            done as isize);
        done
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T>
    {
        this.shared_try_peek()
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_try_peek_iter()
    }


    /// Asynchronously returns an iterator over the messages in the channel,
    /// waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Asynchronous
    async fn peek_async_iter<'a, T>(&'a self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {
        let _guard = self.start_profile(CALL_OTHER);
        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0))
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        this.shared_peek_async_iter_timeout(wait_for_count, timeout).await
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    fn is_empty<T>(&self, this: &mut Rx<T>) -> bool {
        this.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    fn avail_units<T>(&self, this: &mut Rx<T>) -> usize {
        this.shared_avail_units()
    }

    /// Asynchronously peeks at the next available message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Asynchronous
    async fn peek_async<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T>
    {
        let _guard = self.start_profile(CALL_OTHER);
        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 { 
                Some(Duration::from_millis(0)) //immediate timeout
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None //no timeout
        };
        this.shared_peek_async_timeout(timeout).await
    }


    /// Sends a slice of messages to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    fn send_slice_until_full<T>(&mut self, this: &mut Tx<T>, slice: &[T]) -> usize
        where
            T: Copy,
    {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }

        let done = this.shared_send_slice_until_full(slice);

        this.local_index = if let Some(ref mut tel) = self.telemetry.send_tx {
            tel.process_event(this.local_index, this.channel_meta_data.id, done as isize)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }

        let done = this.shared_send_iter_until_full(iter);

        this.local_index = if let Some(ref mut tel) = self.telemetry.send_tx {
            tel.process_event(this.local_index, this.channel_meta_data.id, done as isize)
        } else {
            MONITOR_NOT
        };

        done
    }

    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    fn try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T> {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_SINGLE_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }

        match this.shared_try_send(msg) {
            Ok(_) => {
                this.local_index = if let Some(ref mut tel) = self.telemetry.send_tx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                Ok(())
            }
            Err(sensitive) => Err(sensitive),
        }
    }

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    fn is_full<T>(&self, this: &mut Tx<T>) -> bool {
        this.is_full()
    }

    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    fn vacant_units<T>(&self, this: &mut Tx<T>) -> usize {
        this.shared_vacant_units()
    }

    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    async fn wait_empty<T>(&self, this: &mut Tx<T>) -> bool {
        let _guard = self.start_profile(CALL_WAIT);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                this.shared_vacant_units()==this.capacity()
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_empty();
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            this.shared_wait_empty().await
        }
    }

    /// Takes messages into an iterator.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the taken messages.
    fn take_into_iter<'a, T: Sync + Send>(& mut self, this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a
    {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }

        //we assume these will be consumed by the iterator but on drop we will
        //confirm the exact number if it is too large or too small
        let units = this.shared_avail_units();

        this.local_index = if let Some(ref mut tel) = self.telemetry.send_rx {
                                let drift = this.iterator_count_drift.load(Ordering::Relaxed);
                                this.iterator_count_drift.store(0, Ordering::Relaxed);
                                let idx = this.local_index;
                                let id = this.channel_meta_data.id;
                                tel.process_event(idx, id, units as isize + drift)
                            } else {
                                MONITOR_NOT
                            };
        // get the iterator from this RX so we can count each item there.
        let iterator_count_drift = this.iterator_count_drift.clone(); //Arc
        DriftCountIterator::new( units
                                , this.shared_take_into_iter()
                                , iterator_count_drift )


    }


    /// Calls an asynchronous function and monitors its execution for telemetry.
    ///
    /// # Parameters
    /// - `f`: The asynchronous function to call.
    ///
    /// # Returns
    /// The output of the asynchronous function `f`.
    ///
    /// # Asynchronous
    async fn call_async<F>(&self, operation: F) -> Option<F::Output>
        where
            F: Future,
    {
        let _guard = self.start_profile(CALL_OTHER);

        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        select! { _ = one_down.deref_mut() => None, r = operation.fuse() => Some(r), }
    }

    /// Waits for a specified duration, ensuring a consistent periodic interval between calls.
    ///
    /// This method helps maintain a consistent period between consecutive calls, even if the
    /// execution time of the work performed in between calls fluctuates. It calculates the
    /// remaining time until the next desired periodic interval and waits for that duration.
    ///
    /// If a shutdown signal is detected during the waiting period, the method returns early
    /// with a value of `false`. Otherwise, it waits for the full duration and returns `true`.
    ///
    /// # Arguments
    ///
    /// * `duration_rate` - The desired duration between periodic calls.
    ///
    /// # Returns
    ///
    /// * `true` if the full waiting duration was completed without interruption.
    /// * `false` if a shutdown signal was detected during the waiting period.
    ///
    async fn wait_periodic(&self, duration_rate: Duration) -> bool {
        let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
        let last = self.last_periodic_wait.load(Ordering::SeqCst);
        let remaining_duration = if last <= now_nanos {
            duration_rate.saturating_sub(Duration::from_nanos(now_nanos - last))
        } else {
            duration_rate
        };
        let waiter = Delay::new(remaining_duration);

        let _guard = self.start_profile(CALL_WAIT);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let result = select! {
                _ = &mut one_down.deref_mut() => false,
                _ = waiter.fuse() => true,
            };
        self.last_periodic_wait.store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::SeqCst);
        result
    }

    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    async fn wait(&self, duration: Duration) {
        let _guard = self.start_profile(CALL_WAIT);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        select! { _ = one_down.deref_mut() => {}, _ = Delay::new(duration).fuse() => {} }
    }

    /// Yields control so other actors may be able to make use of this thread.
    /// Returns immediately if there is nothing scheduled to check.
    async fn yield_now(&self) {
        let _guard = self.start_profile(CALL_WAIT); // start timer to measure duration
        yield_now().await;
    }


    /// Asynchronously waits for a future to complete or a shutdown signal to be received.
    ///
    /// # Parameters
    /// - `fut`: A pinned future to wait for completion.
    ///
    /// # Returns
    /// `true` if the future completes before the shutdown signal, otherwise `false`.
    ///
    /// # Asynchronous
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync {
        let mut pinned_fut = Box::pin(fut);
        let _guard = self.start_profile(CALL_OTHER);
        //TODO: should we add our dirty data check ?? not sure since we do not know what is called.
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let mut one_fused = one_down.deref_mut().fuse();
        if !one_fused.is_terminated() {
            select! { _ = one_fused => false, _ = pinned_fut => true, }
        } else {
            false
        }
    }

    /// Sends a message to the Tx channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `a`: The message to be sent.
    /// - `saturation`: The saturation policy to use.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Asynchronous
    async fn send_async<T>(&mut self, this: &mut Tx<T>, a: T, saturation: SendSaturation) -> Result<(), T> {
        let guard = self.start_profile(CALL_SINGLE_WRITE);

        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0))
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        
        let result = this.shared_send_async_timeout(a, self.ident, saturation, timeout).await; 
        drop(guard);
        match result {
            Ok(_) => {
                this.local_index = if let Some(ref mut tel) = self.telemetry.send_tx {
                    tel.process_event(this.local_index, this.channel_meta_data.id, 1)
                } else {
                    MONITOR_NOT
                };
                Ok(())
            }
            Err(sensitive) => {
                error!("Unexpected error send_async telemetry: {:?} type: {}", self.ident, type_name::<T>());
                Err(sensitive)
            }
        }
    }


    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    fn try_take<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_SINGLE_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        match this.shared_try_take() {
            Some(msg) => {
                this.local_index = self.dynamic_event_count(
                    this.local_index,
                    this.channel_meta_data.id,
                    1);
                Some(msg)
            }
            None => None,
        }
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` when a message becomes available.
    /// None is ONLY returned if there is no data AND a shutdown was requested!
    ///
    /// # Asynchronous
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        let guard = self.start_profile(CALL_SINGLE_READ);

        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0)) //stop now
            } else {
                Some(Duration::from_micros(remaining_micros as u64)) //stop at remaining time
            }
        } else {
            None //do not use the timeout 
        };

        let result = this.shared_take_async_timeout(timeout).await; //Can return None if we are shutting down
        drop(guard);
        match result {
            Some(result) => {
                this.local_index = self.dynamic_event_count(this.local_index, this.channel_meta_data.id, 1);
                Some(result)
            }
            None => None,
        }
    }

    /// Asynchronously waits until a specified number of units are available in the Rx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `count`: The number of units to wait for availability.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false`.
    ///
    /// # Asynchronous
    async fn wait_shutdown_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        let count = Self::validate_capacity_rx(this, count);

        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                false //need a relay now so return
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_shutdown_or_avail_units(count);
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            this.shared_wait_shutdown_or_avail_units(count).await
        }
    }

    /// Asynchronously waits until a specified number of units are available in the Rx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `count`: The number of units to wait for availability.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false` if closed channel.
    ///
    /// # Asynchronous
    async fn wait_closed_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        let count = Self::validate_capacity_rx(this, count);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                false //need a relay now so return
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_closed_or_avail_units(count);
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            this.shared_wait_closed_or_avail_units(count).await
        }
    }

    /// Asynchronously waits until a specified number of units are available in the Rx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    /// - `count`: The number of units to wait for availability.
    ///
    /// # Returns
    /// `true` if the units are available, otherwise `false` if pending telemetry data to send.
    ///
    /// # Asynchronous
    async fn wait_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        let count = Self::validate_capacity_rx(this, count);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                false //need a relay now so return
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_avail_units(count);
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            this.shared_wait_avail_units(count).await
        }

    }



    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    async fn wait_shutdown_or_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool {
        let _guard = self.start_profile(CALL_WAIT);
        let count = Self::validate_capacity_tx(this, count);

        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                        false //need a relay now so return
                    } else {
                        let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                        let wat = this.shared_wait_shutdown_or_vacant_units(count);
                        select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
                    }
        } else {
            this.shared_wait_shutdown_or_vacant_units(count).await
        }
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    async fn wait_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool {
        let _guard = self.start_profile(CALL_WAIT);
        let count = Self::validate_capacity_tx(this, count);

        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                false //need a relay now so return
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = this.shared_wait_vacant_units(count);
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            this.shared_wait_vacant_units(count).await
        }
    }


    /// uses opposite boolean as others since we are asking for shutdown
    /// returns true upon shutdown detection
    async fn wait_shutdown(&self) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                false //need a relay now so return
            } else {
                let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                let wat = self.internal_wait_shutdown();
                select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
            }
        } else {
            self.internal_wait_shutdown().await
        }
    }

    /// Returns a side channel responder if available.
    ///
    /// # Returns
    /// An `Option` containing a `SideChannelResponder` if available.
    fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|tr| SideChannelResponder::new(tr.clone(), self.ident))
    }

    /// Checks if the LocalMonitor is running.
    ///
    /// # Parameters
    /// - `accept_fn`: A mutable closure that returns a boolean.
    ///
    /// # Returns
    /// `true` if the monitor is running, otherwise `false`.
    #[inline]
    fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool {
        // in case we are in a tight loop and need to let other actors run on this thread.
        executor::block_on(yield_now::yield_now());

        loop {
            let result = {
                let liveliness = self.runtime_state.read();
                let ident = self.ident;
                liveliness.is_running(ident, &mut accept_fn)
            };
            if let Some(result) = result {
                if (!result) || self.is_running_iteration_count.is_zero() {
                    //stopping or starting so clear out all the buffers
                    self.relay_stats(); //testing mutable self and self flush of relay data.
                } else {
                    //if the frame rate dictates do a refresh
                    self.relay_stats_smartly();
                }
                self.is_running_iteration_count += 1;
                return result;
            } else {
                //wait until we are in a running state
                executor::block_on(Delay::new(Duration::from_millis(10)));
            }
        }
    }

    /// Requests a stop of the graph.
    ///
    /// # Returns
    /// `true` if the request was successful, otherwise `false`.
    #[inline]
    fn request_graph_stop(&self) -> bool {
        let mut liveliness = self.runtime_state.write();
        liveliness.request_shutdown();
        true
    }
    /// Retrieves the actor's arguments, cast to the specified type.
    ///
    /// # Returns
    /// An `Option<&A>` containing the arguments if available and of the correct type.
    fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    /// Retrieves the actor's identity.
    ///
    /// # Returns
    /// An `ActorIdentity` representing the actor's identity.
    fn identity(&self) -> ActorIdentity {
        self.ident
    }
}

impl<const RX_LEN: usize, const TX_LEN: usize> LocalMonitor<RX_LEN, TX_LEN> {
    fn telemetry_remaining_micros(&self) -> i64 {
       (1000i64 * self.frame_rate_ms as i64) - (self.last_telemetry_send.elapsed().as_micros() as i64
       * CONSUMED_MESSAGES_BY_COLLECTOR as i64)
    }
}
pub(crate) struct DriftCountIterator<I> {
    iter: I,
    expected_count: usize,
    actual_count: usize,
    iterator_count_drift: Arc<AtomicIsize>,
}

impl<I> DriftCountIterator<I>
where
    I: Iterator + Send,
{
    pub fn new(
        expected_count: usize,
        iter: I,
        iterator_count_drift: Arc<AtomicIsize>,
    ) -> Self {
        DriftCountIterator {
            iter,
            expected_count,
            actual_count: 0,
            iterator_count_drift,
        }
    }
}

impl<I> Drop for DriftCountIterator<I> {
    fn drop(&mut self) {
        let drift = self.actual_count as isize - self.expected_count as isize;
        if drift != 0 {
            self.iterator_count_drift.fetch_add(drift, Ordering::Relaxed);
        }
    }
}

impl<I> Iterator for DriftCountIterator<I>
where
    I: Iterator,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next();
        if item.is_some() {
            self.actual_count += 1;
        }
        item
    }
}


#[cfg(test)]
pub(crate) mod monitor_tests {

    use crate::*;
    use super::*;
    use std::ops::DerefMut;
    use lazy_static::lazy_static;
    use std::sync::Once;
    use std::time::Duration;
    use futures_timer::Delay;
    use std::sync::{Arc};
    use parking_lot::RwLock;
    use futures::channel::oneshot;
    use std::time::Instant;
    use std::sync::atomic::{AtomicUsize};
    use crate::channel_builder::ChannelBuilder;

    lazy_static! {
        static ref INIT: Once = Once::new();
    }

    // Helper method to build tx and rx arguments
    fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    // Test for try_peek
    #[test]
    fn test_try_peek() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.try_peek(&mut rx);
            assert_eq!(result, Some(&1));
        };
    }

    // Test for take_slice
    #[test]
    fn test_take_slice() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let mut monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }

    // Test for try_peek_slice
    #[test]
    fn test_try_peek_slice() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.try_peek_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }



    // Test wait_while_running method
    #[async_std::test]
    async fn test_wait_while_running() {
        let context = test_steady_context();
        let monitor = into_monitor!(context,[],[]);

        let fut = monitor.wait_while_running();
        assert_eq!(fut.await, Ok(()));

    }


    // Test is_empty method
    #[test]
    fn test_is_empty() {
        let context = test_steady_context();
        let (_tx,rx) = create_rx::<String>(vec![]); // Creating an empty Rx
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            assert!(monitor.is_empty(&mut rx));
        };
    }


    // Test for is_full
    #[test]
    fn test_is_full() {
        let (tx, _rx) = create_test_channel::<String>(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = into_monitor!(context,[],[tx]);

        if let Some(mut tx) = tx.try_lock() {
            assert!(!monitor.is_full(&mut tx));
        };
    }

    // Test for vacant_units
    #[test]
    fn test_vacant_units() {
        let (tx, _rx) = create_test_channel::<String>(13);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = into_monitor!(context,[],[tx]);

        if let Some(mut tx) = tx.try_lock() {
            let vacant_units = monitor.vacant_units(&mut tx);
            assert_eq!(vacant_units, 13); // Assuming only one unit can be vacant
        };
    }

    // Test for wait_empty
    #[async_std::test]
    async fn test_wait_empty() {
        let (tx, _rx) = create_test_channel::<String>(10);
        let tx  = tx.clone();
        let context = test_steady_context();
        let monitor = into_monitor!(context,[],[tx]);

        if let Some(mut tx) = tx.try_lock() {
            let empty = monitor.wait_empty(&mut tx).await;
            println!("Empty: {}", empty);
            println!("Vacant units: {}", monitor.vacant_units(&mut tx));
            println!("Capacity: {}", tx.capacity());
                
            assert!(empty);
        };
    }


    // Test avail_units method
    #[test]
    fn test_avail_units() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = context.into_monitor_internal([],[]);
           // into_monitor!(context,[],[]);

        if let Some(mut rx) = rx.try_lock() {
            assert_eq!(monitor.avail_units(&mut rx), 3);
        };
    }

    // Test for try_peek_iter
    #[test]
    fn test_try_peek_iter() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.try_peek_iter(&mut rx);
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async_iter
    #[async_std::test]
    async fn test_peek_async_iter() {
        let (_tx,rx) = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.peek_async_iter(&mut rx, 3).await;
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async
    #[async_std::test]
    async fn test_peek_async() {
        let (_tx,rx) = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.peek_async(&mut rx).await;
            assert_eq!(result, Some(&1));
        };
    }

    // Test for send_slice_until_full
    #[test]
    fn test_send_slice_until_full() {
        let (tx, rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let rx = rx.clone();

        let mut monitor = into_monitor!(context,[],[tx]);

        let slice = [1, 2, 3];
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = monitor.send_slice_until_full(&mut tx, &slice);
            assert_eq!(sent_count, slice.len());
            if let Some(mut rx) = rx.try_lock() {
                assert_eq!(monitor.try_take(&mut rx), Some(1));
                assert_eq!(monitor.try_peek(&mut rx), Some(&2));
            }
            
        };
    }


    // Test for try_send
    #[test]
    fn test_try_send() {
        let (tx, _rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = into_monitor!(context,[],[tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.try_send(&mut tx, 42);
            assert!(result.is_ok());
        };
    }




    // Test for call_async
    #[async_std::test]
    async fn test_call_async() {
        let context = test_steady_context();
        let monitor = into_monitor!(context,[],[]);

        let fut = async { 42 };
        let result = monitor.call_async(fut).await;
        assert_eq!(result, Some(42));
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default()
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(())),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
        }
    }

    // Helper function to create a new Rx instance
    fn create_rx<T: std::fmt::Debug>(data: Vec<T>) -> (Arc<Mutex<Tx<T>>>,Arc<Mutex<Rx<T>>>) {
        let (tx, rx) = create_test_channel(10);

        let send = tx.clone();
        if let Some(ref mut send_guard) = send.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        (tx.clone(),rx.clone())
    }

    fn create_test_channel<T: Debug>(capacity: usize) -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let builder = ChannelBuilder::new(
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            Instant::now(),
            40).with_capacity(capacity);

        builder.build::<T>()
    }

    #[test]
    fn test_simple_monitor_build() {
        let context = test_steady_context();
        let monitor = context.into_monitor([],[]);
        assert_eq!("test_actor",monitor.ident.label.name);
    }

    #[test]
    fn test_macro_monitor_build() {
        let context = test_steady_context();
        let monitor = into_monitor!(context,[],[]);
        assert_eq!("test_actor",monitor.ident.label.name);

    }


    /// Unit test for relay_stats_tx_custom.
    #[async_std::test]
    async fn test_relay_stats_tx_rx_custom() {
        util::logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let (tx_string, rx_string) = graph.channel_builder().with_capacity(8).build();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let monitor = graph.new_test_monitor("test");
        let mut monitor = into_monitor!(monitor, [rx_string], [tx_string]);

        let mut rxd = rx_string.lock().await;
        let mut txd = tx_string.lock().await;

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(&mut txd, "test".to_string(), SendSaturation::Warn).await;
            count += 1;
        }

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], threshold);
        }

        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(&mut rxd).await;
            assert_eq!(x, Some("test".to_string()));
            count -= 1;
        }

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_index], threshold);
        }

        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;

        monitor.relay_stats_smartly();

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }

    /// Unit test for relay_stats_tx_rx_batch.
    #[async_std::test]
    async fn test_relay_stats_tx_rx_batch() {
        util::logger::initialize();

        let mut graph = GraphBuilder::for_testing().build("");
        let monitor = graph.new_test_monitor("test");

        let (tx_string, rx_string) = graph.channel_builder().with_capacity(5).build();
        let tx_string = tx_string.clone();
        let rx_string = rx_string.clone();

        let mut monitor = monitor.into_monitor([&rx_string], [&tx_string]);

        let mut rx_string_guard = rx_string.lock().await;
        let mut tx_string_guard = tx_string.lock().await;

        let rxd: &mut Rx<String> = rx_string_guard.deref_mut();
        let txd: &mut Tx<String> = tx_string_guard.deref_mut();

        let threshold = 5;
        let mut count = 0;
        while count < threshold {
            let _ = monitor.send_async(txd, "test".to_string(), SendSaturation::Warn).await;
            count += 1;
            if let Some(ref mut tx) = monitor.telemetry.send_tx {
                assert_eq!(tx.count[txd.local_index], count);
            }
        }
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;
        monitor.relay_stats_smartly();

        if let Some(ref mut tx) = monitor.telemetry.send_tx {
            assert_eq!(tx.count[txd.local_index], 0);
        }

        while count > 0 {
            let x = monitor.take_async(rxd).await;
            assert_eq!(x, Some("test".to_string()));
            count -= 1;
        }
        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_index], threshold);
        }
        Delay::new(Duration::from_millis(graph.telemetry_production_rate_ms)).await;

        monitor.relay_stats_smartly();

        if let Some(ref mut rx) = monitor.telemetry.send_rx {
            assert_eq!(rx.count[rxd.local_index], 0);
        }
    }


    // Test for send_iter_until_full
    #[test]
    fn test_send_iter_until_full() {
        let (tx, rx) = create_test_channel(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let rx = rx.clone();
        let mut monitor = into_monitor!(context,[],[tx]);

        let iter = vec![1, 2, 3].into_iter();
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = monitor.send_iter_until_full(&mut tx, iter);
            assert_eq!(sent_count, 3);
            if let Some(mut rx) = rx.try_lock() {
                let i = monitor.take_into_iter(&mut rx);
                assert_eq!(i.collect::<Vec<i32>>(), vec![1, 2, 3]);
            }
        };
    }

    // Test for take_into_iter
    #[test]
    fn test_take_into_iter() {
        let data = vec![1, 2, 3, 4, 5];
        let (tx1, rx1) = create_test_channel(5);

        let tx = tx1.clone();
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        let rx = rx1.clone();
        let context = test_steady_context();
        let mut monitor = into_monitor!(context,[rx],[]);

        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(5, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(1));
                assert_eq!(iter.next(), Some(2));
                // we stop early to test if we can continue later
            }
            {//ensure we can take from where we left off
                assert_eq!(3, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(3));
                assert_eq!(iter.next(), Some(4));

            }
        };

        //we still have 1 from before
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in vec![6, 7, 8] {
                let _ = send_guard.shared_try_send(item);
            }
        };
        
        
        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(4, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(5));
                assert_eq!(iter.next(), Some(6));
                // we stop early to test if we can continue later
            }
            {//ensure we can take from where we left off
                assert_eq!(2, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(7));
                assert_eq!(iter.next(), Some(8));
            }
        };
        if let Some(ref mut send_guard) = tx.try_lock() {
            for item in vec![9, 10, 11, 12] {
                let _ = send_guard.shared_try_send(item);
            }
        };
        if let Some(mut rx) = rx.try_lock() {
            {
                assert_eq!(4, rx.avail_units());

                let mut iter = monitor.take_into_iter(&mut rx);
                assert_eq!(iter.next(), Some(9));
                assert_eq!(iter.next(), Some(10));
             //   drop(iter);
                //inject new data while we have an iterator open
                if let Some(ref mut send_guard) = tx.try_lock() {
                    assert_eq!(1, send_guard.vacant_units());
                    for item in vec![13, 14, 15] {
                        
                        match send_guard.shared_try_send(item) {
                            Ok(()) => {},
                            Err(_) => {}
                        }
                    }
                    assert_eq!(0, send_guard.vacant_units());

                };
            }

        };

    }

    // Test for wait_shutdown_or_avail_units_bundle
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);
        
        let monitor = into_monitor!(context, [rx1, rx2], []);
        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor
            .wait_shutdown_or_avail_units_bundle(&mut rx_bundle, 2, 2)
            .await;
        assert!(result);
    }

    // Test for wait_closed_or_avail_units_bundle
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);
        let monitor = into_monitor!(context, [rx1, rx2], []);

        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor
            .wait_closed_or_avail_units_bundle(&mut rx_bundle, 2, 2)
            .await;
        assert!(result);
    }

    // Test for wait_avail_units_bundle
    #[async_std::test]
    async fn test_wait_avail_units_bundle() {
        let context = test_steady_context();
        let (_tx1,rx1) = create_rx(vec![1, 2]);
        let (_tx2,rx2) = create_rx(vec![3, 4]);
        let monitor = into_monitor!(context, [rx1, rx2], []);

        let mut rx_bundle = RxBundle::new();
        if let Some(rx1) = rx1.try_lock() {
            rx_bundle.push(rx1);
        }
        if let Some(rx2) = rx2.try_lock() {
            rx_bundle.push(rx2);
        }

        let result = monitor.wait_avail_units_bundle(&mut rx_bundle, 2, 2).await;
        assert!(result);
    }

    // Test for wait_shutdown_or_vacant_units_bundle
    #[async_std::test]
    async fn test_wait_shutdown_or_vacant_units_bundle() {
        let (tx1, _rx1) = create_test_channel::<i32>(10);
        let (tx2, _rx2) = create_test_channel::<i32>(10);
        let context = test_steady_context();
        
        let tx1 =tx1.clone();
        let tx2 =tx2.clone();
        
        let monitor = into_monitor!(context, [], [tx1, tx2]);

        let mut tx_bundle = TxBundle::new();
        if let Some(tx1) = tx1.try_lock() {
            tx_bundle.push(tx1);
        }
        if let Some(tx2) = tx2.try_lock() {
            tx_bundle.push(tx2);
        }

        let result = monitor
            .wait_shutdown_or_vacant_units_bundle(&mut tx_bundle, 5, 2)
            .await;
        assert!(result);
    }

    // Test for wait_vacant_units_bundle
    #[async_std::test]
    async fn test_wait_vacant_units_bundle() {
        let (tx1, _rx1) = create_test_channel::<i32>(10);
        let (tx2, _rx2) = create_test_channel::<i32>(10);
        let context = test_steady_context();
        let tx1 =tx1.clone();
        let tx2 =tx2.clone();
        let monitor = into_monitor!(context, [], [tx1, tx2]);

        let mut tx_bundle = TxBundle::new();
        if let Some(tx1) = tx1.try_lock() {
            tx_bundle.push(tx1);
        }
        if let Some(tx2) = tx2.try_lock() {
            tx_bundle.push(tx2);
        }

        let result = monitor.wait_vacant_units_bundle(&mut tx_bundle, 5, 2).await;
        assert!(result);
    }

    // Test for wait_shutdown
    #[async_std::test]
    async fn test_wait_shutdown() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        // Simulate shutdown
        {
            let mut liveliness = monitor.runtime_state.write();
            liveliness.request_shutdown();
        }

        let result = monitor.wait_shutdown().await;
        assert!(result);
    }

    // Test for wait_periodic
    #[async_std::test]
    async fn test_wait_periodic() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        let duration = Duration::from_millis(100);
        let result = monitor.wait_periodic(duration).await;
        assert!(result);
    }

    // Test for wait
    #[async_std::test]
    async fn test_wait() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        let duration = Duration::from_millis(100);
        let start = Instant::now();
        monitor.wait(duration).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= duration);
    }

    // Test for wait_shutdown_or_avail_units
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_shutdown_or_avail_units(&mut rx, 1).await;
            assert!(!result);
        };
    }

    // Test for wait_closed_or_avail_units
    #[async_std::test]
    async fn test_wait_closed_or_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![1]);
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_closed_or_avail_units(&mut rx, 1).await;
            assert!(result);
        };
    }

    // Test for wait_avail_units
    #[async_std::test]
    async fn test_wait_avail_units() {
        let (_tx,rx) = create_rx::<i32>(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.wait_avail_units(&mut rx, 3).await;
            assert!(result);
        };
    }

    // Test for send_async
    #[async_std::test]
    async fn test_send_async() {
        let (tx, _rx) = create_test_channel::<i32>(10);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = into_monitor!(context, [], [tx]);

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.send_async(&mut tx, 42, SendSaturation::Warn).await;
            assert!(result.is_ok());
        };
    }

    // Test for args method
    #[test]
    fn test_args() {
        let args = 42u32;
        let context = test_steady_context_with_args(args);
        let monitor = into_monitor!(context, [], []);

        let retrieved_args: Option<&u32> = monitor.args();
        assert_eq!(retrieved_args, Some(&42u32));
    }

    // Test for identity method
    #[test]
    fn test_identity() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        let identity = monitor.identity();
        assert_eq!(identity.label.name, "test_actor");
    }

    // Helper function to create a test context with arguments
    fn test_steady_context_with_args<A: Any + Send + Sync>(args: A) -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default(),
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(args)),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            team_id: 0,
            show_thread_info: false,
        }
    }
 
    // Test for is_liveliness_in
    #[test]
    fn test_is_liveliness_in() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        // Initially, the liveliness state should be Building
        assert!(monitor.is_liveliness_in(&[GraphLivelinessState::Building]));
        
    }

    // Test for yield_now
    #[async_std::test]
    async fn test_yield_now() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        monitor.yield_now().await;
        // If it didn't hang, the test passes
        assert!(true);
    }
   
  
    // Test for wait_shutdown_or_avail_units with closed channel
    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units_closed_channel() {
        let (tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let rx = rx.clone();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();            
        }

        if let Some(mut rx) = rx.try_lock() {
         
            let result = monitor.wait_shutdown_or_avail_units(&mut rx, 1).await;
            assert!(!result);
        };
    }

    // Test for wait_shutdown_or_vacant_units with shutdown requested
    #[async_std::test]
    async fn test_wait_shutdown_or_vacant_units_shutdown() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let monitor = into_monitor!(context, [], [tx]);

        // Request shutdown
        {
            let mut liveliness = monitor.runtime_state.write();
            liveliness.request_shutdown();
        }

        if let Some(mut tx) = tx.try_lock() {
            let result = monitor.wait_shutdown_or_vacant_units(&mut tx, 1).await;
            assert!(result);
        };
    }

    // Test for call_async with future that returns an error
    #[async_std::test]
    async fn test_call_async_with_error() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        let fut = async { Err::<i32, &str>("error") };
        let result = monitor.call_async(fut).await;
        assert_eq!(result, Some(Err("error")));
    }
 
    // Test for args method with String
    #[test]
    fn test_args_string() {
        let args = "test_args".to_string();
        let context = test_steady_context_with_args(args.clone());
        let monitor = into_monitor!(context, [], []);

        let retrieved_args: Option<&String> = monitor.args();
        assert_eq!(retrieved_args, Some(&args));
    }



    // Test for take_slice with empty channel
    #[test]
    fn test_take_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let mut monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.take_slice(&mut rx, &mut slice);
            assert_eq!(count, 0);
        };
    }

    // Test for send_slice_until_full with full channel
    #[test]
    fn test_send_slice_until_full_full_channel() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = into_monitor!(context, [], [tx]);

        if let Some(mut tx) = tx.try_lock() {
            // Fill the channel
            let _ = monitor.try_send(&mut tx, 1);

            // Now the channel is full
            let slice = [2, 3, 4];
            let sent_count = monitor.send_slice_until_full(&mut tx, &slice);
            assert_eq!(sent_count, 0);
        };
    }

    // Test for take_into_iter with empty channel
    #[test]
    fn test_take_into_iter_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let mut iter = monitor.take_into_iter(&mut rx);
            assert_eq!(iter.next(), None);
        };
    }

    // Test for try_peek_slice with empty channel
    #[test]
    fn test_try_peek_slice_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let count = monitor.try_peek_slice(&mut rx, &mut slice);
            assert_eq!(count, 0);
        };
    }

    // Test for try_take with empty channel
    #[test]
    fn test_try_take_empty_channel() {
        let (_tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let mut monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            let result = monitor.try_take(&mut rx);
            assert_eq!(result, None);
        };
    }

    // Test for is_empty when channel has elements
    #[test]
    fn test_is_empty_with_elements() {
        let (_tx,rx) = create_rx::<i32>(vec![1, 2, 3]);
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut rx) = rx.try_lock() {
            assert!(!monitor.is_empty(&mut rx));
        };
    }

    // Test for is_full when channel is full
    #[test]
    fn test_is_full_when_full() {
        let (tx, _rx) = create_test_channel::<i32>(1);
        let context = test_steady_context();
        let tx = tx.clone();
        let mut monitor = into_monitor!(context, [], [tx]);

        if let Some(mut tx) = tx.try_lock() {
            // Fill the channel
            let _ = monitor.try_send(&mut tx, 1);
            assert!(monitor.is_full(&mut tx));
        };
    }


    // Test for wait_closed_or_avail_units with closed channel
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_closed_channel() {
        let (tx,rx) = create_rx::<i32>(vec![]);
        let context = test_steady_context();
        let monitor = into_monitor!(context, [rx], []);

        if let Some(mut tx) = tx.try_lock() {
            tx.mark_closed();
        }
        if let Some(mut rx) = rx.try_lock() {         
            let result = monitor.wait_closed_or_avail_units(&mut rx, 1).await;            
            assert!(!result);
        };
    }

    // Test for wait_shutdown with shutdown requested
    #[async_std::test]
    async fn test_wait_shutdown_with_shutdown() {
        let context = test_steady_context();
        let monitor = into_monitor!(context, [], []);

        // Request shutdown
        {
            let mut liveliness = monitor.runtime_state.write();
            liveliness.request_shutdown();
        }

        let result = monitor.wait_shutdown().await;
        assert!(result);
    }




}
