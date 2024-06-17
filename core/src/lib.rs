//! # Steady State Core - Easy Performant Async
//!  Steady State is a high performance, easy to use, actor based framework for building concurrent applications in Rust.
//!  Guarantee your SLA with telemetry, alerts and Prometheus.
//!  Build low latency high volume solutions.

pub(crate) mod telemetry {
    //! Telemetry module for monitoring and collecting metrics.
    pub(crate) mod metrics_collector;
    pub(crate) mod metrics_server;
    pub(crate) mod setup;
}

pub(crate) mod serialize {
    //! Serialization module for efficient data packing.
    pub(crate) mod byte_buffer_packer;
    pub(crate) mod fast_protocol_packed;
}
pub(crate) mod channel_stats;
pub(crate) mod config;
pub(crate) mod dot;
pub mod monitor;

pub mod channel_builder;
pub mod util;
pub mod install {
    //! Installation module for setting up various deployment methods.
    pub mod serviced;
    pub mod local_cli;
    pub mod container;
}
pub mod actor_builder;
mod actor_stats;
pub mod graph_testing;
mod graph_liveliness;
mod loop_driver;
mod yield_now;
mod abstract_executor;
mod test_panic_capture;
mod steady_telemetry;
pub mod steady_tx;
pub mod steady_rx;



pub use graph_testing::GraphTestResult;
pub use monitor::LocalMonitor;
pub use channel_builder::Rate;
pub use channel_builder::Filled;
pub use actor_builder::MCPU;
pub use actor_builder::Work;
pub use actor_builder::Percentile;
pub use graph_liveliness::*;
pub use install::serviced::*;
pub use loop_driver::wrap_bool_future;
pub use nuclei::spawn_local;
pub use nuclei::spawn_blocking;
pub use steady_rx::Rx;
pub use steady_tx::Tx;
pub use steady_rx::SteadyRxBundleTrait;
pub use steady_tx::SteadyTxBundleTrait;
pub use steady_rx::RxBundleTrait;
pub use steady_tx::TxBundleTrait;

use std::any::{Any, type_name};
use std::backtrace::{Backtrace, BacktraceStatus};
use std::time::{Duration, Instant};
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use futures::lock::{Mutex, MutexLockFuture};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use ahash::AHasher;
use log::*;
use channel_builder::{InternalReceiver, InternalSender};

use async_ringbuf::traits::{Consumer, Observer, Producer};
use colored::Colorize;

use actor_builder::ActorBuilder;
use crate::monitor::{ActorMetaData, CALL_OTHER, CALL_SINGLE_READ, ChannelMetaData, RxMetaData, TxMetaData};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;
use futures::*;
use futures::channel::oneshot;
use futures::select;
use futures_timer::Delay;
use futures_util::future::{BoxFuture, FusedFuture, select_all};
use futures_util::lock::MutexGuard;
use num_traits::Zero;
use steady_rx::{RxDef};
use steady_telemetry::SteadyTelemetry;
use steady_tx::{TxDef};
use crate::graph_testing::{SideChannel, SideChannelResponder};
use crate::yield_now::yield_now;
/// Type alias for a thread-safe transmitter (Tx) wrapped in an `Arc` and `Mutex`.
///
/// This type alias simplifies the usage of a transmitter that can be shared across multiple threads.
pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;

/// Type alias for an array of thread-safe transmitters (Tx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of transmitters that can be shared across multiple threads.
pub type SteadyTxBundle<T, const GIRTH: usize> = Arc<[SteadyTx<T>; GIRTH]>;

/// Type alias for a thread-safe receiver (Rx) wrapped in an `Arc` and `Mutex`.
///
/// This type alias simplifies the usage of a receiver that can be shared across multiple threads.
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;

/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of receivers that can be shared across multiple threads.
pub type SteadyRxBundle<T, const GIRTH: usize> = Arc<[SteadyRx<T>; GIRTH]>;

/// Type alias for a vector of `MutexGuard` references to transmitters (Tx).
///
/// This type alias simplifies the usage of a collection of transmitter guards for batch operations.
pub type TxBundle<'a, T> = Vec<MutexGuard<'a, Tx<T>>>;

