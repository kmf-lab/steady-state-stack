use std::time::{Duration, Instant};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use parking_lot::RwLock;
use std::any::Any;
use futures_util::lock::Mutex;
use futures::channel::oneshot;
use futures_util::stream::FuturesUnordered;
use log::warn;
use std::future::Future;
use futures_util::{select, FutureExt, StreamExt};
use futures_timer::Delay;
use futures_util::future::FusedFuture;
use std::thread;
use ringbuf::consumer::Consumer;
use ringbuf::traits::Observer;
use ringbuf::producer::Producer;
use std::ops::DerefMut;
use crate::{ActorIdentity, GraphLiveliness, GraphLivelinessState, Rx, RxBundle, RxCore, SendSaturation, SteadyCommander, Tx, TxBundle};
use crate::actor_builder::NodeTxRx;
use crate::core_tx::TxCore;
use crate::distributed::steady_stream::{Defrag, StreamItem, StreamRxBundle, StreamTxBundle};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::ActorMetaData;
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::util::logger;
use crate::yield_now::yield_now;

/// Context for managing actor state and interactions within the Steady framework.
pub struct SteadyContext {
    pub(crate) ident: ActorIdentity,
    pub(crate) instance_id: u32,
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
    pub(crate) show_thread_info: bool
}

impl Clone for SteadyContext {
    fn clone(&self) -> Self {
        SteadyContext {
            ident: self.ident.clone(),
            instance_id: self.instance_id,
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
            show_thread_info: self.show_thread_info
        }
    }
}

impl SteadyCommander for SteadyContext {

    /// Initializes the logger with the specified log level.
    fn loglevel(&self, loglevel: &str) {
        let _ = logger::initialize_with_level(loglevel);
    }

