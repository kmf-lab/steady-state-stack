//! The `commander_monitor` module provides the `LocalMonitor` type and its implementation
//! of the `SteadyCommander` trait, enabling telemetry collection, profiling, and controlled
//! execution monitoring for actors and channels within the Steady framework.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use log::*;
use std::time::{Duration, Instant};
use std::sync::{Arc, OnceLock};
use async_lock::Barrier;
use parking_lot::RwLock;
use futures_util::lock::{Mutex, MutexGuard};
use futures::channel::oneshot;
use std::any::Any;
use std::error::Error;
use futures_util::future::{FusedFuture};
use futures_timer::Delay;
use futures_util::{select, FutureExt, StreamExt};
use std::future::Future;
use futures::executor;
use num_traits::Zero;
use std::ops::DerefMut;
use std::task::Poll;
use aeron::aeron::Aeron;
use futures_util::stream::FuturesUnordered;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use ringbuf::producer::Producer;
use crate::monitor::{DriftCountIterator, FinallyRollupProfileGuard, CALL_BATCH_READ, CALL_BATCH_WRITE, CALL_OTHER, CALL_SINGLE_READ, CALL_SINGLE_WRITE, CALL_WAIT};
use crate::{simulate_edge, yield_now, ActorIdentity, Graph, GraphLiveliness, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, SteadyActor, Tx, TxCoreBundle, MONITOR_NOT};
use crate::actor_builder::NodeTxRx;
use crate::steady_actor::{BlockingCallFuture, SendOutcome};
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem};
use crate::graph_testing::SideChannelResponder;
use crate::monitor_telemetry::SteadyTelemetry;
use crate::simulate_edge::IntoSimRunner;
use crate::steady_config::{CONSUMED_MESSAGES_BY_COLLECTOR, REAL_CHANNEL_LENGTH_TO_COLLECTOR};
use crate::steady_rx::RxDone;
use crate::steady_tx::TxDone;
use crate::telemetry::setup;
use crate::telemetry::setup::send_all_local_telemetry_async;
use crate::logging_util::steady_logger;
use crate::core_exec;

// Debug constant to enable verbose telemetry debugging
const ENABLE_TELEMETRY_DEBUG: bool = false;

// Threshold multiplier for detecting telemetry transmission issues
const TELEMETRY_DELAY_THRESHOLD_MULTIPLIER: u64 = 2; // e.g., 2 times the frame rate

/// Automatically sends the last telemetry data when a `LocalMonitor` instance is dropped.
impl<const RXL: usize, const TXL: usize> Drop for SteadyActorSpotlight<RXL, TXL> {
    fn drop(&mut self) {
        if self.is_in_graph {
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
pub struct SteadyActorSpotlight<const RX_LEN: usize, const TX_LEN: usize> {
    pub(crate) ident: ActorIdentity,
    pub(crate) is_in_graph: bool,
    pub(crate) telemetry: SteadyTelemetry<RX_LEN, TX_LEN>,
    pub(crate) last_telemetry_send: Instant,
    pub(crate) last_periodic_wait: AtomicU64,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown: Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) actor_start_time: Instant,
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    pub(crate) frame_rate_ms: u64,
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    pub(crate) is_running_iteration_count: u64,
    pub(crate) _team_id: usize,
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// If true, the monitor uses its internal simulation behavior for events.
    pub use_internal_behavior: bool,
    pub(crate) regeneration: u32,
    pub(crate) shutdown_barrier: Option<Arc<Barrier>>,
}

impl<const RXL: usize, const TXL: usize> SteadyActorSpotlight<RXL, TXL> {
    /// Checks if telemetry data has not been sent for longer than the threshold and logs a warning.
    fn check_telemetry_delay(&self) {
        let elapsed_since_last_send = self.last_telemetry_send.elapsed();
        let threshold_duration_ms = self.frame_rate_ms * TELEMETRY_DELAY_THRESHOLD_MULTIPLIER;
        if elapsed_since_last_send.as_millis() > threshold_duration_ms as u128 {
            // not an error but it might be
            trace!(
                "Telemetry data not sent for actor {:?} in {} ms (threshold: {} ms). Possible causes: \
                - Telemetry relay logic stalled (check `relay_stats_smartly` calls). \
                - Collector overwhelmed (verify `REAL_CHANNEL_LENGTH_TO_COLLECTOR`). \
                - Actor blocked (review actor state and logs).",
                self.ident,
                elapsed_since_last_send.as_millis(),
                threshold_duration_ms
            );
        }
    }

    #[allow(async_fn_in_trait)]
    /// Runs simulation runners within the context of this local monitor.
    pub async fn simulated_behavior(
        mut self,
        sims: Vec<&dyn IntoSimRunner<Self>>
    ) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<Self>(&mut self, sims).await
    }

    /// Marks the start of a high-activity profile period for telemetry monitoring.
    pub(crate) fn start_profile(&self, x: usize) -> Option<FinallyRollupProfileGuard> {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[x].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
            if st.hot_profile_concurrent.fetch_add(1, Ordering::SeqCst).is_zero() {
                st.hot_profile.store(self.actor_start_time.elapsed().as_nanos() as u64, Ordering::Relaxed);
            }
            Some(FinallyRollupProfileGuard { st, start: self.actor_start_time })
        } else {
            None
        }
    }