/// Type alias for a vector of `MutexGuard` references to receivers (Rx).
///
/// This type alias simplifies the usage of a collection of receiver guards for batch operations.
pub type RxBundle<'a, T> = Vec<MutexGuard<'a, Rx<T>>>;

/// Creates a bundle of thread-safe transmitters (Tx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This function takes an array of transmitters and wraps it in an `Arc` for shared ownership.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyTx<T>` with a fixed size (GIRTH).
///
/// # Returns
/// - `SteadyTxBundle<T, GIRTH>`: A bundle of transmitters wrapped in an `Arc`.
pub fn steady_tx_bundle<T, const GIRTH: usize>(internal_array: [SteadyTx<T>; GIRTH]) -> SteadyTxBundle<T, GIRTH> {
    Arc::new(internal_array)
}

/// Creates a bundle of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This function takes an array of receivers and wraps it in an `Arc` for shared ownership.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyRx<T>` with a fixed size (GIRTH).
///
/// # Returns
/// - `SteadyRxBundle<T, GIRTH>`: A bundle of receivers wrapped in an `Arc`.
pub fn steady_rx_bundle<T, const GIRTH: usize>(internal_array: [SteadyRx<T>; GIRTH]) -> SteadyRxBundle<T, GIRTH> {
    Arc::new(internal_array)
}

/// Initialize logging for the steady_state crate.
/// This is a convenience function that should be called at the beginning of main.
pub fn init_logging(loglevel: &str) -> Result<(), Box<dyn std::error::Error>> {
    steady_logging_init(loglevel)
}

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
    pub(crate) node_tx_rx: Option<Arc<Mutex<SideChannel>>>,
    pub(crate) frame_rate_ms: u64,
}

impl SteadyContext {
    /// Get the unique identifier of the actor.
    pub fn id(&self) -> usize {
        self.ident.id
    }

    /// Get the name of the actor.
    pub fn name(&self) -> &str {
        self.ident.name
    }

