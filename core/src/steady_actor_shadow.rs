use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use async_lock::Barrier;
use parking_lot::RwLock;
use std::any::Any;
use std::error::Error;
use futures_util::lock::{Mutex};
use futures::channel::oneshot;
use futures_util::stream::FuturesUnordered;
use std::future::Future;
use futures_util::{select, FutureExt, StreamExt};
use futures_timer::Delay;
use futures_util::future::{FusedFuture, Shared};
use aeron::aeron::Aeron;
use log::warn;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use ringbuf::producer::Producer;
use crate::{simulate_edge, ActorIdentity, Graph, GraphLiveliness, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, SteadyActor, Tx, TxCoreBundle};
use crate::actor_builder::NodeTxRx;
use crate::steady_actor::{BlockingCallFuture, SendOutcome};
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::steady_actor_core::SteadyActorCore;
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::{ActorMetaData};
use crate::simulate_edge::{IntoSimRunner};
use crate::steady_rx::RxDone;
use crate::steady_tx::TxDone;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::logging_util::steady_logger;
use crate::core_exec;

/// Context for managing actor state and interactions within the Steady framework.
pub struct SteadyActorShadow {
    pub(crate) ident: ActorIdentity,
    pub(crate) regeneration: u32,
    pub(crate) is_in_graph: bool,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) args: Arc<Box<dyn Any + Send + Sync>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// A shared future that resolves when a shutdown is requested.
    pub(crate) oneshot_shutdown: Shared<oneshot::Receiver<()>>,
    pub(crate) last_periodic_wait: AtomicU64,
    pub(crate) actor_start_time: Instant,
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    pub(crate) frame_rate_ms: u64,
    pub(crate) team_id: usize,
    pub(crate) show_thread_info: bool,
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    pub(crate) aeron_init_for_tests: bool,
    pub use_internal_behavior: bool,
    pub(crate) shutdown_barrier: Option<Arc<Barrier>>,
}

impl Clone for SteadyActorShadow {
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
        }
    }
}

