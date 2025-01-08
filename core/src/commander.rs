
use std::future::Future;
use std::time::{Duration, Instant};
use futures_util::future::{FusedFuture};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use futures_util::{select, FutureExt};
use futures_timer::Delay;
use std::thread;
use std::ops::DerefMut;
use crate::{steady_config, ActorIdentity, GraphLivelinessState, LocalMonitor, Rx, RxBundle, SendSaturation, SteadyContext, Tx, TxBundle};
use crate::graph_testing::SideChannelResponder;
use crate::monitor::{RxMetaData, TxMetaData};
use crate::monitor_telemetry::SteadyTelemetry;
use crate::steady_rx::RxDef;
use crate::steady_tx::TxDef;
use crate::telemetry::setup;
use crate::yield_now::yield_now;
use futures::stream::{FuturesUnordered, StreamExt};
use crate::distributed::aqueduct::{AqueductFragment, AqueductRx, AqueductTx, SteadyAqueductTx};
use crate::util::logger;

impl SteadyContext {
    /// Converts the context into a local monitor.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver monitors.
    /// - `tx_mons`: Array of transmitter monitors.
    ///
    /// # Returns
    /// A `LocalMonitor` instance.
    pub fn into_monitor<const RX_LEN: usize, const TX_LEN: usize>(
        self,
        rx_mons: [&dyn RxDef; RX_LEN],
        tx_mons: [&dyn TxDef; TX_LEN],
    ) -> LocalMonitor<RX_LEN, TX_LEN> {
        let rx_meta = rx_mons
            .iter()
            .map(|rx| rx.meta_data())
            .collect::<Vec<_>>()
            .try_into()
            .expect("Length mismatch should never occur");

        let tx_meta = tx_mons
            .iter()
            .map(|tx| tx.meta_data())
            .collect::<Vec<_>>()
            .try_into()
            .expect("Length mismatch should never occur");

        self.into_monitor_internal(rx_meta, tx_meta)
    }
    /// Internal method to convert the context into a local monitor.
    ///
    /// # Parameters
    /// - `rx_mons`: Array of receiver metadata.
    /// - `tx_mons`: Array of transmitter metadata.
    ///
    /// # Returns
    /// A `LocalMonitor` instance.
    pub fn into_monitor_internal<const RX_LEN: usize, const TX_LEN: usize>(
        self,
        rx_mons: [RxMetaData; RX_LEN],
        tx_mons: [TxMetaData; TX_LEN],
    ) -> LocalMonitor<RX_LEN, TX_LEN> {
        let (send_rx, send_tx, state) = if (self.frame_rate_ms > 0)
            && (steady_config::TELEMETRY_HISTORY || steady_config::TELEMETRY_SERVER) {
            let mut rx_meta_data = Vec::new();
            let mut rx_inverse_local_idx = [0; RX_LEN];
            rx_mons.iter().enumerate().for_each(|(c, md)| {
                assert!(md.0.id < usize::MAX);
                rx_inverse_local_idx[c] = md.0.id;
                rx_meta_data.push(md.0.clone());
            });

            let mut tx_meta_data = Vec::new();
            let mut tx_inverse_local_idx = [0; TX_LEN];
            tx_mons.iter().enumerate().for_each(|(c, md)| {
                assert!(md.0.id < usize::MAX);
                tx_inverse_local_idx[c] = md.0.id;
                tx_meta_data.push(md.0.clone());
            });

            setup::construct_telemetry_channels(
                &self,
                rx_meta_data,
                rx_inverse_local_idx,
                tx_meta_data,
                tx_inverse_local_idx,
            )
        } else {
            (None, None, None)
        };

        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry: SteadyTelemetry {
                send_rx,
                send_tx,
                state,
            },
            last_telemetry_send: Instant::now(),
            ident: self.ident,
            is_in_graph: self.is_in_graph,
            runtime_state: self.runtime_state,
            oneshot_shutdown: self.oneshot_shutdown,
            last_periodic_wait: Default::default(),
            actor_start_time: self.actor_start_time,
            node_tx_rx: self.node_tx_rx.clone(),
            frame_rate_ms: self.frame_rate_ms,
            args: self.args,
            show_thread_info: self.show_thread_info,
            team_id: self.team_id,
            is_running_iteration_count: 0,
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
    fn try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T> {
        this.shared_try_send(msg)
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


    fn try_aqueduct_send<'a>(&mut self, tx: &mut AqueductTx, stream_id: i32, payload: &'a[u8]) -> Result<(), &'a[u8]> {

        debug_assert!(stream_id >= tx.stream_first);
        debug_assert!(stream_id < tx.stream_first + tx.stream_count);

        if tx.payload_channel.vacant_units()>=payload.len() && tx.control_channel.vacant_units()>=1 {
            let len = self.send_slice_until_full(&mut tx.payload_channel, &payload);
            debug_assert!(len==payload.len());
            let result = self.try_send(&mut tx.control_channel, AqueductFragment::simple_outgoing(stream_id, payload.len() as i32));
            debug_assert!(result.is_err());
            Ok(())
        } else {
            Err(payload)
        }
    }

    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    fn try_take<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_try_take()
    }
    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
    }

    ///  Waits until we have the specified message count and byte count room on this target aqueduct
    async fn wait_shutdown_or_vacant_aqueduct(&self, this: &mut AqueductTx, message_count: usize, bytes_count: usize) -> bool {
        this.payload_channel.shared_wait_shutdown_or_vacant_units(bytes_count).await
        &&
        this.control_channel.shared_wait_shutdown_or_vacant_units(message_count).await
    }

    /// Waits until we have available count of messages
    async fn wait_shutdown_or_avail_aqueduct(&self, this: &mut AqueductRx, avail_count: usize) -> bool {
        this.control_channel.shared_wait_shutdown_or_avail_units(avail_count).await
    }

    /// Waits until we have available count of messages
    async fn wait_closed_or_avail_aqueduct(&self, this: &mut AqueductRx, avail_count: usize) -> bool {
        this.control_channel.shared_wait_closed_or_avail_units(avail_count).await
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


/// NOTE this trait is passed into actors and actors are tied to a single thread. As a result
///      we need not worry about these methods needing Send. We also know that T will come
///      from other actors so we can assume that T is Send + Sync
#[allow(async_fn_in_trait)]
pub trait SteadyCommander {

    /// try send aqueduct message
    fn try_aqueduct_send<'a>(&mut self, tx: &mut AqueductTx, stream_id: i32, payload: &'a[u8]) -> Result<(), &'a[u8]>;

    /// set log level for the entire application
    fn loglevel(&self, loglevel: &str);

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// This method holds the data if it is called more frequently than the collector can consume the data.
    /// It is designed for use in tight loops where telemetry data is collected frequently.
    fn relay_stats_smartly(&mut self);

    /// Triggers the transmission of all collected telemetry data to the configured telemetry endpoints.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// This method ignores the last telemetry send time and may overload the telemetry if called too frequently.
    /// It is designed for use in low-frequency telemetry collection scenarios, specifically cases
    /// when we know the actor will do a blocking wait for a long time and we want relay what we have before that.
    fn relay_stats(&mut self);

    /// Periodically relays telemetry data at a specified rate.
    /// NOTE: This does NOTHING unless called on the LocalMonitor instance
    ///
    /// # Parameters
    /// - `duration_rate`: The interval at which telemetry data should be sent.
    ///
    /// # Returns
    /// `true` if the full waiting duration was completed without interruption.
    /// `false` if a shutdown signal was detected during the waiting period.
    ///
    /// # Asynchronous
    async fn relay_stats_periodic(&mut self, duration_rate: Duration) -> bool;


    /// Checks if the liveliness state matches any of the target states.
    ///
    /// # Parameters
    /// - `target`: A slice of `GraphLivelinessState`.
    ///
    /// # Returns
    /// `true` if the liveliness state matches any target state, otherwise `false`.
    fn is_liveliness_in(&self, target: &[GraphLivelinessState]) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_building(&self) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_running(&self) -> bool;
    /// Convenience methods for checking the liveliness state of the actor.
    fn is_liveliness_stop_requested(&self) -> bool;

    /// wait for outgoing room on the aqueduct or shutdown signal
    async fn wait_shutdown_or_vacant_aqueduct(&self, this: &mut AqueductTx, message_count: usize, bytes_count: usize) -> bool;

    /// wait for messages on aqueduct or shutdown signal
    async fn wait_shutdown_or_avail_aqueduct(&self, this: &mut AqueductRx, avail_count: usize) -> bool;

    /// wait for message on aqueduct or closed signal
    async fn wait_closed_or_avail_aqueduct(&self, this: &mut AqueductRx, avail_count: usize) -> bool;

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
    async fn wait_shutdown_or_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    ;
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
    async fn wait_closed_or_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    ;
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
    async fn wait_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    ;
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
    ;
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
    ;

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
    async fn wait_periodic(&self, duration_rate: Duration) -> bool;

    /// Waits for a future to complete or until a shutdown signal is received.
    ///
    /// # Parameters
    /// - `fut`: The future to wait for.
    ///
    /// # Returns
    /// `true` if the future completed, `false` if a shutdown signal was received.
    async fn wait_future_void<F>(&self, fut: F) -> bool
    where
        F: FusedFuture<Output = ()> + 'static + Send + Sync;

    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_shutdown_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool;
    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_closed_or_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool;
    /// Waits until the specified number of available units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available
    async fn wait_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool;
    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    async fn wait_shutdown_or_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool;
    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available
    async fn wait_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool;
    /// Waits until shutdown
    ///
    /// # Returns
    /// true
    async fn wait_shutdown(&self) -> bool;
    
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
        T: Copy;
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
        T: Copy;
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
    ;
    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&'a T>
    ;
    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item=&'a T> + 'a;
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
    async fn peek_async_iter<'a, T>(&'a self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item=&'a T> + 'a;
    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    fn is_empty<T>(&self, this: &mut Rx<T>) -> bool;
    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    fn avail_units<T>(&self, this: &mut Rx<T>) -> usize;
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
    ;
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
        T: Copy;
    /// Sends messages from an iterator to the Tx channel until it is full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    fn send_iter_until_full<T, I: Iterator<Item=T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize;
    /// Attempts to send a single message to the Tx channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    fn try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T>;
    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    fn is_full<T>(&self, this: &mut Tx<T>) -> bool;
    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    fn vacant_units<T>(&self, this: &mut Tx<T>) -> usize;
    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    async fn wait_empty<T>(&self, this: &mut Tx<T>) -> bool;
    /// Takes messages into an iterator.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the taken messages.
    fn take_into_iter<'a, T: Sync + Send>(&mut self, this: &'a mut Rx<T>) -> impl Iterator<Item=T> + 'a;
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
    ;

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
    async fn send_async<T>(&mut self, this: &mut Tx<T>, a: T, saturation: SendSaturation) -> Result<(), T>;
    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    fn try_take<T>(&mut self, this: &mut Rx<T>) -> Option<T>;
    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    async fn take_async<T>(&mut self, this: &mut Rx<T>) -> Option<T>;
    
    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    async fn wait(&self, duration: Duration);

    /// Yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    async fn yield_now(&self);
    

    /// Returns a side channel responder if available.
    ///
    /// # Returns
    /// An `Option` containing a `SideChannelResponder` if available.
    fn sidechannel_responder(&self) -> Option<SideChannelResponder>;
    /// Checks if the actor is running, using a custom accept function.
    ///
    /// # Parameters
    /// - `accept_fn`: The custom accept function to check the running state.
    ///
    /// # Returns
    /// `true` if the actor is running, `false` otherwise.
    fn is_running<F: FnMut() -> bool>(&mut self, accept_fn: F) -> bool;
    /// Requests a graph stop for the actor.
    ///
    /// # Returns
    /// `true` if the request was successful, `false` otherwise.
    fn request_graph_stop(&self) -> bool;
    /// Retrieves the actor's arguments, cast to the specified type.
    ///
    /// # Returns
    /// An `Option<&A>` containing the arguments if available and of the correct type.
    fn args<A: Any>(&self) -> Option<&A>;
    /// Retrieves the actor's identity.
    ///
    /// # Returns
    /// An `ActorIdentity` representing the actor's identity.
    fn identity(&self) -> ActorIdentity;

}