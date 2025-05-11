use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use log::{error, warn};
use std::time::{Duration, Instant};
use std::sync::{Arc, OnceLock};
use parking_lot::RwLock;
use futures_util::lock::{Mutex, MutexGuard};
use futures::channel::oneshot;
use std::any::{Any};
use std::error::Error;
use futures_util::future::{FusedFuture};
use futures_timer::Delay;
use futures_util::{select, FutureExt, StreamExt};
use std::future::Future;
use futures::executor;
use num_traits::Zero;
use std::ops::DerefMut;
use aeron::aeron::Aeron;
use futures_util::stream::FuturesUnordered;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use ringbuf::producer::Producer;
use crate::monitor::{DriftCountIterator, FinallyRollupProfileGuard, CALL_BATCH_READ, CALL_BATCH_WRITE, CALL_OTHER, CALL_SINGLE_READ, CALL_SINGLE_WRITE, CALL_WAIT};
use crate::{simulate_edge, yield_now, ActorIdentity, Graph, GraphLiveliness, GraphLivelinessState, Rx, RxCoreBundle, SendSaturation, SteadyCommander, Tx, TxCoreBundle, MONITOR_NOT};
use crate::actor_builder::NodeTxRx;
use crate::commander::SendOutcome;
use crate::core_rx::RxCore;
use crate::core_tx::TxCore;
use crate::distributed::distributed_stream::{Defrag, StreamItem};
use crate::graph_testing::SideChannelResponder;
use crate::monitor_telemetry::SteadyTelemetry;
use crate::SendOutcome::Blocked;
use crate::simulate_edge::{ IntoSimRunner};
use crate::steady_config::{CONSUMED_MESSAGES_BY_COLLECTOR, REAL_CHANNEL_LENGTH_TO_COLLECTOR};
use crate::steady_rx::RxDone;
use crate::steady_tx::TxDone;
use crate::telemetry::setup;
use crate::telemetry::setup::send_all_local_telemetry_async;
use crate::util::steady_logger;

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
    pub(crate) aeron_meda_driver: OnceLock<Option<Arc<Mutex<Aeron>>>>,
    pub use_internal_behavior: bool
}

/// Implementation of `LocalMonitor`.
impl<const RXL: usize, const TXL: usize> LocalMonitor<RXL, TXL> {

    // fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
    //     Graph::aeron_media_driver_internal(&self.aeron_meda_driver)
    // }

    #[allow(async_fn_in_trait)]
    pub async fn simulated_behavior(self, sims: Vec<&dyn IntoSimRunner<Self>>
    ) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<Self>(self,sims).await
    }

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

    pub(crate) fn validate_capacity_rx<T: RxCore>(this: &mut T, count: usize) -> usize {
        if count <= this.shared_capacity() {
            count
        } else {
            let capacity = this.shared_capacity();
            if this.log_perodic() {
                warn!("wait_*: count {} exceeds capacity {}, reduced to capacity", count, capacity);
            }
            capacity
        }
    }

    // pub(crate) fn validate_capacity_tx<T: TxCore>(this: &mut T, count: usize) -> usize {
    //     if count <= this.shared_capacity() {
    //         count
    //     } else {
    //         let capacity = this.shared_capacity();
    //         if this.log_perodic() {
    //             warn!("wait_*: count {} exceeds capacity {}, reduced to capacity", count, capacity);
    //         }
    //         capacity
    //     }
    // }

    //TODO: check usage and add to the SteadyState
    pub(crate) async fn internal_wait_shutdown(&self) -> bool {
        let one_shot = &self.oneshot_shutdown;
        let mut guard = one_shot.lock().await;

        if !guard.is_terminated() {
            error!("expecting hang A");

            let _ = guard.deref_mut().await;
            error!("expecting hang B");

        }
        true
    }

    // pub(crate) fn dynamic_event_count(&mut self
    //                                   , local_index: usize //this.local_index
    //                                   , id: usize //this.channel_meta_data.id
    //                                   , count: isize) -> usize {
    //     if let Some(ref mut tel) = self.telemetry.send_rx {
    //         tel.process_event(local_index, id, count)
    //     } else {
    //         MONITOR_NOT
    //     }
    // }

}

impl<const RX_LEN: usize, const TX_LEN: usize> SteadyCommander for LocalMonitor<RX_LEN, TX_LEN> {

    fn aeron_media_driver(&self) -> Option<Arc<Mutex<Aeron>>> {
        Graph::aeron_media_driver_internal(&self.aeron_meda_driver)
    }

    async fn simulated_behavior(self, sims: Vec<&dyn IntoSimRunner<Self>>
    ) -> Result<(), Box<dyn Error>> {
        simulate_edge::simulated_behavior::<LocalMonitor<RX_LEN, TX_LEN>>(self, sims).await
    }

    /// set loglevel for the application
    fn loglevel(&self, loglevel: crate::LogLevel) {
        let _ = steady_logger::initialize_with_level(loglevel);
    }

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
    /// Timeout if shutdown has been called
    fn is_liveliness_shutdown_timeout(&self) -> Option<Duration> {
        let liveliness = self.runtime_state.read();
        liveliness.shutdown_timeout
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

        if let Some(ref mut tel) = self.telemetry.send_rx {
            this.telemetry_inc(RxDone::Normal(done), tel);
        } else {
            this.monitor_not();
        };

        done
    }