    /// Check if the actor's liveliness is in the specified state.
    pub(crate) fn is_liveliness_in(&self, target: &[GraphLivelinessState], upon_poison: bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => liveliness.is_in_state(target),
            Err(e) => {
                trace!("Internal error, unable to get liveliness read lock {}", e);
                upon_poison
            }
        }
    }

    /// Update the transmission instance for the given channel.
    pub fn update_tx_instance<T>(&self, target: &mut Tx<T>) {
        target.tx_version.store(self.instance_id, Ordering::SeqCst);
    }

    /// Update the transmission instance for the given bundle of channels.
    pub fn update_tx_instance_bundle<T>(&self, target: &mut TxBundle<T>) {
        target.iter_mut().for_each(|tx| tx.tx_version.store(self.instance_id, Ordering::SeqCst));
    }

    /// Update the reception instance for the given channel.
    pub fn update_rx_instance<T>(&self, target: &mut Rx<T>) {
        target.rx_version.store(self.instance_id, Ordering::SeqCst);
    }

    /// Update the reception instance for the given bundle of channels.
    pub fn update_rx_instance_bundle<T>(&self, target: &mut RxBundle<T>) {
        target.iter_mut().for_each(|rx| rx.tx_version.store(self.instance_id, Ordering::SeqCst));
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
    pub async fn wait_periodic(&self, duration_rate: Duration) -> bool {
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
    pub async fn wait(&self, duration: Duration) {
        let one_down = &mut self.oneshot_shutdown.lock().await;
        if !one_down.is_terminated() {
            select! { _ = one_down.deref_mut() => {}, _ =Delay::new(duration).fuse() => {} }
        }
    }

    /// Yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    pub async fn yield_now(&self) {
        yield_now().await;
    }

    /// Waits for a future to complete or until a shutdown signal is received.
    ///
    /// # Parameters
    /// - `fut`: The future to wait for.
    ///
    /// # Returns
    /// `true` if the future completed, `false` if a shutdown signal was received.
    pub async fn wait_future_void(&self, mut fut: Pin<Box<dyn FusedFuture<Output = ()>>>) -> bool {
        let one_down = &mut self.oneshot_shutdown.lock().await;
        let mut one_fused = one_down.deref_mut().fuse();
        if !one_fused.is_terminated() {
            select! { _ = one_fused => false, _ = fut => true, }
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
    pub async fn send_async<T>(&mut self, this: &mut Tx<T>, a: T, saturation: SendSaturation) -> Result<(), T> {
        #[cfg(debug_assertions)]
        this.direct_use_check_and_warn();
        this.shared_send_async(a, self.ident, saturation).await
    }

    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    pub fn try_take<T>(&self, this: &mut Rx<T>) -> Option<T> {
        this.shared_try_take()
    }

    /// Waits until the specified number of available units are in the receiver.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    pub async fn wait_avail_units<T>(&self, this: &mut Rx<T>, count: usize) -> bool {
        this.shared_wait_avail_units(count).await
    }

    /// Waits until the specified number of vacant units are in the transmitter.
    ///
    /// # Parameters
    /// - `count`: The number of units to wait for.
    ///
    /// # Returns
    /// `true` if the required number of units became available, `false` if the wait was interrupted.
    pub async fn wait_vacant_units<T>(&self, this: &mut Tx<T>, count: usize) -> bool {
        this.shared_wait_vacant_units(count).await
    }

    /// Simulates edge behavior for the actor.
    ///
    /// # Returns
    /// An `Option<SideChannelResponder>` if the simulation is available, otherwise `None`.
    pub fn edge_simulator(&self) -> Option<SideChannelResponder> {
        self.node_tx_rx.as_ref().map(|node_tx_rx| SideChannelResponder::new(node_tx_rx.clone()))
    }

    /// Checks if the actor is running, using a custom accept function.
    ///
    /// # Parameters
    /// - `accept_fn`: The custom accept function to check the running state.
    ///
    /// # Returns
    /// `true` if the actor is running, `false` otherwise.
    #[inline]
    pub fn is_running(&self, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => liveliness.is_running(self.ident, accept_fn),
            Err(e) => {
                error!("Internal error, unable to get liveliness read lock {}", e);
                true
            }
        }
    }

    /// Requests a graph stop for the actor.
    ///
    /// # Returns
    /// `true` if the request was successful, `false` otherwise.
    #[inline]
    pub fn request_graph_stop(&self) -> bool {
        match self.runtime_state.write() {
            Ok(mut liveliness) => {
                liveliness.request_shutdown();
                true
            }
            Err(e) => {
                trace!("Internal error, unable to get liveliness write lock {}", e);
                false //keep running as the default under error conditions
            }
        }
    }

    /// Retrieves the actor's arguments, cast to the specified type.
    ///
    /// # Returns
    /// An `Option<&A>` containing the arguments if available and of the correct type.
    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    /// Retrieves the actor's identity.
    ///
    /// # Returns
    /// An `ActorIdentity` representing the actor's identity.
    pub fn identity(&self) -> ActorIdentity {
        self.ident
    }

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
        let (send_rx, send_tx, state) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {
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
            instance_id: self.instance_id,
            is_in_graph: self.is_in_graph,
            runtime_state: self.runtime_state,
            oneshot_shutdown: self.oneshot_shutdown,
            last_perodic_wait: Default::default(),
            actor_start_time: self.actor_start_time,
            node_tx_rx: self.node_tx_rx.clone(),
            frame_rate_ms: self.frame_rate_ms,

            #[cfg(test)]
            test_count: HashMap::new(),
        }
    }
}