    /// No op, and only relays stats upon the LocalMonitor instance
    fn relay_stats_smartly(&mut self) {
        //do nothing this is only implemented for the monitor
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
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_stop_requested(&self) -> bool {
        self.is_liveliness_in(&[ GraphLivelinessState::StopRequested ])
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
    ///
    /// # Asynchronous
    async fn wait_shutdown_or_avail_units_bundle<T>(
        &self,
        this: &mut RxBundle<'_, T>,
        avail_count: usize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.shared_wait_shutdown_or_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
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
    ///
    /// # Asynchronous
    async fn wait_closed_or_avail_units_bundle<T>(
        &self,
        this: &mut RxBundle<'_, T>,
        avail_count: usize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.shared_wait_closed_or_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }


    async fn wait_shutdown_or_vacant_units_stream<S: StreamItem>(&self
                          , this: &mut StreamTxBundle<'_, S>, vacant: (usize, usize), ready_channels: usize) -> bool
    {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for tx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = tx.item_channel.shared_wait_shutdown_or_vacant_units(vacant.0).await
                                     && tx.payload_channel.shared_wait_shutdown_or_vacant_units(vacant.1).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
                break;
            }
        }

        result.load(Ordering::Relaxed)
    }

    async fn wait_closed_or_avail_message_stream<S: StreamItem>(&self
                                                                , this: &mut StreamRxBundle<'_, S>
                                                                , avail_count: usize
                                                                , ready_channels: usize) -> bool
    {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.item_channel.shared_wait_closed_or_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
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
    async fn wait_avail_units_bundle<T>(
        &self,
        this: &mut RxBundle<'_, T>,
        avail_count: usize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for rx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = rx.shared_wait_avail_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
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
    async fn wait_shutdown_or_vacant_units_bundle<T>(
        &self,
        this: &mut TxBundle<'_, T>,
        avail_count: usize,
        ready_channels: usize,
    ) -> bool
    {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        // Push futures into the FuturesUnordered collection
        for tx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = tx.shared_wait_shutdown_or_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;

        // Poll futures concurrently
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
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
    async fn wait_vacant_units_bundle<T>(
        &self,
        this: &mut TxBundle<'_, T>,
        avail_count: usize,
        ready_channels: usize,
    ) -> bool {
        let count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let mut futures = FuturesUnordered::new();

        for tx in this.iter_mut().take(count_down) {
            let local_r = result.clone();
            futures.push(async move {
                let bool_result = tx.shared_wait_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            });
        }

        let mut completed = 0;
        while let Some(_) = futures.next().await {
            completed += 1;
            if completed >= count_down {
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
    /// The number of messages peeked and stored in `elems`.
    ///
    /// # Type Constraints
    /// - `T`: Must implement `Copy`.
    ///
    /// # Asynchronous
    async fn peek_async_slice<T>(&self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
    where
        T: Copy
    {
        this.shared_peek_async_slice(wait_for_count, elems).await
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
    async fn peek_async_iter<'a, T>(&'a self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item=&'a T> + 'a {
        this.shared_peek_async_iter(wait_for_count).await
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
    fn avail_units<T: RxCore>(&self, this: &mut T) -> usize {
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
        this.shared_peek_async().await
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
        T: Copy
    {
        this.shared_send_slice_until_full(slice)
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
    fn try_send<T: TxCore>(&mut self, this: &mut T, msg: T::MsgIn<'_>) -> Result<(), T::MsgOut> {
        match this.shared_try_send(msg) {
            Ok(d) => Ok(()),
            Err(msg) => Err(msg),
        }
    }



    // fn try_stream_send(&mut self, this: &mut StreamTxBundle<'_, StreamSimpleMessage>
    //                    , stream_id: i32
    //                    , payload: &[u8]) -> Result<(), StreamSimpleMessage>{
    //     debug_assert!(stream_id>= this[0].stream_id);
    //     debug_assert!(stream_id<= this[this.len()-1].stream_id);
    //     let idx:usize = (stream_id - this[0].stream_id) as usize;
    //     let control = StreamSimpleMessage::new(payload.len() as i32);
    //     if this[idx].payload_channel.vacant_units()>= payload.len()
    //        && this[idx].item_channel.vacant_units()>= 1 {
    //         let count = this[idx].payload_channel.shared_send_slice_until_full(payload);
    //         debug_assert_eq!(count, payload.len());
    //         let result = this[idx].item_channel.shared_try_send(control);
    //         debug_assert!(result.is_ok());
    //         Ok(())
    //     } else {
    //         Err(control)
    //     }
    // }

    fn flush_defrag_messages<S: StreamItem>(&mut self
                                            , out_item: &mut Tx<S>
                                            , out_data: &mut Tx<u8>
                                            , defrag: &mut Defrag<S>) -> Option<i32> {


        //slice messages
        let (items_a, items_b) = defrag.ringbuffer_items.1.as_slices();
        //warn!("on ring buffer items {} {}",items_a.len(),items_b.len());
        let msg_count = out_item.tx.vacant_len().min(items_a.len()+items_b.len());
        let msg_count = msg_count.min(50000); //hack test must be smaller than our channel!!!
        let items_len_a = msg_count.min(items_a.len()) as usize;
        let items_len_b = msg_count - items_len_a;
        //sum messages length
        let total_bytes_a = items_a[0..items_len_a].iter().map(|x| x.length()).sum::<i32>() as usize;
        let total_bytes_b = items_b[0..items_len_b].iter().map(|x| x.length()).sum::<i32>() as usize;
        let total_bytes = total_bytes_a + total_bytes_b;
        //warn!("push one large group {} {}",msg_count,total_bytes);
        if total_bytes <= out_data.tx.vacant_len() {
            //move this slice to the tx
            let (payload_a, payload_b) = defrag.ringbuffer_bytes.1.as_slices();
            // warn!("tx payload slices {} {}",payload_a.len(),payload_b.len());

            let len_a = total_bytes.min(payload_a.len()) as usize;
            out_data.tx.push_slice(&payload_a[0..len_a]);
            let len_b = total_bytes - len_a;
            if len_b > 0 {
                out_data.tx.push_slice(&payload_b[0..len_b]);
            }
            unsafe {
                defrag.ringbuffer_bytes.1.advance_read_index(total_bytes)
            }
            //move the messages
            //warn!("tx push slice {} {}",len_a,len_b);
            out_item.tx.push_slice(&items_a[0..items_len_a]);
            if items_len_b > 0 {
                out_item.tx.push_slice(&items_b[0..items_len_b]);
            }
            unsafe {
                defrag.ringbuffer_items.1.advance_read_index(msg_count)
            }
        } else {
            //skipped no room
            warn!("no room skipped");
            return Some(defrag.session_id); //try again later and do not pick up more
        }
        None
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
    fn vacant_units<T: TxCore>(&self, this: &mut T) -> usize {
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
        F: Future,
    {
        let one_down = &mut self.oneshot_shutdown.lock().await;
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
        let one_down = &mut self.oneshot_shutdown.lock().await;
        if !one_down.is_terminated() {
            let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
            let run_duration = now_nanos - self.last_periodic_wait.load(Ordering::Relaxed);
            let remaining_duration = duration_rate.saturating_sub(Duration::from_nanos(run_duration));

            let mut operation = &mut Delay::new(remaining_duration).fuse();
            let result = select! {
                _= &mut one_down.deref_mut() => false,
                _= operation => true
             };
            self.last_periodic_wait.store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::Relaxed);
            result
        } else {
            false
        }
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
    async fn send_async<T>(&mut self, this: &mut Tx<T>, a: T, saturation: SendSaturation) -> Result<(), T> {
        this.shared_send_async(a, self.ident, saturation).await
    }



    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        this.shared_try_take().map(|(d,m)|m)
    }


    // fn take_stream_slice<const LEN:usize, S: StreamItem>(&mut self, this: &mut StreamRx<S>, target: &mut [StreamData<S>; LEN]) -> usize {
    //     //count total items we will take
    //     let total_items = LEN.min(this.item_channel.avail_units());
    //
    //     //item backing arrays plus specific lengths
    //     let (item_a,item_b) = this.item_channel.rx.as_slices();
    //     let item_a_len = total_items.min(item_a.len());
    //     let item_b_len = total_items-item_a_len;
    //
    //     // all bytes consumed by all these items
    //     let mut total_bytes = 0;
    //
    //     let (payload_a,payload_b) = this.payload_channel.rx.as_slices();
    //     let mut first_payload_block = true; //start with first block
    //     let mut payload_index = 0; //payload index for current active payload
    //
    //     let mut item_target_index = 0;
    //     for i in 0..item_a_len {
    //         let item = item_a[i];
    //         total_bytes += item.length();
    //
    //         if first_payload_block {
    //             let next_payload = payload_index+item.length();
    //             if next_payload <= payload_a.len() as i32 { //normal case
    //                 target[item_target_index] = StreamData::new(item, payload_a[payload_index as usize..next_payload as usize].into());
    //                 payload_index = next_payload;
    //             } else {//rare case where we span
    //                 let a_len = item.length() as usize -(payload_a.len() as usize - payload_index as usize) as usize;
    //                 let b_len = item.length() as usize - a_len as usize ;
    //                 let mut vec:Vec<u8> = Vec::with_capacity(item.length() as usize);
    //                 vec.put_slice(&payload_a[payload_index as usize ..(payload_index as usize + a_len) as usize]); //TODO: must not be item index..
    //                 vec.put_slice(&payload_b[0 as usize ..(0+b_len) as usize]);
    //                 target[item_target_index] = StreamData::new(item, vec.into());
    //                 first_payload_block=false;
    //                 payload_index= b_len as i32;
    //             }
    //         } else { //normal case
    //             let next_payload = payload_index+item.length();
    //             target[item_target_index] = StreamData::new(item, payload_b[payload_index as usize ..next_payload as usize].into());
    //             payload_index = next_payload;
    //         }
    //         item_target_index += 1;
    //     }
    //     for i in 0..item_b_len {
    //         let item = item_b[i];
    //         total_bytes += item.length();
    //
    //         if first_payload_block {
    //             let next_payload = payload_index+item.length();
    //             if next_payload <= payload_a.len() as i32 { //normal case
    //                 target[item_target_index] = StreamData::new(item, payload_a[payload_index as usize .. next_payload as usize].into());
    //                 payload_index = next_payload;
    //             } else {//rare case where we span
    //                 let a_len = item.length() as usize -(payload_a.len() as usize - payload_index as usize) as usize;
    //                 let b_len = item.length() as usize - a_len;
    //                 let mut vec:Vec<u8> = Vec::with_capacity(item.length() as usize);
    //                 vec.put_slice(&payload_a[payload_index as usize..(payload_index as usize + a_len) as usize]); //TODO: must not be item index..
    //                 vec.put_slice(&payload_b[0 as usize..(0+b_len) as usize]);
    //                 target[item_target_index] = StreamData::new(item, vec.into());
    //                 first_payload_block=false;
    //                 payload_index= b_len as i32;
    //             }
    //         } else { //normal case
    //             let next_payload = payload_index+item.length();
    //             target[item_target_index] = StreamData::new(item, payload_b[payload_index as usize..next_payload as usize].into());
    //             payload_index = next_payload;
    //         }
    //         item_target_index += 1;
    //     }
    //
    //     this.payload_channel.shared_advance_index(total_bytes as usize);
    //     this.item_channel.shared_advance_index(total_items as usize);
    //     return total_items;
    // }
    //
    //
    // fn try_take_stream<S: StreamItem>(&mut self, this: &mut StreamRx<S>) -> Option<StreamData<S>> {
    //     let item = self.try_take(&mut this.item_channel);
    //     if let Some(item) = item {
    //         let mut target = Vec::with_capacity(item.length() as usize);
    //         self.take_slice(&mut this.payload_channel, &mut target);
    //         let box_slice = target.into_boxed_slice();
    //         Some(StreamData::new(item, box_slice))
    //     } else {
    //         None
    //     }
    // }


    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
    }

    fn advance_read_index<T>(&mut self, this: &mut Rx<T>, count: usize) -> usize {
        this.shared_advance_index(count)
    }

    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_shutdown_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        this.shared_wait_shutdown_or_avail_units(count).await
    }
    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_closed_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        this.shared_wait_closed_or_avail_units(count).await
    }
    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available
    async fn wait_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        this.shared_wait_avail_units(count).await
    }
    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_shutdown_or_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool {
        this.shared_wait_shutdown_or_vacant_units(count).await
    }
    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available
    async fn wait_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool {
        this.shared_wait_vacant_units(count).await
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
                //wait until we are in a running state
                thread::yield_now();
            }
        }
    }
    /// Requests a graph stop for the actor.
    ///
    /// # Returns
    /// `true` if the request was successful, `false` otherwise.
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