    pub(crate) async fn internal_wait_shutdown(&self) -> bool {
        let one_shot = &self.oneshot_shutdown;
        let mut guard = one_shot.lock().await;
        if !guard.is_terminated() {
            let _ = guard.deref_mut().await;
        }
        true
    }

    fn telemetry_remaining_micros(&self) -> i64 {
        (1000i64 * self.frame_rate_ms as i64) - (self.last_telemetry_send.elapsed().as_micros() as i64
            * CONSUMED_MESSAGES_BY_COLLECTOR as i64)
    }
}

impl<const RX_LEN: usize, const TX_LEN: usize> SteadyActor for SteadyActorSpotlight<RX_LEN, TX_LEN> {
    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Graph::aeron_media_driver_internal(&self.aeron_meda_driver)
    }

    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool {
        rx.is_showstopper(threshold)
    }

    async fn simulated_behavior(mut self, sims: Vec<&dyn IntoSimRunner<Self>>
    ) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<SteadyActorSpotlight<RX_LEN, TX_LEN>>(&mut self, sims).await
    }

    fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    fn relay_stats_smartly(&mut self) -> bool {

        let last_elapsed = self.last_telemetry_send.elapsed();
        if last_elapsed.as_micros() as u64 * (REAL_CHANNEL_LENGTH_TO_COLLECTOR as u64) >= (1000u64 * self.frame_rate_ms) {
            setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
            self.last_telemetry_send = Instant::now();
            if ENABLE_TELEMETRY_DEBUG {
                info!("Telemetry data sent for actor {:?} after {} micros", self.ident, last_elapsed.as_micros());
            }
            true
        } else if self.is_running_iteration_count == 0 || setup::is_empty_local_telemetry(self) {
            setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
            self.last_telemetry_send = Instant::now();
            if ENABLE_TELEMETRY_DEBUG {
                info!("Initial/empty telemetry data sent for actor {:?}", self.ident);
            }
            true
        } else {
            if ENABLE_TELEMETRY_DEBUG {
                error!("Telemetry data not sent for actor {:?} (elapsed: {} ms < threshold)",
                      self.ident, last_elapsed.as_millis());
            }
            self.check_telemetry_delay();
            false
        }
    }

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    fn relay_stats(&mut self) {
        let last_elapsed = self.last_telemetry_send.elapsed();
        setup::try_send_all_local_telemetry(self, Some(last_elapsed.as_micros() as u64));
        self.last_telemetry_send = Instant::now();
        if ENABLE_TELEMETRY_DEBUG {
            info!("Telemetry data sent for actor {:?} via relay_stats after {} ms",
                  self.ident, last_elapsed.as_millis());
        }
    }



    /// Periodically relays telemetry data at a specified rate.
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        let result = self.wait_periodic(duration_rate).await;
        self.relay_stats_smartly();
        self.check_telemetry_delay();
        result
    }

    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool {
        let liveliness = self.runtime_state.read();
        liveliness.is_in_state(target)
    }

    fn is_liveliness_building(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::Building])
    }

    fn is_liveliness_running(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::Running])
    }

    fn is_liveliness_stop_requested(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::StopRequested])
    }

    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> {
        let liveliness = self.runtime_state.read();
        liveliness.shutdown_timeout
    }

    fn peek_slice<'b, T>(&self, this: &'b mut T) -> T::SliceSource<'b>
    where T: RxCore {
        this.shared_peek_slice()
    }

    fn take_slice<T: RxCore>(&mut self, this: &mut T, slice: T::SliceTarget<'_>) -> RxDone
    where T::MsgItem: Copy {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_take_slice(slice);
        if let Some(ref mut tel) = self.telemetry.send_rx {
            this.telemetry_inc(done, tel);
        } else {
            this.monitor_not();
        }
        done
    }

    fn advance_take_index<T: RxCore>(&mut self, this: &mut T, count: T::MsgSize) -> RxDone {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_advance_index(count);
        if let Some(ref mut tel) = self.telemetry.send_rx {
            this.telemetry_inc(done, tel);
        } else {
            this.monitor_not();
        }
        done
    }

    fn advance_send_index<T: TxCore>(&mut self, this: &mut T, count: T::MsgSize) -> TxDone {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_advance_index(count);
        if let Some(ref mut tel) = self.telemetry.send_tx {
            this.telemetry_inc(done, tel);
        } else {
            this.monitor_not();
        }

        done
    }

    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T> {
        this.shared_try_peek()
    }

    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_try_peek_iter()
    }

    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool {
        this.shared_is_empty()
    }

    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize {
        this.shared_avail_units()
    }

    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>> {
        let _guard = self.start_profile(CALL_OTHER);
        let timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_millis(0))
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        this.shared_peek_async_timeout(timeout).await
    }

    fn send_slice<T: TxCore>(&mut self, this: &mut T, slice: T::SliceSource<'_>) -> TxDone
    where T::MsgOut: Copy {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_send_slice(slice);

        if let Some(ref mut tel) = self.telemetry.send_tx {
            this.telemetry_inc(done, tel);
        } else {
            this.monitor_not();
        }

        done
    }

    fn poke_slice<'b, T>(&self, this: &'b mut T) -> T::SliceTarget<'b>
    where
        T: TxCore {
        this.shared_poke_slice()
    }

    fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_send_iter_until_full(iter);
        this.local_monitor_index = if let Some(ref mut tel) = self.telemetry.send_tx {
            tel.process_event(this.local_monitor_index, this.channel_meta_data.meta_data.id, done as isize)
        } else {
            MONITOR_NOT
        };
        done
    }

    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut> {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_SINGLE_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        match this.shared_try_send(msg) {
            Ok(done_count) => {
                if let Some(ref mut tel) = self.telemetry.send_tx {
                    this.telemetry_inc(done_count, tel);
                } else {
                    this.monitor_not();
                }
                SendOutcome::Success
            }
            Err(sensitive) => SendOutcome::Blocked(sensitive),
        }
    }

    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        out_item: &mut Tx<S>,
        out_data: &mut Tx<u8>,
        defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) {
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| {
                Some(f.saturating_add(1))
            });
        }
        debug_assert!(out_data.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(out_item.make_closed.is_some(), "Send called after channel marked closed");

        let (items_a, items_b) = defrag.ringbuffer_items.1.as_slices();
        let total_items = items_a.len() + items_b.len();
        if total_items == 0 {
            return (0, 0, None);
        }

        let vacant_items = out_item.tx.vacant_len();
        let vacant_bytes = out_data.tx.vacant_len();
        let mut msg_count = 0;
        let mut total_bytes = 0;

        for item in items_a.iter().chain(items_b.iter()) {
            let item_bytes = item.length() as usize;
            if msg_count < vacant_items && total_bytes + item_bytes <= vacant_bytes {
                msg_count += 1;
                total_bytes += item_bytes;
            } else {
                break;
            }
        }

        if msg_count == 0 {
            return (0, 0, Some(defrag.session_id));
        }

        let (payload_a, payload_b) = defrag.ringbuffer_bytes.1.as_slices();
        let len_a = total_bytes.min(payload_a.len());
        let pushed_a = out_data.tx.push_slice(&payload_a[0..len_a]);
        let len_b = (total_bytes - pushed_a).min(payload_b.len());
        let pushed_b = if len_b > 0 {
            out_data.tx.push_slice(&payload_b[0..len_b])
        } else {
            0
        };
        let pushed_bytes = pushed_a + pushed_b;
        assert_eq!(pushed_bytes, total_bytes, "Failed to push all payload bytes");
        unsafe {
            defrag.ringbuffer_bytes.1.advance_read_index(pushed_bytes);
        }

        let items_len_a = msg_count.min(items_a.len());
        let items_len_b = msg_count - items_len_a;
        out_item.tx.push_slice(&items_a[0..items_len_a]);
        if items_len_b > 0 {
            out_item.tx.push_slice(&items_b[0..items_len_b]);
        }
        unsafe {
            defrag.ringbuffer_items.1.advance_read_index(msg_count);
        }

        if msg_count > 0 {
            if let Some(ref mut tel) = self.telemetry.send_tx {
                out_item.local_monitor_index = tel.process_event(
                    out_item.local_monitor_index,
                    out_item.channel_meta_data.meta_data.id,
                    msg_count as isize,
                );
                out_data.local_monitor_index = tel.process_event(
                    out_data.local_monitor_index,
                    out_data.channel_meta_data.meta_data.id,
                    total_bytes as isize,
                );
            } else {
                out_item.local_monitor_index = MONITOR_NOT;
                out_data.local_monitor_index = MONITOR_NOT;
            }
        }

        if msg_count == total_items {
            (msg_count as u32, total_bytes as u32, None)
        } else {
            (msg_count as u32, total_bytes as u32, Some(defrag.session_id))
        }
    }

    fn is_full<T: TxCore>(&self, this: &mut T) -> bool {
        this.shared_is_full()
    }

    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize {
        this.shared_vacant_units()
    }

    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool {
        let _guard = self.start_profile(CALL_WAIT);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                this.shared_is_empty()
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

    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let units = this.shared_avail_units();
        if let Some(ref mut tel) = self.telemetry.send_rx {
            let drift = this.iterator_count_drift.load(Ordering::Relaxed);
            this.iterator_count_drift.store(0, Ordering::Relaxed);
            let done_count = RxDone::Normal((units as isize + drift) as usize);
            this.telemetry_inc(done_count, tel);
        } else {
            this.monitor_not();
        }
        let iterator_count_drift = this.iterator_count_drift.clone();
        DriftCountIterator::new(units, this.shared_take_into_iter(), iterator_count_drift)
    }

    async fn call_async<F>(&self, operation: F) -> Option<F::Output>
    where F: Future {

        let operation = operation.fuse();
        futures::pin_mut!(operation); // Pin the future on the stack

        let _guard = self.start_profile(CALL_OTHER);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        if one_down.is_terminated() {
            if let Some(duration) = self.is_liveliness_shutdown_timeout() {
                //in this case we know that the shutdown signal happened but we have duration
                //before it is a hard shutdown, so we will wait 4/1 of duration
                select! {
                        _ = Delay::new(duration.div_f32(4f32)).fuse() => None,
                        r = operation => Some(r)
                    }
            } else {
                if let Poll::Ready(result) = futures::poll!(&mut operation) {
                    Some(result) // Return value if operation completed
                } else {
                    None
                }
            }
        } else {
            select! { _ = one_down.deref_mut() => None, r = operation.fuse() => Some(r), }
        }
    }

    fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        warn!("Blocking calls are not recommended but we do support them. You should however consider async options if possible.");
        info!("For engineering help in updating your solution please reach out to support@kmf-lab.com ");
        let guard = self.start_profile(CALL_OTHER);
        //this is a blocking call so we drop to measure this as 100% used core
        drop(guard);
        BlockingCallFuture(core_exec::spawn_blocking(f))
    }

    async fn wait_periodic(&self, duration_rate: Duration) -> bool {
        let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
        let last = self.last_periodic_wait.load(Ordering::SeqCst);
        let remaining_duration = if last <= now_nanos {
                duration_rate.saturating_sub(Duration::from_nanos(now_nanos - last))
            } else {
                if Duration::from_nanos(last-now_nanos).gt(&duration_rate) {
                    warn!("the actor {:?} loop took {:?} which is longer than the required periodic time of: {:?}, consider doing less work OR increating the wait_periodic duration."
                        ,self.ident, Duration::from_nanos(last-now_nanos), duration_rate);
                }
                Duration::ZERO //SHOULD NEVER HAPPEN BECAUSE last is in the future.
            };
        //must store now because this wait may be abandoned if data comes in
        self.last_periodic_wait.store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::Relaxed);
        let delay = Delay::new(remaining_duration);

        let _guard = self.start_profile(CALL_WAIT);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let result = select! {
                    _= &mut one_down.deref_mut() => false,
                    _= &mut delay.fuse() => true
                 };
        result
    }
    async fn wait_timeout(&self, timeout: Duration) -> bool {
        let delay = Delay::new(timeout);
        let _guard = self.start_profile(CALL_WAIT);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let result = select! {
                    _= &mut one_down.deref_mut() => false,
                    _= &mut delay.fuse() => true
                 };
        result
    }

    async fn wait(&self, duration: Duration) {
        let _guard = self.start_profile(CALL_WAIT);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        select! { _ = one_down.deref_mut() => {}, _ = Delay::new(duration).fuse() => {} }
    }

    async fn yield_now(&self) {
        let _guard = self.start_profile(CALL_WAIT);
        yield_now().await;
    }

    async fn wait_future_void<F>(&self, fut: F) -> bool
    where F: FusedFuture<Output = ()> + 'static + Send + Sync {
        let mut pinned_fut = Box::pin(fut);
        let _guard = self.start_profile(CALL_OTHER);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let mut one_fused = one_down.deref_mut().fuse();
        if !one_fused.is_terminated() {
            select! { _ = one_fused => false, _ = pinned_fut => true, }
        } else {
            false
        }
    }

    async fn send_async<T: TxCore>(&mut self, this: &mut T, a: T::MsgIn<'_>, saturation: SendSaturation) -> SendOutcome<T::MsgOut> {
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
        let done_one = this.done_one(&a);
        let result = this.shared_send_async_timeout(a, self.ident, saturation, timeout).await;
        drop(guard);
        match result {
            SendOutcome::Success => {
                if let Some(ref mut tel) = self.telemetry.send_tx {
                    this.telemetry_inc(done_one, tel);
                } else {
                    this.monitor_not();
                }
                SendOutcome::Success
            }
            SendOutcome::Blocked(sensitive) => SendOutcome::Blocked(sensitive),
        }
    }

    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_SINGLE_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        match this.shared_try_take() {
            Some((done_count, msg)) => {
                if let Some(ref mut tel) = self.telemetry.send_rx {
                    this.telemetry_inc(done_count, tel);
                } else {
                    this.monitor_not();
                }
                Some(msg)
            }
            None => None,
        }
    }

    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        let guard = self.start_profile(CALL_SINGLE_READ);
        let shutdown_timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0))
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        let result = this.shared_take_async_timeout(shutdown_timeout).await;
        drop(guard);
        match result {
            Some(result) => {
                if let Some(ref mut tel) = self.telemetry.send_rx {
                    this.telemetry_inc(RxDone::Normal(1), tel);
                } else {
                    this.monitor_not();
                }
                Some(result)
            }
            None => None,
        }
    }

    async fn take_async_with_timeout<T>(&mut self, this: &mut Rx<T>, user_timeout: Duration) -> Option<T> {
        let guard = self.start_profile(CALL_SINGLE_READ);
        let shutdown_timeout = if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                Some(Duration::from_micros(0))
            } else {
                Some(Duration::from_micros(remaining_micros as u64))
            }
        } else {
            None
        };
        let timeout = if let Some(st) = shutdown_timeout {
            st.min(user_timeout)
        } else {
            user_timeout
        };
        let result = this.shared_take_async_timeout(Some(timeout)).await;
        drop(guard);
        match result {
            Some(result) => {
                if let Some(ref mut tel) = self.telemetry.send_rx {
                    this.telemetry_inc(RxDone::Normal(1), tel);
                } else {
                    this.monitor_not();
                }
                Some(result)
            }
            None => None,
        }
    }

    async fn wait_avail<T: RxCore>(&self, this: &mut T, count: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);

        let count = this.shared_validate_capacity_items(count);

        if this.shared_avail_items_count() >= count {
            true
        } else {
            if self.telemetry.is_dirty() {
                let remaining_micros = self.telemetry_remaining_micros();
                if remaining_micros <= 0 {
                    false
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

    }

    async fn wait_vacant<T: TxCore>(&self, this: &mut T, size: T::MsgSize) -> bool {
        let _guard = self.start_profile(CALL_WAIT);
        if this.shared_vacant_units_for(size) {
            true
        } else {

            if self.telemetry.is_dirty() {
                let remaining_micros = self.telemetry_remaining_micros();
                if remaining_micros <= 0 {
                    false //immediate return to do telemetry will be back later
                } else {
                        let dur = Delay::new(Duration::from_micros(remaining_micros as u64));
                        let wat = this.shared_wait_shutdown_or_vacant_units(size);
                        select! {
                            _ = dur.fuse() => false,
                            x = wat.fuse() => x
                        }
                }
            } else {
                this.shared_wait_shutdown_or_vacant_units(size).await
            }
        }
    }

    async fn wait_shutdown(&self) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 && self.is_liveliness_running() {
                false
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

    fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|tr| SideChannelResponder::new(tr.clone(), self.ident))
    }

    #[inline]
    fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool {

        loop {//TODO: hold read for a few pases???
            let runnning = self.runtime_state.read().is_running(self.ident, &mut accept_fn);
            if let Some(result) = runnning {
                if result && !self.is_running_iteration_count.is_zero() {
                    if self.relay_stats_smartly() {
                        self.check_telemetry_delay();
                    }
                } else {
                    self.relay_stats();
                }
                self.is_running_iteration_count += 1;
                return result;
            } else {
                executor::block_on(Delay::new(Duration::from_millis(20)));
            }
        }
    }

    #[inline]
    async fn request_shutdown(&mut self) {
        self.relay_stats();
        if let Some(barrier) = &self.shutdown_barrier {
            barrier.clone().wait().await;
        }
        GraphLiveliness::internal_request_shutdown(self.runtime_state.clone()).await;
    }

    fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    fn identity(&self) -> ActorIdentity {
        self.ident
    }

    async fn wait_vacant_bundle<T: TxCore>(&self, this: &mut TxCoreBundle<'_, T>, size: T::MsgSize, ready_channels: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let mut futures = FuturesUnordered::new();   //TODO: optimize this similar to wait_vacant if possible
        for tx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = tx.shared_wait_shutdown_or_vacant_units(size).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }
        let mut completed = 0;
        while futures.next().await.is_some() {
            completed += 1;
            if completed >= count_down {
                break;
            }
        }
        result.load(Ordering::Relaxed)
    }

    async fn wait_avail_bundle<T: RxCore>(&self, this: &mut RxCoreBundle<'_, T>, item_count: usize, ready_channels: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let mut futures = FuturesUnordered::new();
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.shared_wait_closed_or_avail_units(item_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }
        let mut completed = 0;
        while futures.next().await.is_some() {
            completed += 1;
            if completed >= count_down {
                break;
            }
        }
        result.load(Ordering::Relaxed)
    }

    fn frame_rate_ms(&self) -> u64 {
        self.frame_rate_ms
    }

    fn regeneration(&self) -> u32 {
        self.regeneration
    }
}


