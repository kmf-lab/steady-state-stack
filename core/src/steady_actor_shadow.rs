// ss[related actor.shadow-spotlight]
use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::{Arc, OnceLock};
// ss[related actor.shadow-spotlight]
use async_lock::Barrier;
// ss[related philosophy.structural-hierarchy]
use parking_lot::RwLock;
// ss[related philosophy.structural-hierarchy]
use std::any::Any;
// ss[related actor.shadow-spotlight]
use std::error::Error;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::{Mutex};
// ss[related philosophy.structural-hierarchy]
use futures::channel::oneshot;
// ss[related actor.shadow-spotlight]
use futures_util::stream::FuturesUnordered;
// ss[related philosophy.structural-hierarchy]
use std::future::Future;
// ss[related philosophy.structural-hierarchy]
use futures_util::{select, FutureExt, StreamExt};
// ss[related actor.shadow-spotlight]
use futures_timer::Delay;
// ss[related philosophy.structural-hierarchy]
use futures_util::future::{FusedFuture, Shared};
// ss[related philosophy.structural-hierarchy]
use aeron::aeron::Aeron;
// ss[related actor.shadow-spotlight]
use log::warn;
// ss[related philosophy.structural-hierarchy]
use ringbuf::traits::Observer;
// ss[related philosophy.structural-hierarchy]
use ringbuf::consumer::Consumer;
// ss[related actor.shadow-spotlight]
use ringbuf::producer::Producer;
// ss[related philosophy.structural-hierarchy]
use crate::{simulate_edge, ActorIdentity, Graph, GraphLiveliness, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, SteadyActor, Tx, TxCoreBundle};
// ss[related philosophy.structural-hierarchy]
use crate::actor_builder::NodeTxRx;
// ss[related actor.shadow-spotlight]
use crate::steady_actor::{
    index_wait_avoid_repeat_lane, next_index_wait_start, wait_paired_lane_ready, wait_rx_until_avail_items_ready,
    wait_tx_until_vacant_satisfied, BlockingCallFuture, SendOutcome,
};
// ss[related actor.shadow-spotlight]
use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
use crate::core_tx::TxCore;
// ss[related philosophy.structural-hierarchy]
use crate::steady_actor_core::SteadyActorCore;
// ss[related actor.shadow-spotlight]
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem};
// ss[related philosophy.structural-hierarchy]
use crate::graph_testing::SideChannelResponder;
// ss[related philosophy.structural-hierarchy]
use crate::monitor::{ActorMetaData};
// ss[related actor.shadow-spotlight]
use crate::simulate_edge::{IntoSimRunner};
// ss[related philosophy.structural-hierarchy]
use crate::steady_rx::RxDone;
// ss[related philosophy.structural-hierarchy]
use crate::steady_tx::TxDone;
// ss[related actor.shadow-spotlight]
use crate::telemetry::metrics_collector::CollectorDetail;
// ss[related philosophy.structural-hierarchy]
use crate::logging_util::steady_logger;
// ss[related philosophy.structural-hierarchy]
use crate::core_exec;

/// Context for managing actor state and interactions within the Steady framework.
// ss[related actor.shadow-spotlight]
pub struct SteadyActorShadow {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) ident: ActorIdentity,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) regeneration: u32,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) is_in_graph: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) channel_count: Arc<AtomicUsize>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// A shared future that resolves when a shutdown is requested.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) oneshot_shutdown: Shared<oneshot::Receiver<()>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) last_periodic_wait: AtomicU64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) actor_start_time: Instant,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) frame_rate_ms: u64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) team_id: usize,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_thread_info: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) aeron_init_for_tests: bool,
    // ss[impl actor.internal-behavior-logic]
    /// When true, run real actor logic (e.g. Aeron I/O); when false, use simulation.
    pub use_internal_behavior: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) shutdown_barrier: Option<Arc<Barrier>>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) index_wait_last_avail: AtomicUsize,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) index_wait_last_vacant: AtomicUsize,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) index_wait_last_avail_vacant: AtomicUsize,
}