impl SteadyActor for SteadyActorShadow {
    // ── Lifecycle ─────────────────────────────────────────────────────────

    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool {
        rx.is_showstopper(threshold)
    }

    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Graph::aeron_media_driver_internal(&self.aeron_meda_driver, self.aeron_init_for_tests)
    }

    async fn simulated_behavior(mut self, sims: Vec<&dyn IntoSimRunner<SteadyActorShadow>>) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<SteadyActorShadow>(&mut self, sims).await
    }

    fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    fn relay_stats_smartly(&mut self) -> bool {
        false
    }

    fn relay_stats(&mut self) {}

    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        self.wait_periodic(duration_rate).await
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

    // ── RxCore wrappers (delegate to SteadyActorCore) ─────────────────────

    fn peek_slice<'b, T>(&self, this: &'b mut T) -> T::SliceSource<'b>
    where
        T: RxCore,
    {
        SteadyActorCore::peek_slice(this)
    }

    fn take_slice<T: RxCore>(&mut self, this: &mut T, slice: T::SliceTarget<'_>) -> RxDone
    where T::MsgItem: Copy {
        SteadyActorCore::take_slice(this, slice)
    }

    fn advance_take_index<T: RxCore>(&mut self, this: &mut T, count: T::MsgSize) -> RxDone {
        SteadyActorCore::advance_take_index(this, count)
    }

    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T> {
        SteadyActorCore::try_peek(this)
    }

    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
        SteadyActorCore::try_peek_iter(this)
    }

    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool {
        SteadyActorCore::is_empty(this)
    }

    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize {
        SteadyActorCore::avail_units(this)
    }

    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>> {
        this.shared_peek_async_timeout(None).await
    }

    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        SteadyActorCore::try_take(this)
    }

    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
    }

    async fn take_async_with_timeout<T>(&mut self, this: &mut Rx<T>, timeout: Duration) -> Option<T> {
        this.shared_take_async_timeout(Some(timeout)).await
    }

    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a {
        SteadyActorCore::take_into_iter(this)
    }

    // ── TxCore wrappers (delegate to SteadyActorCore) ─────────────────────

    fn send_slice<T: TxCore>(&mut self, this: &mut T, slice: T::SliceSource<'_>) -> TxDone
    where T::MsgOut: Copy {
        SteadyActorCore::send_slice(this, slice)
    }

    fn poke_slice<'b, T>(&self, this: &'b mut T) -> T::SliceTarget<'b>
    where T: TxCore {
        SteadyActorCore::poke_slice(this)
    }

    fn advance_send_index<T: TxCore>(&mut self, this: &mut T, count: T::MsgSize) -> TxDone {
        SteadyActorCore::advance_send_index(this, count)
    }

    fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
        SteadyActorCore::send_iter_until_full(this, iter)
    }

    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut> {
        SteadyActorCore::try_send(this, msg)
    }

    fn is_full<T: TxCore>(&self, this: &mut T) -> bool {
        SteadyActorCore::is_full(this)
    }

    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize {
        SteadyActorCore::vacant_units(this)
    }

    async fn send_async<T: TxCore>(&mut self, this: &mut T, a: T::MsgIn<'_>, saturation: SendSaturation) -> SendOutcome<T::MsgOut> {
        this.shared_send_async(a, self.ident, saturation).await
    }

    // ── Stream defrag (kept inline, not delegated to core) ────────────────

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

    async fn wait_timeout(&self, timeout: Duration) -> bool {
        let delay = Delay::new(timeout);
        select! {
            _= self.oneshot_shutdown.clone().fuse() => false,
            _= &mut delay.fuse() => true,
        }
    }

    async fn wait(&self, duration: Duration) {
        SteadyActorCore::wait(&self.oneshot_shutdown, duration).await
    }

    async fn yield_now(&self) {
        SteadyActorCore::yield_now().await
    }

    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync,
    {
        SteadyActorCore::wait_future_void(&self.oneshot_shutdown, fut).await
    }

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

    fn call_blocking<F, T>(&self, f: F) -> BlockingCallFuture<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        BlockingCallFuture(core_exec::spawn_blocking(f))
    }

    async fn wait_vacant<T: TxCore>(&self, this: &mut T, size: T::MsgSize) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_shutdown_or_vacant_units(size).fuse() => x,
        }
    }

    async fn wait_avail<T: RxCore>(&self, this: &mut T, size: usize) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_closed_or_avail_units(size).fuse() => x,
        }
    }

    async fn wait_shutdown(&self) -> bool {
        SteadyActorCore::wait_shutdown(&self.oneshot_shutdown).await
    }

    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool {
        select! {
            _ = self.oneshot_shutdown.clone().fuse() => false,
            x = this.shared_wait_empty().fuse() => x,
        }
    }

    // ── Bundle waits ──────────────────────────────────────────────────────

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

    // ── Misc ──────────────────────────────────────────────────────────────

    fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|tr| SideChannelResponder::new(tr.clone(), self.ident))
    }

    fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool {
        let liveliness = self.runtime_state.read();
        liveliness.is_running(self.ident, &mut accept_fn).unwrap_or(true)
    }

    async fn request_shutdown(&mut self) {
        if let Some(barrier) = &self.shutdown_barrier {
            barrier.clone().wait().await;
        }
        SteadyActorCore::request_shutdown(&self.runtime_state).await
    }

    fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    fn identity(&self) -> ActorIdentity {
        self.ident
    }

    fn set_dot_display_text(&mut self, _text: Option<&str>) {}

    fn frame_rate_ms(&self) -> u64 {
        self.frame_rate_ms
    }

    fn regeneration(&self) -> u32 {
        self.regeneration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use std::time::Duration;
    use futures_util::future::ready;

    #[test]
    fn test_wait_periodic() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let result = core_exec::block_on(shadow.wait_periodic(Duration::from_millis(10)));
        assert!(result);
    }

    #[test]
    fn test_wait_timeout() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let start = Instant::now();
        let result = core_exec::block_on(shadow.wait_timeout(Duration::from_millis(50)));
        assert!(result);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn test_yield_now() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        core_exec::block_on(shadow.yield_now());
    }

    #[test]
    fn test_relay_stats_smartly() {
        let graph = GraphBuilder::for_testing().build(());
        let mut shadow = graph.new_testing_test_monitor("test");
        // Shadow implementation always returns false
        assert!(!shadow.relay_stats_smartly());
    }

    #[test]
    fn test_relay_stats_periodic() {
        let graph = GraphBuilder::for_testing().build(());
        let mut shadow = graph.new_testing_test_monitor("test");
        let result = core_exec::block_on(shadow.relay_stats_periodic(Duration::from_millis(10)));
        assert!(result);
    }

    #[test]
    fn test_loglevel() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Should not panic
        shadow.loglevel(LogLevel::Info);
    }

    #[test]
    fn test_aeron_media_driver_none() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert!(shadow.aeron_media_driver().is_none());
    }

    #[test]
    fn test_is_showstopper() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();

        // Send a message using the lazy transmitter (has `testing_send_all`)
        tx.testing_send_all(vec![42], false);

        let shadow = graph.new_testing_test_monitor("test");

        // Clone the lazy rx to get a steady (Arc<Mutex<Rx<u8>>>) which supports .lock()
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());

        // Initially peek_repeats is 0, so not a showstopper
        assert!(!shadow.is_showstopper(&mut rx_guard, 3));

        // Peek multiple times without taking to trigger showstopper
        for _ in 0..5 {
            shadow.try_peek(&mut rx_guard);
        }
        assert!(shadow.is_showstopper(&mut rx_guard, 3));
    }

    #[test]
    fn test_wait() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let start = Instant::now();
        core_exec::block_on(shadow.wait(Duration::from_millis(30)));
        assert!(start.elapsed() >= Duration::from_millis(30));
    }

    // ── Additional tests for increased coverage ──────────────────────────

    #[test]
    fn test_call_async() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let result = core_exec::block_on(shadow.call_async(ready(42u32)));
        assert_eq!(result, Some(42));
    }

    #[test]
    fn test_wait_shutdown() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Trigger shutdown by requesting it on the graph's runtime state
        let runtime_state = shadow.runtime_state.clone();
        core_exec::block_on(SteadyActorCore::request_shutdown(&runtime_state));
        let result = core_exec::block_on(shadow.wait_shutdown());
        assert!(result);
    }

    #[test]
    fn test_wait_vacant_and_avail() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let shadow = graph.new_testing_test_monitor("test");

        // Test wait_vacant with a tx guard that has space
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let vacant_result = core_exec::block_on(shadow.wait_vacant(&mut tx_guard, 1usize));
        assert!(vacant_result);

        // Test wait_avail with rx guard that has no data
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let avail_result = core_exec::block_on(shadow.wait_avail(&mut rx_guard, 1));
        assert!(!avail_result); // No data, shutdown not issued, but channel not closed → false
    }

    #[test]
    fn test_wait_empty() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let shadow = graph.new_testing_test_monitor("test");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let result = core_exec::block_on(shadow.wait_empty(&mut tx_guard));
        assert!(result); // Empty channel returns true
    }

    #[test]
    fn test_wait_future_void() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let fut = ready(()).fuse();
        let result = core_exec::block_on(shadow.wait_future_void(fut));
        assert!(result);
    }

    #[test]
    fn test_sidechannel_responder_none() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert!(shadow.sidechannel_responder().is_none());
    }

    #[test]
    fn test_is_running() {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        let mut shadow = graph.new_testing_test_monitor("test");
        let running = shadow.is_running(|| true);
        assert!(running);
        graph.request_shutdown();
        // After shutdown, is_running should return false
        let running2 = shadow.is_running(|| true);
        assert!(!running2);
    }

    #[test]
    fn test_request_shutdown() {
        let mut graph = GraphBuilder::for_testing().build(());
        graph.start();
        let mut shadow = graph.new_testing_test_monitor("test");
        core_exec::block_on(shadow.request_shutdown());
        assert!(shadow.is_liveliness_stop_requested());
    }

    #[test]
    fn test_args() {
        let graph = GraphBuilder::for_testing().build(42i32);
        let shadow = graph.new_testing_test_monitor("test");
        // The graph was built with i32
        let value: Option<&i32> = shadow.args();
        assert_eq!(value, Some(&42));
    }

    #[test]
    fn test_identity() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test_monitor");
        let id = shadow.identity();
        assert_eq!(id.label.name, "test_monitor");
        assert_eq!(id.id, usize::MAX);
    }

    #[test]
    fn test_frame_rate_ms() {
        let mut graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // The default telemetry production rate for testing is 1000 ms
        assert_eq!(shadow.frame_rate_ms(), 1000);
    }

    #[test]
    fn test_regeneration() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        assert_eq!(shadow.regeneration(), 0);
    }

    #[test]
    fn test_liveliness_states() {
        let mut graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Initial state is Building until start
        assert!(shadow.is_liveliness_building());
        assert!(!shadow.is_liveliness_running());
        assert!(!shadow.is_liveliness_stop_requested());
        graph.start();
        assert!(!shadow.is_liveliness_building());
        assert!(shadow.is_liveliness_running());
        graph.request_shutdown();
        assert!(shadow.is_liveliness_stop_requested());
    }

    #[test]
    fn test_liveliness_shutdown_timeout() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        // Default has no shutdown_timeout
        assert!(shadow.is_liveliness_shutdown_timeout().is_none());
    }

    #[test]
    fn test_flush_defrag_empty() {
        // Create a shadow and call flush_defrag_messages with an empty defrag
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder()
            .with_capacity(5)
            .build_stream::<StreamEgress>(1);
        let mut shadow = graph.new_testing_test_monitor("test");

        let mut defrag = Defrag::<StreamEgress>::new(1, 10, 100);
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        // Workaround: borrow fields through separate deref to satisfy borrow checker
        let tx_inner = &mut *tx_guard;
        let (msgs, bytes, session) = shadow.flush_defrag_messages(
            &mut tx_inner.control_channel,
            &mut tx_inner.payload_channel,
            &mut defrag,
        );
        assert_eq!(msgs, 0);
        assert_eq!(bytes, 0);
        assert_eq!(session, None);
    }

    #[test]
    fn test_try_send_and_try_take() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let send_result = shadow.try_send(&mut tx_guard, 42);
        assert!(send_result.is_sent());
        drop(tx_guard);

        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let take_result = shadow.try_take(&mut rx_guard);
        assert_eq!(take_result, Some(42));
    }

    #[test]
    fn test_is_full_and_vacant_units() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(3).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        // Fresh channel should not be full
        assert!(!shadow.is_full(&mut tx_guard));
        assert_eq!(shadow.vacant_units(&mut tx_guard), 3);
        drop(tx_guard);

        // Fill the channel
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        for i in 0..3 {
            let _ = tx_guard.shared_try_send(i);
        }
        assert!(shadow.is_full(&mut tx_guard));
        assert_eq!(shadow.vacant_units(&mut tx_guard), 0);
    }

    #[test]
    fn test_send_iter_until_full() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, _rx) = graph.channel_builder().with_capacity(4).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let iter = vec![1, 2, 3].into_iter();
        let count = shadow.send_iter_until_full(&mut tx_guard, iter);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_is_empty_and_avail_units() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        let mut shadow = graph.new_testing_test_monitor("test");

        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        assert!(shadow.is_empty(&mut rx_guard));
        assert_eq!(shadow.avail_units(&mut rx_guard), 0);
        drop(rx_guard);

        // Send a message and check again
        let tx_steady = tx.clone();
        let mut tx_guard = core_exec::block_on(tx_steady.lock());
        let _ = tx_guard.shared_try_send(99);
        drop(tx_guard);

        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        assert!(!shadow.is_empty(&mut rx_guard));
        assert_eq!(shadow.avail_units(&mut rx_guard), 1);
    }

    #[test]
    fn test_call_blocking() {
        let graph = GraphBuilder::for_testing().build(());
        let shadow = graph.new_testing_test_monitor("test");
        let fut = shadow.call_blocking(|| 42u32);
        let result = core_exec::block_on(fut);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_take_async_with_timeout() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![10, 20], false);
        let mut shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let taken = core_exec::block_on(shadow.take_async_with_timeout(&mut rx_guard, Duration::from_millis(100)));
        assert_eq!(taken, Some(10));
    }

    #[test]
    fn test_take_into_iter() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![1, 2, 3], false);
        let mut shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let items: Vec<u8> = shadow.take_into_iter(&mut rx_guard).collect();
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn test_peek_async() {
        let mut graph = GraphBuilder::for_testing().build(());
        let (tx, rx) = graph.channel_builder().with_capacity(5).build_channel::<u8>();
        tx.testing_send_all(vec![77], false);
        let shadow = graph.new_testing_test_monitor("test");
        let rx_steady = rx.clone();
        let mut rx_guard = core_exec::block_on(rx_steady.lock());
        let peeked = core_exec::block_on(shadow.peek_async(&mut rx_guard));
        assert!(peeked.is_some());
    }
}