#[cfg(test)]
mod steady_actor_spotlight_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::sleep;
    use std::time::Duration;
    use crate::*;
    use super::*;

    type BlockingResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    fn blocking_simulator(blocking_on: Arc<AtomicBool>) -> BlockingResult {
        while blocking_on.load(Ordering::Relaxed) {
            sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    #[test]
    fn call_blocking_test() -> Result<(), Box<dyn Error>> {
        // NOTE: this pattern needs to be used for ALL tests where applicable.

        let mut graph = GraphBuilder::for_testing().build(());

        let actor_builder = graph.actor_builder();
        let trigger = Arc::new(AtomicBool::new(true));

        actor_builder
            .with_name("call_blocking_example")
            .build(move |actor| {
                
                let mut actor = actor.into_spotlight([],[]);
                                
                let trigger = trigger.clone();
                Box::pin(async move {
                    let mut iter_count = 0;

                    let mut blocking_future: Option<BlockingCallFuture<BlockingResult>> = None;

                    //##!##// Look at this part to copy
                    while actor.is_running(|| {
                        if let Some(ref f) = blocking_future {
                            f.is_terminated() // we accept the shutdown if our blocking is terminated
                        } else {
                            true // nothing blocking is waiting
                        }
                    }) {
                        let nothing_blocking = blocking_future.is_none();
                        if nothing_blocking {
                            let trigger = trigger.clone();
                            let blocking_function = move || {
                                blocking_simulator(trigger)
                            };
                            //##!##// start up background blocking call
                            blocking_future = Some(actor.call_blocking(blocking_function));
                        }
                        if iter_count > 1000 {
                            trigger.store(false, Ordering::SeqCst);
                        }
                        if let Some(ref mut f) = blocking_future {
                            let timeout = Duration::from_millis(10);
                            //##!##// call fetch for short stretches to see if the future is done
                            let result = f.fetch(timeout).await;
                            if let Some(_r) = result {
                                //NOTE: process your result here.
                            }
                        };
                        // At some point we call
                        if iter_count == 5000 {
                            actor.request_shutdown().await;
                        }
                        iter_count += 1;
                    }
                    Ok(())
                })
            }, ScheduleAs::SoloAct);

        graph.start();
        graph.block_until_stopped(Duration::from_secs(5))
    }
}