    fn advance_read_index<T>(&mut self, this: &mut Rx<T>, count: usize) -> usize {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }
        let done = this.shared_advance_index(count);
        if let Some(ref mut tel) = self.telemetry.send_rx {
            this.telemetry_inc(RxDone::Normal(done), tel);
        } else {
            this.monitor_not();
        };

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
    async fn peek_async<'a, T: RxCore>(&'a self, this: &'a mut T) -> Option<T::MsgPeek<'a>>
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
            tel.process_event(this.local_index, this.channel_meta_data.meta_data.id, done as isize)
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
            tel.process_event(this.local_index, this.channel_meta_data.meta_data.id, done as isize)
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
    /// A SendOutcome<T>
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
                };
                SendOutcome::Success
            }
            Err(sensitive) => SendOutcome::Blocked(sensitive),
        }
    }



    fn flush_defrag_messages<S: StreamItem>(
        &mut self,
        out_item: &mut Tx<S>,
        out_data: &mut Tx<u8>,
        defrag: &mut Defrag<S>,
    ) -> (u32, u32, Option<i32>) {
        // Record this was called
        if let Some(ref mut st) = self.telemetry.state {
            let _ = st.calls[CALL_BATCH_WRITE].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| {
                Some(f.saturating_add(1))
            });
        }
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

        // Record telemetry
        if msg_count > 0 {
            if let Some(ref mut tel) = self.telemetry.send_tx {
                out_item.local_index = tel.process_event(
                    out_item.local_index,
                    out_item.channel_meta_data.meta_data.id,
                    msg_count as isize,
                );
                out_data.local_index = tel.process_event(
                    out_data.local_index,
                    out_data.channel_meta_data.meta_data.id,
                    total_bytes as isize,
                );
            } else {
                out_item.local_index = MONITOR_NOT;
                out_data.local_index = MONITOR_NOT;
            }
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
        let _guard = self.start_profile(CALL_WAIT);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 {
                this.shared_vacant_units()==this.shared_capacity()
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

         if let Some(ref mut tel) = self.telemetry.send_rx {
             let drift = this.iterator_count_drift.load(Ordering::Relaxed);
             this.iterator_count_drift.store(0, Ordering::Relaxed);
             let done_count = RxDone::Normal((units as isize + drift) as usize);
             this.telemetry_inc(done_count, tel);
         } else {
             this.monitor_not();
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
                    this.telemetry_inc(done_one, tel); } else { this.monitor_not(); };
                SendOutcome::Success
            }
            SendOutcome::Blocked(sensitive) => SendOutcome::Blocked(sensitive),
        }

    }


    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    fn try_take<T: RxCore>(&mut self, this: &mut T) -> Option<T::MsgOut> {
        if let Some(ref st) = self.telemetry.state {
            let _ = st.calls[CALL_SINGLE_READ].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| Some(f.saturating_add(1)));
        }

        match this.shared_try_take() {
            Some((done_count,msg)) => {
                if let Some(ref mut tel) = self.telemetry.send_rx {
                   this.telemetry_inc(done_count, tel);
                } else {
                   this.monitor_not();
                };
                Some(msg)
            },
            None => None
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
                if let Some(ref mut tel) = self.telemetry.send_rx {
                    this.telemetry_inc(RxDone::Normal(1), tel);
                } else {
                    this.monitor_not();
                };
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
    /// `true` if the units are available, otherwise `false` if closed channel.
    ///
    /// # Asynchronous
    async fn wait_avail<T: RxCore>(&self, this: &mut T, count: usize) -> bool {
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




    /// Asynchronously waits until at least a specified number of units are vacant in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Asynchronous
    async fn wait_vacant<T: TxCore>(&self, this: &mut T, count: T::MsgSize) -> bool {
        let _guard = self.start_profile(CALL_WAIT);

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

    /// uses opposite boolean as others since we are asking for shutdown
    /// returns true upon shutdown detection
    async fn wait_shutdown(&self) -> bool {
        let _guard = self.start_profile(CALL_OTHER);
        if self.telemetry.is_dirty() {
            let remaining_micros = self.telemetry_remaining_micros();
            if remaining_micros <= 0 && self.is_liveliness_running() {
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
            error!("expecting hang 6.1");

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

    async fn wait_vacant_bundle<T: TxCore>(&self, this: &mut TxCoreBundle<'_, T>, count: T::MsgSize, ready_channels: usize) -> bool {
        let _guard = self.start_profile(CALL_OTHER);

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
        let _guard = self.start_profile(CALL_OTHER);

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

}

impl<const RX_LEN: usize, const TX_LEN: usize> LocalMonitor<RX_LEN, TX_LEN> {
    fn telemetry_remaining_micros(&self) -> i64 {
       (1000i64 * self.frame_rate_ms as i64) - (self.last_telemetry_send.elapsed().as_micros() as i64
       * CONSUMED_MESSAGES_BY_COLLECTOR as i64)
    }
}