// ss[related actor.shadow-spotlight]
impl Clone for SteadyActorShadow {
    // ss[related philosophy.structural-hierarchy]
    fn clone(&self) -> Self {
        SteadyActorShadow {
            ident: self.ident,
            regeneration: self.regeneration,
            is_in_graph: self.is_in_graph,
            channel_count: self.channel_count.clone(),
            all_telemetry_rx: self.all_telemetry_rx.clone(),
            runtime_state: self.runtime_state.clone(),
            args: self.args.clone(),
            actor_metadata: self.actor_metadata.clone(),
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown: self.oneshot_shutdown.clone(),
            last_periodic_wait: Default::default(),
            actor_start_time: Instant::now(),
            node_tx_rx: self.node_tx_rx.clone(),
            frame_rate_ms: self.frame_rate_ms,
            team_id: self.team_id,
            show_thread_info: self.show_thread_info,
            aeron_meda_driver: self.aeron_meda_driver.clone(),
            aeron_init_for_tests: self.aeron_init_for_tests,
            use_internal_behavior: self.use_internal_behavior,
            shutdown_barrier: self.shutdown_barrier.clone(),
            index_wait_last_avail: AtomicUsize::new(usize::MAX),
            index_wait_last_vacant: AtomicUsize::new(usize::MAX),
            index_wait_last_avail_vacant: AtomicUsize::new(usize::MAX),
        }
    }
}

// ss[related actor.shadow-spotlight]
impl SteadyActor for SteadyActorShadow {
    // ── Lifecycle ─────────────────────────────────────────────────────────

    // ss[related actor.shadow-spotlight]
    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool {
        rx.is_showstopper(threshold)
    }

