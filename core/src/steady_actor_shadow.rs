//! The `commander_context` module defines `SteadyContext` and its implementation
//! of the `SteadyCommander` trait, providing the execution context for actors
//! including state management, telemetry channels, and lifecycle controls.
use std::time::{Duration, Instant};
use std::sync::{Arc, OnceLock};
use async_lock::Barrier;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use parking_lot::RwLock;
use std::any::Any;
use std::error::Error;
use futures_util::lock::{Mutex, MutexGuard};
use futures::channel::oneshot;
use futures_util::stream::FuturesUnordered;
use std::future::Future;
use futures_util::{select, FutureExt, StreamExt};
use futures_timer::Delay;
use futures_util::future::{Fuse, FusedFuture};
use std::thread;
use ringbuf::consumer::Consumer;
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use std::ops::DerefMut;
use std::task::Poll;
use aeron::aeron::Aeron;
use log::{info, warn};
use crate::{simulate_edge, ActorIdentity, Graph, GraphLiveliness, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, SteadyActor, Tx, TxCoreBundle};
use crate::abstract_executor_async_std::core_exec;
use crate::actor_builder::NodeTxRx;
use crate::steady_actor::{SendOutcome};
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::aqueduct_stream::{Defrag, StreamControlItem};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::{ActorMetaData};
use crate::simulate_edge::{IntoSimRunner};
use crate::steady_rx::RxDone;
use crate::steady_tx::TxDone;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::logging_util::steady_logger;
use crate::yield_now::yield_now;
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
    pub(crate) oneshot_shutdown: Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) last_periodic_wait: AtomicU64,
    pub(crate) actor_start_time: Instant,
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    pub(crate) frame_rate_ms: u64,
    pub(crate) team_id: usize,
    pub(crate) show_thread_info: bool,
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    /// Controls whether the internal simulation behavior is applied instead of actual commands.
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
            use_internal_behavior: self.use_internal_behavior,
            shutdown_barrier: self.shutdown_barrier.clone(),
        }
    }
}

impl SteadyActor for SteadyActorShadow {