#[macro_export]
macro_rules! into_monitor {
    ($self:expr, [$($rx:expr),*], [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        let rx_meta = [$($rx.meta_data(),)*];
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
    ($self:expr, [$($rx:expr),*], $tx_bundle:expr) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        let rx_meta = [$($rx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, $tx_bundle.meta_data())
    }};
    ($self:expr, $rx_bundle:expr, [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal($rx_bundle.meta_data(), tx_meta)
    }};
    ($self:expr, $rx_bundle:expr, $tx_bundle:expr) => {{
        $self.into_monitor_internal($rx_bundle.meta_data(), $tx_bundle.meta_data())
    }};
    ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*], $($rx_bundle:expr),* ), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        let mut rx_count = [$( { $rx; 1 } ),*].len();
        $(
            rx_count += $rx_bundle.meta_data().len();
        )*
        assert_eq!(rx_count, $rx_channels_to_monitor, "Mismatch in RX channel count");

        let mut tx_count = [$( { $tx; 1 } ),*].len();
        $(
            tx_count += $tx_bundle.meta_data().len();
        )*
        assert_eq!(tx_count, $tx_channels_to_monitor, "Mismatch in TX channel count");

        let mut rx_mon = [RxMetaData::default(); $rx_channels_to_monitor];
        let mut rx_index = 0;
        $(
            rx_mon[rx_index] = $rx.meta_data();
            rx_index += 1;
        )*
        $(
            for meta in $rx_bundle.meta_data() {
                rx_mon[rx_index] = meta;
                rx_index += 1;
            }
        )*

        let mut tx_mon = [TxMetaData::default(); $tx_channels_to_monitor];
        let mut tx_index = 0;
        $(
            tx_mon[tx_index] = $tx.meta_data();
            tx_index += 1;
        )*
        $(
            for meta in $tx_bundle.meta_data() {
                tx_mon[tx_index] = meta;
                tx_index += 1;
            }
        )*

        $self.into_monitor_internal(rx_mon, tx_mon)
    }};
   ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*]), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        let mut rx_count = [$( { $rx; 1 } ),*].len();
        assert_eq!(rx_count, $rx_channels_to_monitor, "Mismatch in RX channel count");

        let mut tx_count = [$( { $tx; 1 } ),*].len();
        $(
            tx_count += $tx_bundle.meta_data().len();
        )*
        assert_eq!(tx_count, $tx_channels_to_monitor, "Mismatch in TX channel count");

        let mut rx_mon = [RxMetaData::default(); $rx_channels_to_monitor];
        let mut rx_index = 0;
        $(
            rx_mon[rx_index] = $rx.meta_data();
            rx_index += 1;
        )*

        let mut tx_mon = [TxMetaData::default(); $tx_channels_to_monitor];
        let mut tx_index = 0;
        $(
            tx_mon[tx_index] = $tx.meta_data();
            tx_index += 1;
        )*
        $(
            for meta in $tx_bundle.meta_data() {
                tx_mon[tx_index] = meta;
                tx_index += 1;
            }
        )*

        $self.into_monitor_internal(rx_mon, tx_mon)
    }};
}

const MONITOR_UNKNOWN: usize = usize::MAX;
const MONITOR_NOT: usize = MONITOR_UNKNOWN-1;

pub(crate) fn write_warning_to_console(dedupeset: &mut HashSet<String>) {
    let backtrace = Backtrace::capture();
    match backtrace.status() {
        BacktraceStatus::Captured => {
            let backtrace_str = format!("{:#?}", backtrace);
            let mut call_seeker = 0;
            let mut tofix = String::new();
            let mut called = String::new();
            for line in backtrace_str.lines() {
                if line.contains("::direct_use_check_and_warn") {
                    call_seeker = 1;
                } else if call_seeker == 2 {
                    if !line.contains("futures_util::future::") && !line.contains("::pin::Pin") {
                        tofix = line.to_string();
                        break;
                    }
                } else if call_seeker == 1 {
                    called = line.to_string();
                    call_seeker = 2;
                }
            }
            if dedupeset.contains(&tofix) {
                return;
            }
            eprintln!("       called:{} but probably should have used monitor.XXX since monitoring was enabled.", called);
            eprintln!("fix code here:{}\n", tofix.red());
            dedupeset.insert(tofix);
        }
        _ => {
            warn!("you called this without the monitor but monitoring for this channel is enabled. see the monitor version of this method");
        }
    }
}

#[derive(Default)]
pub enum SendSaturation {
    IgnoreAndWait,
    IgnoreAndErr,
    #[default]
    Warn,
    IgnoreInRelease,
}

struct HashedIterator<I, T> {
    inner: I,
    _marker: std::marker::PhantomData<T>,
}