    // ss[related actor.shadow-spotlight]
    // ss[related distributed.subscribe-publish]
    // ss[related distributed.media-driver-testing]
    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Graph::aeron_media_driver_internal(&self.aeron_meda_driver, self.aeron_init_for_tests)
    }

    // ss[related actor.shadow-spotlight]
    async fn simulated_behavior(mut self, sims: Vec<&dyn IntoSimRunner<SteadyActorShadow>>) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<SteadyActorShadow>(&mut self, sims).await
    }

    // ss[related actor.shadow-spotlight]
    fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    // ss[related actor.shadow-spotlight]
    fn relay_stats_smartly(&mut self) -> bool {
        false
    }

    // ss[related actor.shadow-spotlight]
    fn relay_stats(&mut self) {}

    // ss[related philosophy.structural-hierarchy]
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        self.wait_periodic(duration_rate).await
    }

    // ss[related actor.shadow-spotlight]
    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool {
        let liveliness = self.runtime_state.read();
        liveliness.is_in_state(target)
    }

    // ss[related actor.shadow-spotlight]
    fn is_liveliness_building(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::Building])
    }

    // ss[related actor.shadow-spotlight]
    fn is_liveliness_running(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::Running])
    }

    // ss[related actor.shadow-spotlight]
    fn is_liveliness_stop_requested(&self) -> bool {
        self.is_liveliness_in(&[GraphLivelinessState::StopRequested])
    }

    // ss[related actor.shadow-spotlight]
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> {
        let liveliness = self.runtime_state.read();
        liveliness.shutdown_timeout
    }

    // ── RxCore wrappers (delegate to SteadyActorCore) ─────────────────────

    // ss[related actor.shadow-spotlight]
    fn peek_slice<'b, T>(&self, this: &'b mut T) -> T::SliceSource<'b>
    where
        T: RxCore,
    {
        SteadyActorCore::peek_slice(this)
    }

    // ss[related actor.shadow-spotlight]
    fn take_slice<T: RxCore>(&mut self, this: &mut T, slice: T::SliceTarget<'_>) -> RxDone
    where T::MsgItem: Copy {
        SteadyActorCore::take_slice(this, slice)
    }

    // ss[related actor.shadow-spotlight]
    fn advance_take_index<T: RxCore>(&mut self, this: &mut T, count: T::MsgSize) -> RxDone {
        SteadyActorCore::advance_take_index(this, count)
    }

    // ss[related actor.shadow-spotlight]
    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T> {
        SteadyActorCore::try_peek(this)
    }

    // ss[related actor.shadow-spotlight]
    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        SteadyActorCore::try_peek_iter(this)
    }

    // ss[related actor.shadow-spotlight]
    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool {
        SteadyActorCore::is_empty(this)
    }

    // ss[related actor.shadow-spotlight]
    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize {
        SteadyActorCore::avail_units(this)
    }

    // ss[related actor.shadow-spotlight]
    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>> {
        this.shared_peek_async_timeout(None).await
    }

    // ss[related actor.shadow-spotlight]
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        SteadyActorCore::try_take(this)
    }

    // ss[related actor.shadow-spotlight]
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
    }

    // ss[related actor.shadow-spotlight]
    async fn take_async_with_timeout<T>(&mut self, this: &mut Rx<T>, timeout: Duration) -> Option<T> {
        this.shared_take_async_timeout(Some(timeout)).await
    }

    // ss[related actor.shadow-spotlight]
    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a {
        SteadyActorCore::take_into_iter(this)
    }

    // ── TxCore wrappers (delegate to SteadyActorCore) ─────────────────────

    // ss[related actor.shadow-spotlight]
    fn send_slice<T: TxCore>(&mut self, this: &mut T, slice: T::SliceSource<'_>) -> TxDone
    where T::MsgOut: Copy {
        SteadyActorCore::send_slice(this, slice)
    }

    // ss[related actor.shadow-spotlight]
    fn poke_slice<'b, T>(&self, this: &'b mut T) -> T::SliceTarget<'b>
    where T: TxCore {
        SteadyActorCore::poke_slice(this)
    }

    // ss[related actor.shadow-spotlight]
    fn advance_send_index<T: TxCore>(&mut self, this: &mut T, count: T::MsgSize) -> TxDone {
        SteadyActorCore::advance_send_index(this, count)
    }

    // ss[related actor.shadow-spotlight]
    fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
        SteadyActorCore::send_iter_until_full(this, iter)
    }

    // ss[related actor.shadow-spotlight]
    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut> {
        SteadyActorCore::try_send(this, msg)
    }

    // ss[related actor.shadow-spotlight]
    fn is_full<T: TxCore>(&self, this: &mut T) -> bool {
        SteadyActorCore::is_full(this)
    }

    // ss[related actor.shadow-spotlight]
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize {
        SteadyActorCore::vacant_units(this)
    }

    // ss[related actor.shadow-spotlight]
    async fn send_async<T: TxCore>(&mut self, this: &mut T, a: T::MsgIn<'_>, saturation: SendSaturation) -> SendOutcome<T::MsgOut> {
        this.shared_send_async(a, self.ident, saturation).await
    }

    // ── Stream defrag (kept inline, not delegated to core) ────────────────

    // ss[related actor.shadow-spotlight]
    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        out_item: &mut Tx<S>,
        out_data: &mut Tx<u8>,
        defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) {
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

        // Push payload bytes
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
        debug_assert_eq!(pushed_bytes, total_bytes, "Failed to push all payload bytes");
        unsafe {
            defrag.ringbuffer_bytes.1.advance_read_index(pushed_bytes);
        }

        // Push items
        let items_len_a = msg_count.min(items_a.len());
        let items_len_b = msg_count - items_len_a;
        out_item.tx.push_slice(&items_a[0..items_len_a]);
        if items_len_b > 0 {
            out_item.tx.push_slice(&items_b[0..items_len_b]);
        }
        unsafe {
            defrag.ringbuffer_items.1.advance_read_index(msg_count);
        }

        if msg_count == total_items {
            (msg_count as u32, total_bytes as u32, None)
        } else {
            (msg_count as u32, total_bytes as u32, Some(defrag.session_id))
        }
    }

    // ── Wait helpers (delegate to SteadyActorCore) ────────────────────────

    // ss[related actor.shadow-spotlight]
    async fn wait_periodic(&self, duration_rate: Duration) -> bool {
        let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
        let last = self.last_periodic_wait.load(Ordering::SeqCst);
        let remaining_duration = if last <= now_nanos {
            duration_rate.saturating_sub(Duration::from_nanos(now_nanos - last))
        } else {
            if Duration::from_nanos(last - now_nanos).gt(&duration_rate) {
                warn!(
                    "the actor {:?} loop took {:?} which is longer than the required periodic time of: {:?}, consider doing less work OR increating the wait_periodic duration.",
                    self.ident,
                    Duration::from_nanos(last - now_nanos),
                    duration_rate
                );
            }
            Duration::ZERO
        };
        self.last_periodic_wait
            .store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::Relaxed);
        let delay = Delay::new(remaining_duration);
        select! {
            _= self.oneshot_shutdown.clone().fuse() => false,
            _= &mut delay.fuse() => true,
        }
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_timeout(&self, timeout: Duration) -> bool {
        let delay = Delay::new(timeout);
        select! {
            _= self.oneshot_shutdown.clone().fuse() => false,
            _= &mut delay.fuse() => true,
        }
    }

    // ss[related actor.shadow-spotlight]
    async fn wait(&self, duration: Duration) {
        SteadyActorCore::wait(&self.oneshot_shutdown, duration).await
    }

    // ss[related actor.shadow-spotlight]
    async fn yield_now(&self) {
        SteadyActorCore::yield_now().await
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync,
    {
        SteadyActorCore::wait_future_void(&self.oneshot_shutdown, fut).await
    }

    // ss[related actor.shadow-spotlight]
    async fn call_async<F>(&self, operation: F) -> Option<F::Output>
    where
        F: Future,
    {
        SteadyActorCore::call_async(
            &self.oneshot_shutdown,
            self.is_liveliness_shutdown_timeout(),
            operation,
        )
        .await
    }

    // ss[related actor.shadow-spotlight]
    fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        BlockingCallFuture(core_exec::spawn_blocking(f))
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_vacant<T: TxCore>(&self, this: &mut T, size: T::MsgSize) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_shutdown_or_vacant_units(size).fuse() => x,
        }
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_avail<T: RxCore>(&self, this: &mut T, size: usize) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_closed_or_avail_units(size).fuse() => x,
        }
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_shutdown(&self) -> bool {
        SteadyActorCore::wait_shutdown(&self.oneshot_shutdown).await
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_empty().fuse() => x,
        }
    }

    // ── Bundle waits ──────────────────────────────────────────────────────

    #[allow(deprecated)]
    // ss[related actor.shadow-spotlight]
    async fn wait_vacant_bundle<T: TxCore>(
        &self,
        this: &mut TxCoreBundle<'_, T>,
        count: T::MsgSize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let mut futures = FuturesUnordered::new();
        for tx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = tx.shared_wait_shutdown_or_vacant_units(count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }
        let mut completed = 0;
        loop {
            if completed >= count_down {
                break;
            }
            select! {
                _ = self.oneshot_shutdown.clone().fuse() => {
                    result.store(false, Ordering::Relaxed);
                    break;
                }
                next = futures.next() => {
                    if next.is_some() {
                        completed += 1;
                    } else {
                        break;
                    }
                }
            }
        }
        result.load(Ordering::Relaxed)
    }

    #[allow(deprecated)]
    // ss[related actor.shadow-spotlight]
    async fn wait_avail_bundle<T: RxCore>(
        &self,
        this: &mut RxCoreBundle<'_, T>,
        count: usize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));
        let mut futures = FuturesUnordered::new();
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.shared_wait_closed_or_avail_units(count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }
        let mut completed = 0;
        loop {
            if completed >= count_down {
                break;
            }
            select! {
                _ = self.oneshot_shutdown.clone().fuse() => {
                    result.store(false, Ordering::Relaxed);
                    break;
                }
                next = futures.next() => {
                    if next.is_some() {
                        completed += 1;
                    } else {
                        break;
                    }
                }
            }
        }
        result.load(Ordering::Relaxed)
    }

    // ss[impl actor.index-wait-truthful]
    // ss[impl actor.index-wait-round-robin]
    // ss[impl actor.index-wait-repeat-bypass]
    async fn wait_avail_index<T: RxCore>(
        &self,
        this: &mut RxCoreBundle<'_, T>,
        counts: &[usize],
    ) -> Option<usize> {
        debug_assert_eq!(this.len(), counts.len(), "wait_avail_index: bundle and counts length mismatch");

        let len = this.len();
        if len == 0 {
            return None;
        }
        let last_stored = self.index_wait_last_avail.load(Ordering::Relaxed);
        let start = next_index_wait_start(last_stored, len);
        let len_start = len - start;

        let (head, tail) = this.split_at_mut(start);
        for (step, rx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            if counts[i] > 0 && rx.shared_avail_items_count() >= counts[i] {
                let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                    counts[j] > 0 && this[j].shared_avail_items_count() >= counts[j]
                });
                self.index_wait_last_avail.store(picked, Ordering::Relaxed);
                return Some(picked);
            }
        }

        let mut futures = FuturesUnordered::new();
        let (head, tail) = this.split_at_mut(start);
        for (step, rx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            if counts[i] == 0 {
                continue;
            }
            let required = counts[i];
            futures.push(async move {
                if wait_rx_until_avail_items_ready(rx, required).await {
                    Some(i)
                } else {
                    None
                }
            });
        }

        if futures.is_empty() {
            return None;
        }
        loop {
            select! {
                _ = self.oneshot_shutdown.clone().fuse() => return None,
                next = futures.next() => {
                    match next {
                        Some(Some(i)) => {
                            drop(futures);
                            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                                counts[j] > 0 && this[j].shared_avail_items_count() >= counts[j]
                            });
                            self.index_wait_last_avail.store(picked, Ordering::Relaxed);
                            return Some(picked);
                        }
                        Some(None) => {}
                        None => return None,
                    }
                }
            }
        }
    }

    // ss[related actor.shadow-spotlight]
    async fn wait_vacant_index<T: TxCore>(
        &self,
        this: &mut TxCoreBundle<'_, T>,
        counts: &[T::MsgSize],
    ) -> Option<usize> {
        debug_assert_eq!(this.len(), counts.len(), "wait_vacant_index: bundle and counts length mismatch");

        let len = this.len();
        if len == 0 {
            return None;
        }
        let last_stored = self.index_wait_last_vacant.load(Ordering::Relaxed);
        let start = next_index_wait_start(last_stored, len);

        let len_start = len - start;

        let (head, tail) = this.split_at_mut(start);
        for (step, tx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            if tx.shared_vacant_units_for(counts[i]) {
                let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                    this[j].shared_vacant_units_for(counts[j])
                });
                self.index_wait_last_vacant.store(picked, Ordering::Relaxed);
                return Some(picked);
            }
        }

        let mut futures = FuturesUnordered::new();
        let (head, tail) = this.split_at_mut(start);
        for (step, tx) in tail.iter_mut().chain(head.iter_mut()).enumerate() {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            let required = counts[i];
            futures.push(async move {
                if wait_tx_until_vacant_satisfied(tx, required).await {
                    Some(i)
                } else {
                    None
                }
            });
        }

        loop {
            select! {
                _ = self.oneshot_shutdown.clone().fuse() => return None,
                next = futures.next() => {
                    match next {
                        Some(Some(i)) => {
                            drop(futures);
                            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                                this[j].shared_vacant_units_for(counts[j])
                            });
                            self.index_wait_last_vacant.store(picked, Ordering::Relaxed);
                            return Some(picked);
                        }
                        Some(None) => {}
                        None => return None,
                    }
                }
            }
        }
    }

    // ss[impl actor.index-wait-paired]
    // ss[impl bundle.index-wait-shutdown-none]
    async fn wait_avail_vacant_index<R: RxCore, T: TxCore>(
        &self,
        rx: &mut RxCoreBundle<'_, R>,
        tx: &mut TxCoreBundle<'_, T>,
        avail_counts: &[usize],
        vacant_counts: &[T::MsgSize],
    ) -> Option<usize> {
        debug_assert_eq!(rx.len(), tx.len(), "wait_avail_vacant_index: rx and tx bundle length mismatch");
        debug_assert_eq!(rx.len(), avail_counts.len(), "wait_avail_vacant_index: rx bundle and avail_counts length mismatch");
        debug_assert_eq!(rx.len(), vacant_counts.len(), "wait_avail_vacant_index: rx bundle and vacant_counts length mismatch");

        let len = rx.len();
        if len == 0 {
            return None;
        }

        let last_stored = self.index_wait_last_avail_vacant.load(Ordering::Relaxed);
        let start = next_index_wait_start(last_stored, len);
        let len_start = len - start;

        let (rx_head, rx_tail) = rx.split_at_mut(start);
        let (tx_head, tx_tail) = tx.split_at_mut(start);
        for (step, (rx_i, tx_i)) in rx_tail
            .iter_mut()
            .chain(rx_head.iter_mut())
            .zip(tx_tail.iter_mut().chain(tx_head.iter_mut()))
            .enumerate()
        {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            let rx_ok = avail_counts[i] == 0 || rx_i.shared_avail_items_count() >= avail_counts[i];
            let tx_ok = tx_i.shared_vacant_units_for(vacant_counts[i]);
            if rx_ok && tx_ok {
                let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                    let rx_ok_j = avail_counts[j] == 0 || rx[j].shared_avail_items_count() >= avail_counts[j];
                    let tx_ok_j = tx[j].shared_vacant_units_for(vacant_counts[j]);
                    rx_ok_j && tx_ok_j
                });
                self.index_wait_last_avail_vacant.store(picked, Ordering::Relaxed);
                return Some(picked);
            }
        }

        let mut futures = FuturesUnordered::new();
        let (rx_head, rx_tail) = rx.split_at_mut(start);
        let (tx_head, tx_tail) = tx.split_at_mut(start);
        for (step, (rx_i, tx_i)) in rx_tail
            .iter_mut()
            .chain(rx_head.iter_mut())
            .zip(tx_tail.iter_mut().chain(tx_head.iter_mut()))
            .enumerate()
        {
            let i = if step < len_start {
                start + step
            } else {
                step - len_start
            };
            let rx_ok = avail_counts[i] == 0 || rx_i.shared_avail_items_count() >= avail_counts[i];
            let tx_ok = tx_i.shared_vacant_units_for(vacant_counts[i]);
            if rx_ok && tx_ok {
                continue;
            }

            let required_avail = avail_counts[i];
            let required_vacant = vacant_counts[i];
            futures.push(async move {
                if wait_paired_lane_ready(rx_i, tx_i, required_avail, required_vacant).await {
                    Some(i)
                } else {
                    None
                }
            });
        }

        if futures.is_empty() {
            return None;
        }

        loop {
            select! {
                _ = self.oneshot_shutdown.clone().fuse() => return None,
                next = futures.next() => {
                    match next {
                        Some(Some(i)) => {
                            drop(futures);
                            let picked = index_wait_avoid_repeat_lane(len, start, last_stored, i, |j| {
                                let rx_ok_j = avail_counts[j] == 0 || rx[j].shared_avail_items_count() >= avail_counts[j];
                                let tx_ok_j = tx[j].shared_vacant_units_for(vacant_counts[j]);
                                rx_ok_j && tx_ok_j
                            });
                            self.index_wait_last_avail_vacant.store(picked, Ordering::Relaxed);
                            return Some(picked);
                        }
                        Some(None) => {}
                        None => return None,
                    }
                }
            }
        }
    }

    // ── Misc ──────────────────────────────────────────────────────────────

    // ss[related actor.shadow-spotlight]
    fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|tr| SideChannelResponder::new(tr.clone(), self.ident))
    }

    // ss[related actor.shadow-spotlight]
    fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool {
        let liveliness = self.runtime_state.read();
        liveliness.is_running(self.ident, &mut accept_fn).unwrap_or(true)
    }

    // ss[related actor.shadow-spotlight]
    async fn request_shutdown(&mut self) {
        if let Some(barrier) = &self.shutdown_barrier {
            barrier.clone().wait().await;
        }
        SteadyActorCore::request_shutdown(&self.runtime_state).await
    }

    // ss[related actor.shadow-spotlight]
    fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    // ss[related actor.shadow-spotlight]
    fn identity(&self) -> ActorIdentity {
        self.ident
    }

    // ss[related actor.shadow-spotlight]
    fn set_dot_display_text(&mut self, _text: Option<&str>) {}

    // ss[related philosophy.structural-hierarchy]
    fn frame_rate_ms(&self) -> u64 {
        self.frame_rate_ms
    }

    // ss[related actor.shadow-spotlight]
    fn regeneration(&self) -> u32 {
        self.regeneration
    }
}


#[cfg(test)]
// ss[related actor.shadow-spotlight]
#[path = "steady_actor_shadow_embedded_tests.rs"]
mod steady_actor_shadow_embedded_tests;