    /// Checks if the current message in the receiver is a showstopper (peeked N times without being taken).
    /// If true you should consider pulling this message for a DLQ or log it or consider dropping it.
    fn is_showstopper<T>(&self, rx: &mut Rx<T>, threshold: usize) -> bool {
        rx.is_showstopper(threshold)
    }

    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Graph::aeron_media_driver_internal(&self.aeron_meda_driver)
    }

    async fn simulated_behavior(mut self, sims: Vec<&dyn IntoSimRunner<SteadyActorShadow>>) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<SteadyActorShadow>(&mut self, sims).await
    }

    /// Initializes the logger with the specified log level.
    fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

    /// No op, and only relays stats upon the LocalMonitor instance
    fn relay_stats_smartly(&mut self) -> bool {
        //do nothing this is only implemented for the monitor
        false
    }

    /// No op, and only relays stats upon the LocalMonitor instance
    fn relay_stats(&mut self) {
        //do nothing this is only implemented for the monitor
    }

    /// Waits for a specified duration. does not relay because that is only for Monior
    ///
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool {
        self.wait_periodic(duration_rate).await
        //do nothing after waiting this is only implemented for the monitor
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
        self.is_liveliness_in(&[ GraphLivelinessState::Building ])
    }
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_running(&self) -> bool {
        self.is_liveliness_in(&[ GraphLivelinessState::Running ])
    }

    /// Timeout if shutdown has been called
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> {
        let liveliness = self.runtime_state.read();
        liveliness.shutdown_timeout
    }

    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_stop_requested(&self) -> bool {
        self.is_liveliness_in(&[ GraphLivelinessState::StopRequested ])
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
    fn peek_slice<'b,T>(&self, this: &'b mut T) -> T::SliceSource<'b>
    where
        T: RxCore
    {        
        this.shared_peek_slice()
    }


    fn take_slice<T: RxCore>(&mut self, this: &mut T, slice: T::SliceTarget<'_>) -> RxDone  where T::MsgItem: Copy
    {
        this.shared_take_slice(slice)
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
    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item=&'a T> + 'a {
        this.shared_try_peek_iter()
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    fn is_empty<T: RxCore>(&self, this: &mut T) -> bool {
        this.shared_is_empty()
    }
    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    fn avail_units<T: RxCore>(&self, this: &mut T) -> T::MsgSize {
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
    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>>
    {
            this.shared_peek_async_timeout(None).await
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
    fn send_slice<T: TxCore>(& mut self, this: & mut T, slice: T::SliceSource<'_>) -> TxDone  where T::MsgOut: Copy
     {
        this.shared_send_slice(slice)
    }

    fn poke_slice<'b, T>(&self, this: &'b mut T) -> T::SliceTarget<'b>
    where
        T: TxCore{
        this.shared_poke_slice()
    }


    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    fn send_iter_until_full<T, I: Iterator<Item=T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
        this.shared_send_iter_until_full(iter)
    }
    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> SendOutcome<T::MsgOut> {
        match this.shared_try_send(msg) {
            Ok(_d) => SendOutcome::Success,
            Err(msg) => SendOutcome::Blocked(msg),
        }
    }

    fn flush_defrag_messages<S: StreamControlItem>(
        &mut self,
        out_item: &mut Tx<S>,
        out_data: &mut Tx<u8>,
        defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) {

        debug_assert!(out_data.make_closed.is_some(), "Send called after channel marked closed");
        debug_assert!(out_item.make_closed.is_some(), "Send called after channel marked closed");

        // Get item slices and check if there's anything to flush
        let (items_a, items_b) = defrag.ringbuffer_items.1.as_slices();
        let total_items = items_a.len() + items_b.len();
        if total_items == 0 {
            return (0, 0, None); // Early return for empty buffer
        }

        // Determine how many messages can fit in both channels
        let vacant_items = out_item.tx.vacant_len();
        let vacant_bytes = out_data.tx.vacant_len();
        let mut msg_count = 0;
        let mut total_bytes = 0;

        // Calculate feasible number of messages
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
            return (0, 0, Some(defrag.session_id)); // No space to flush anything
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
        assert_eq!(pushed_bytes, total_bytes, "Failed to push all payload bytes");
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

        // Return result
        if msg_count == total_items {
            debug_assert_eq!(0, defrag.ringbuffer_bytes.1.occupied_len());
            (msg_count as u32, total_bytes as u32, None)
        } else {
            (msg_count as u32, total_bytes as u32, Some(defrag.session_id))
        }
    }

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    fn is_full<T: TxCore>(&self, this: &mut T) -> bool {
        this.shared_is_full()
    }
    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> T::MsgSize {
        this.shared_vacant_units()
    }
    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    async fn wait_empty<T: TxCore>(&self, this: &mut T) -> bool {
        this.shared_wait_empty().await
    }
    /// Takes messages into an iterator.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the taken messages.
    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item=T> + 'a {
        this.shared_take_into_iter()
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
        F: Future
    {
        let operation = operation.fuse();
        futures::pin_mut!(operation); // Pin the future on the stack

        let one_down = &mut self.oneshot_shutdown.lock().await;
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
            select! { _ = one_down.deref_mut() => None, r = operation => Some(r), }
        }
    }
    fn call_blocking<F, T>(&self, f: F) -> Fuse<impl Future<Output = Option<T>> + Send>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        warn!("Blocking calls are not recommended but we do support them. You should however consider async options if possible.");
        info!("For engineering help in updating your solution please reach out to support@kmf-lab.com ");

        // Lock the shutdown signal and take ownership of the guard
        let operation = core_exec::spawn_blocking(f).fuse();

        async move {
            futures::pin_mut!(operation); // Pin the future on the stack

            let one_down = &mut self.oneshot_shutdown.lock().await;
            let result = select! {
                                        r = operation => Some(r), // Operation completes first
                                        _ =  &mut one_down.deref_mut() => None,
                                    };
            if result.is_none() && self.is_liveliness_stop_requested() {
                if let Some(duration) = self.is_liveliness_shutdown_timeout() {
                    //in this case we know that the shutdown signal happened but we have duration
                    //before it is a hard shutdown, so we will wait 4/1 of duration
                    select! {
                            _ = Delay::new(duration.div_f32(4f32)).fuse() => None,
                            r = operation => Some(r)
                        }
                } else {
                    None
                }
            } else {
                result
            }
        }.fuse()
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

        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let result = select! {
                    _= &mut one_down.deref_mut() => false,
                    _= &mut delay.fuse() => true
                 };
        result
    }
    async fn wait_timeout(&self, timeout: Duration) -> bool {
        let delay = Delay::new(timeout);
        let one_down: &mut MutexGuard<oneshot::Receiver<()>> = &mut self.oneshot_shutdown.lock().await;
        let result = select! {
                    _= &mut one_down.deref_mut() => false,
                    _= &mut delay.fuse() => true
                 };
        result
    }


    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    async fn wait(&self, duration: Duration) {
        let one_down = &mut self.oneshot_shutdown.lock().await;
        if !one_down.is_terminated() {
            select! { _ = one_down.deref_mut() => {}, _ =Delay::new(duration).fuse() => {} }
        }
    }
    /// Yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    async fn yield_now(&self) {
        yield_now().await;
    }
    /// Waits for a future to complete or until a shutdown signal is received.
    ///
    /// # Parameters
    /// - `fut`: The future to wait for.
    ///
    /// # Returns
    /// `true` if the future completed, `false` if a shutdown signal was received.
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync {
        let mut pinned_fut = Box::pin(fut);
        let one_down = &mut self.oneshot_shutdown.lock().await;
        let mut one_fused = one_down.deref_mut().fuse();
        if !one_fused.is_terminated() {
            select! { _ = one_fused => false, _ = pinned_fut => true, }
        } else {
            false
        }
    }
    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    async fn send_async<T: TxCore>(&mut self, this: &mut T, a: T::MsgIn<'_>, saturation: SendSaturation) -> SendOutcome<T::MsgOut> {
        this.shared_send_async(a, self.ident, saturation).await
    }


    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        this.shared_try_take().map(|(_d,m)|m)
    }



    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
    }


    async fn take_async_with_timeout<T>(&mut self, this: &mut Rx<T>, timeout: Duration) -> Option<T> {
        this.shared_take_async_timeout(Some(timeout)).await
    }

    fn advance_take_index<T: RxCore>(&mut self, this: &mut T, count: T::MsgSize) -> RxDone  {
        this.shared_advance_index(count)
    }

    fn advance_send_index<T: TxCore>(&mut self, this: &mut T, count: T::MsgSize) -> TxDone  {
        this.shared_advance_index(count)
    }


    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_avail<T: RxCore>(&self, this: &mut T, size: usize) -> bool {
        this.shared_wait_closed_or_avail_units(size).await
    }

    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_vacant<T: TxCore>(&self, this: &mut T, size: T::MsgSize) -> bool {
        this.shared_wait_shutdown_or_vacant_units(size).await
    }

    /// Waits until shutdown
    ///
    /// # Returns
    /// true
    async fn wait_shutdown(&self) -> bool {
        let one_shot = &self.oneshot_shutdown;
        let mut guard = one_shot.lock().await;
        if !guard.is_terminated() {
            let _ = guard.deref_mut().await;
        }
        true
    }
    /// Returns a side channel responder if available.
    ///
    /// # Returns
    /// An `Option` containing a `SideChannelResponder` if available.
    fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|tr| SideChannelResponder::new(tr.clone(), self.ident))
    }
    /// Checks if the actor is running, using a custom accept function.
    ///
    /// # Parameters
    /// - `accept_fn`: The custom accept function to check the running state.
    ///
    /// # Returns
    /// `true` if the actor is running, `false` otherwise.
    #[inline]
    fn is_running<F: FnMut() -> bool>(&mut self, mut accept_fn: F) -> bool {
        loop {
            let liveliness = self.runtime_state.read();
            let result = liveliness.is_running(self.ident, &mut accept_fn);
            if let Some(result) = result {
                return result;
            } else {
                //wait until we are finished building the actor (ie still in startup)
                thread::yield_now();
            }
        }
    }
    /// Requests a graph stop for the actor.
    ///
    #[inline]
    async fn request_shutdown(&mut self) {
        if let Some(barrier) = &self.shutdown_barrier {
            // Wait for all required actors to reach the barrier
            barrier.clone().wait().await;
        }
        GraphLiveliness::internal_request_shutdown(self.runtime_state.clone()).await;
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

    async fn wait_vacant_bundle<T: TxCore>(&self, this: &mut TxCoreBundle<'_, T>
                                           , count: T::MsgSize
                                           , ready_channels: usize) -> bool {

            let count_down = ready_channels.min(this.len());
            let result = Arc::new(AtomicBool::new(true));
            let mut futures = FuturesUnordered::new();

            // Push futures into the FuturesUnordered collection
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
            // Poll futures concurrently
            while futures.next().await.is_some() {
                completed += 1;
                if completed >= count_down {
                    break;
                }
            }
            result.load(Ordering::Relaxed)
        }

    async fn wait_avail_bundle<T: RxCore>(&self, this: &mut RxCoreBundle<'_, T>, count: usize, ready_channels: usize) -> bool {

            let count_down = ready_channels.min(this.len());
            let result = Arc::new(AtomicBool::new(true));

            let mut futures = FuturesUnordered::new();

            // Push futures into the FuturesUnordered collection
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

            // Poll futures concurrently
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
mod steady_actor_shadow_tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::sleep;
    use std::time::Duration;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use futures::Future;
    use futures::FutureExt;
    use crate::*;
    use super::*;


    fn blocking_simulator(blocking_on: Arc<AtomicBool>) {
        while blocking_on.load(Ordering::Relaxed) {
            sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn call_blocking_test() -> Result<(), Box<dyn Error>> {
        // NOTE: this pattern needs to be used for ALL tests where applicable.

        let mut graph = GraphBuilder::for_testing().build(());

        let actor_builder = graph.actor_builder();
        let trigger = Arc::new(AtomicBool::new(true));
        let blocking_future = Arc::new(AtomicBool::new(true));
        let blocking_future = Rc::new(RefCell::new(None));

        actor_builder
            .with_name("call_blocking_example")
            .build(
                async |mut actor| {

                    let mut iter_count = 0;
                    while actor.is_running(|| {
                        blocking_future.borrow().as_ref().map_or(true, |f: &Fuse<_>| {
                            f.is_terminated()
                        })
                    }) {
                        if blocking_future.borrow().is_none() {
                            let future = actor.call_blocking(|| {
                                blocking_simulator(trigger.clone())
                            }).fuse();  // Assuming .fuse() is applied here to match your storage type and usage; adjust if call_blocking already returns Fuse
                            *blocking_future.borrow_mut() = Some(future);
                        }

                        if iter_count > 1000 {
                            trigger.store(false, Ordering::SeqCst);
                        }

                        if let Some(f) = blocking_future.borrow_mut().as_mut() {
                            let waker = futures::task::noop_waker();
                            let mut cx = Context::from_waker(&waker);
                            let pinned = unsafe { Pin::new_unchecked(f) };
                            if let Poll::Ready(_result) = pinned.poll(&mut cx) {
                                // Handle the result
                            }
                        }

                        // At some point we call
                        if iter_count == 5000 {
                            actor.request_shutdown().await;
                        }
                        iter_count += 1;
                    }

                    Ok(())
                },ScheduleAs::SoloAct);

        graph.start();
        graph.block_until_stopped(Duration::from_secs(1))
    }
}