impl<I, T> HashedIterator<I, T> {
    fn new(inner: I) -> Self {
        HashedIterator {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<I, T> Iterator for HashedIterator<I, T>
    where
        I: Iterator<Item = T>,
        T: Hash,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.inner.next() {
            let mut hasher = DefaultHasher::new();
            item.hash(&mut hasher);
            let hash = hasher.finish();
            println!("Hash: {}", hash);
            Some(item)
        } else {
            None
        }
    }
}

/// Represents a standard deviation value.
///
/// The `StdDev` struct is used to encapsulate a standard deviation value within a specified range.
/// This struct provides methods to create standard deviation values for use in metrics and alerts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StdDev(f32);

impl StdDev {
    /// Creates a new `StdDev` value if it falls within the valid range (0.0, 10.0).
    ///
    /// # Parameters
    /// - `value`: The standard deviation value.
    ///
    /// # Returns
    /// - `Some(StdDev)`: If the value is within the range (0.0, 10.0).
    /// - `None`: If the value is outside the valid range.
    fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Creates a `StdDev` value of 1.0.
    ///
    /// # Returns
    /// - `StdDev(1.0)`: A standard deviation value of 1.0.
    pub fn one() -> Self {
        Self(1.0)
    }

    /// Creates a `StdDev` value of 1.5.
    ///
    /// # Returns
    /// - `StdDev(1.5)`: A standard deviation value of 1.5.
    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    /// Creates a `StdDev` value of 2.0.
    ///
    /// # Returns
    /// - `StdDev(2.0)`: A standard deviation value of 2.0.
    pub fn two() -> Self {
        Self(2.0)
    }

    /// Creates a `StdDev` value of 2.5.
    ///
    /// # Returns
    /// - `StdDev(2.5)`: A standard deviation value of 2.5.
    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    /// Creates a `StdDev` value of 3.0.
    ///
    /// # Returns
    /// - `StdDev(3.0)`: A standard deviation value of 3.0.
    pub fn three() -> Self {
        Self(3.0)
    }

    /// Creates a `StdDev` value of 4.0.
    ///
    /// # Returns
    /// - `StdDev(4.0)`: A standard deviation value of 4.0.
    pub fn four() -> Self {
        Self(4.0)
    }

    /// Creates a `StdDev` value with a custom value if it falls within the valid range (0.0, 10.0).
    ///
    /// # Parameters
    /// - `value`: The custom standard deviation value.
    ///
    /// # Returns
    /// - `Some(StdDev)`: If the value is within the range (0.0, 10.0).
    /// - `None`: If the value is outside the valid range.
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    /// Retrieves the value of the standard deviation.
    ///
    /// # Returns
    /// - `f32`: The encapsulated standard deviation value.
    pub fn value(&self) -> f32 {
        self.0
    }
}


pub trait Metric {}

pub trait DataMetric: Metric {}

pub trait ComputeMetric: Metric {}
impl Metric for Duration {}

/// Represents the color of an alert in the Steady State framework.
///
/// The `AlertColor` enum is used to indicate the severity level of an alert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertColor {
    /// Indicates a warning level alert.
    ///
    /// Typically used for non-critical issues that may require attention.
    Yellow,

    /// Indicates an elevated alert level.
    ///
    /// Used for more serious issues that need prompt attention.
    Orange,

    /// Indicates a critical alert level.
    ///
    /// Used for severe issues that require immediate action.
    Red,
}

/// Represents a trigger condition for a metric in the Steady State framework.
///
/// The `Trigger` enum is used to define various conditions that, when met, will trigger an alert.
/// Each variant specifies a different type of condition.
#[derive(Clone, Copy, Debug)]
pub enum Trigger<T>
    where
        T: Metric,
{
    /// Trigger an alert when the average value of the metric is above the specified threshold.
    ///
    /// Contains the threshold value of type `T`.
    AvgAbove(T),

    /// Trigger an alert when the average value of the metric is below the specified threshold.
    ///
    /// Contains the threshold value of type `T`.
    AvgBelow(T),

    /// Trigger an alert when the value of the metric is above the mean plus a specified number of standard deviations.
    ///
    /// Contains the number of standard deviations and the mean value of type `T`.
    StdDevsAbove(StdDev, T),

    /// Trigger an alert when the value of the metric is below the mean minus a specified number of standard deviations.
    ///
    /// Contains the number of standard deviations and the mean value of type `T`.
    StdDevsBelow(StdDev, T),

    /// Trigger an alert when the value of the metric is above a specified percentile.
    ///
    /// Contains the percentile value and the threshold value of type `T`.
    PercentileAbove(Percentile, T),

    /// Trigger an alert when the value of the metric is below a specified percentile.
    ///
    /// Contains the percentile value and the threshold value of type `T`.
    PercentileBelow(Percentile, T),
}
