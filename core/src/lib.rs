//! # Steady State Core - Easy Performant Async
//!  Steady State is a high performance, easy to use, actor based framework for building concurrent applications in Rust.
//!  Guarantee your SLA with telemetry, alerts and Prometheus.
//!  Build low latency high volume solutions.

pub(crate) mod telemetry {
    /// Telemetry module for monitoring and collecting metrics.
    pub(crate) mod metrics_collector;
    /// Telemetry module for consume of all collected metrics either for
    /// Prometheus or for the local telemetry server. Also writes history file.
    pub(crate) mod metrics_server;
    /// Build logic for adding telemetry actors to an application graph.
    pub(crate) mod setup;
}

pub(crate) mod serialize {
    /// Serialization module for efficient data packing.
    pub(crate) mod byte_buffer_packer;
    /// Implementation of packed int/long from the FAST/FIX protocol
    pub(crate) mod fast_protocol_packed;
}

pub(crate) mod channel_stats;
pub(crate) mod actor_stats;
pub(crate) mod steady_config;
pub(crate) mod dot;

mod graph_liveliness;
mod loop_driver;
mod yield_now;
mod abstract_executor;
mod test_panic_capture;
mod steady_telemetry;
/////////////////////////////////////////////////

    /// module for all monitor features
pub mod monitor;
    /// module for all channel features
pub mod channel_builder;
    /// module for all actor features
pub mod actor_builder;
    /// util module for various utility functions.
pub mod util;
/// Installation modules for setting up various deployment methods.
pub mod install {
    /// module with support for creating and removing systemd configuration
    pub mod serviced;
    /// module with support for creating local command line applications
    pub mod local_cli;
    /// module with support for creating docker containers (may be deprecated)
    pub mod container;
}
    /// module for testing full graphs of actors
pub mod graph_testing;
    /// module for all tx channel features
pub mod steady_tx;
    /// module for all rx channel features
pub mod steady_rx;


pub use graph_testing::GraphTestResult;
pub use monitor::LocalMonitor;
pub use channel_builder::Rate;
pub use channel_builder::Filled;
pub use channel_builder::LazySteadyRx;
pub use channel_builder::LazySteadyTx;
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

use std::any::{Any};
use std::time::{Duration, Instant};
#[cfg(test)]
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use futures::lock::{Mutex};
use std::ops::{DerefMut};
use std::pin::Pin;
use log::*;


use actor_builder::ActorBuilder;
use crate::monitor::{ActorMetaData, ChannelMetaData, RxMetaData, TxMetaData};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;
use futures::*;
use futures::channel::oneshot;
use futures::select;
use futures_timer::Delay;
use futures_util::future::{FusedFuture, select_all};
use futures_util::lock::MutexGuard;
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


/// Type alias for an array of thread-safe transmitters (Tx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of transmitters that can be shared across multiple threads.
pub type LazySteadyTxBundle<T, const GIRTH: usize> = [LazySteadyTx<T>; GIRTH];

pub trait LazySteadyTxBundleClone<T, const GIRTH: usize> {
    fn clone(&self) -> SteadyTxBundle<T, GIRTH>;
    async fn testing_send(&self, data: Vec<T>, index:usize, close: bool);
    async fn testing_mark_closed(&self, index: usize);
}

impl<T, const GIRTH: usize> LazySteadyTxBundleClone<T, GIRTH> for LazySteadyTxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyTxBundle<T, GIRTH> {
        let tx_clones:Vec<SteadyTx<T>> = self.iter().map(|l|l.clone()).collect();
        match tx_clones.try_into() {
            Ok(array) => steady_tx_bundle(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }

    async fn testing_send(&self, data: Vec<T>, index: usize, close: bool) {
        if index >= GIRTH {
            panic!("Index out of bounds");
        }

        let tx_clone:SteadyTx<T> = self[index].clone();

        let mut tx = tx_clone.lock().await;
        for d in data.into_iter() {
            tx.shared_send_iter_until_full([d].into_iter());
        }
        if close {
            tx.mark_closed();
        };
    }

    async fn testing_mark_closed(&self, index: usize) {
        if index >= GIRTH {
            panic!("Index out of bounds");
        }

        let tx_clone:SteadyTx<T> = self[index].clone();
        let mut tx = tx_clone.lock().await;
        tx.mark_closed();

    }

}

/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of receivers that can be shared across multiple threads.
pub type LazySteadyRxBundle<T, const GIRTH: usize> = [LazySteadyRx<T>; GIRTH];

pub trait LazySteadyRxBundleClone<T, const GIRTH: usize> {
    fn clone(&self) -> SteadyRxBundle<T, GIRTH>;
}

impl<T, const GIRTH: usize> crate::LazySteadyRxBundleClone<T, GIRTH> for LazySteadyRxBundle<T, GIRTH> {
    fn clone(&self) -> SteadyRxBundle<T, GIRTH> {
        let rx_clones:Vec<SteadyRx<T>> = self.iter().map(|l|l.clone()).collect();
        match rx_clones.try_into() {
            Ok(array) => steady_rx_bundle(array),
            Err(_) => {
                panic!("Internal error, bad length");
            }
        }
    }
}

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

/// Creates a bundle of thread-safe transmitters (Tx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This function takes an array of transmitters and wraps it in an `Arc` for shared ownership.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyTx<T>` with a fixed size (GIRTH).
///
/// # Returns
/// - `SteadyTxBundle<T, GIRTH>`: A bundle of transmitters wrapped in an `Arc`.
// pub fn steady_tx_lazy_bundle<T, const GIRTH: usize>(internal_array: [LazySteadyTx<T>; GIRTH]) -> LazySteadyTxBundle<T, GIRTH> {
//     internal_array
// }

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

/// Creates a bundle of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This function takes an array of receivers and wraps it in an `Arc` for shared ownership.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyRx<T>` with a fixed size (GIRTH).
///
/// # Returns
/// - `SteadyRxBundle<T, GIRTH>`: A bundle of receivers wrapped in an `Arc`.
// pub fn steady_rx_lazy_bundle<T, const GIRTH: usize>(internal_array: [LazySteadyRx<T>; GIRTH]) -> LazySteadyRxBundle<T, GIRTH> {
//     internal_array
// }

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

    /// Checks if the liveliness state matches any of the target states.
    ///
    /// # Parameters
    /// - `target`: A slice of `GraphLivelinessState`.
    /// - `upon_posion`: The return value if the liveliness state is poisoned.
    ///
    /// # Returns
    /// `true` if the liveliness state matches any target state, otherwise `false`.
    pub fn is_liveliness_in(&self, target: &[GraphLivelinessState], upon_poison: bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => liveliness.is_in_state(target),
            Err(e) => {
                trace!("Internal error, unable to get liveliness read lock {}", e);
                upon_poison
            }
        }
    }

    /// Checks if the actor is part of the graph.
    ///
    /// # Returns
    /// `true` if the monitor is in the graph, otherwise `false`.
    pub fn is_in_graph(&self) -> bool {
        self.is_in_graph
    }

    /// Waits while the actor is running.
    ///
    /// # Returns
    /// A future that resolves to `Ok(())` if the monitor stops, otherwise `Err(())`.
    pub fn wait_while_running(&self) -> impl Future<Output = Result<(), ()>> {
        crate::graph_liveliness::WaitWhileRunningFuture::new(self.runtime_state.clone())
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
    pub async fn wait_avail_units_bundle<T>(&self, this: &mut RxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    {
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

        let futures: Vec<_> = futures.collect();
        let mut futures = futures;

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
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
    pub async fn wait_vacant_units_bundle<T>(&self, this: &mut TxBundle<'_, T>, avail_count: usize, ready_channels: usize) -> bool
    where
        T: Send + Sync,
    {
        let mut count_down = ready_channels.min(this.len());
        let result = Arc::new(AtomicBool::new(true));

        let futures = this.iter_mut().map(|tx| {
            let local_r = result.clone();
            async move {
                let bool_result = tx.shared_wait_vacant_units(avail_count).await;
                if !bool_result {
                    local_r.store(false, Ordering::Relaxed);
                }
            }
                .boxed() // Box the future to make them the same type
        });

        let futures: Vec<_> = futures.collect();
        let mut futures = futures;

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
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
    pub fn try_peek_slice<T>(&self, this: &mut Rx<T>, elems: &mut [T]) -> usize
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
    pub async fn peek_async_slice<T>(&self, this: &mut Rx<T>, wait_for_count: usize, elems: &mut [T]) -> usize
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
    pub fn take_slice<T>(&mut self, this: &mut Rx<T>, slice: &mut [T]) -> usize
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
    pub fn try_peek<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&T>
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
    pub fn try_peek_iter<'a, T>(&'a self, this: &'a mut Rx<T>) -> impl Iterator<Item = &'a T> + 'a {
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
    pub async fn peek_async_iter<'a, T>(&'a self, this: &'a mut Rx<T>, wait_for_count: usize) -> impl Iterator<Item = &'a T> + 'a {
        this.shared_peek_async_iter(wait_for_count).await
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    pub fn is_empty<T>(&self, this: &mut Rx<T>) -> bool {
        this.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    pub fn avail_units<T>(&self, this: &mut Rx<T>) -> usize {
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
    pub async fn peek_async<'a, T>(&'a self, this: &'a mut Rx<T>) -> Option<&T>
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
    pub fn send_slice_until_full<T>(&mut self, this: &mut Tx<T>, slice: &[T]) -> usize
    where
        T: Copy,
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
    pub fn send_iter_until_full<T, I: Iterator<Item = T>>(&mut self, this: &mut Tx<T>, iter: I) -> usize {
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
    pub fn try_send<T>(&mut self, this: &mut Tx<T>, msg: T) -> Result<(), T> {
        this.shared_try_send(msg)
    }

    /// Checks if the Tx channel is currently full.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    pub fn is_full<T>(&self, this: &mut Tx<T>) -> bool {
        this.is_full()
    }

    /// Returns the number of vacant units in the Tx channel.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    pub fn vacant_units<T>(&self, this: &mut Tx<T>) -> usize {
        this.shared_vacant_units()
    }

    /// Asynchronously waits until the Tx channel is empty.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to a `Tx<T>` instance.
    ///
    /// # Asynchronous
    pub async fn wait_empty<T>(&self, this: &mut Tx<T>) -> bool {
        this.shared_wait_empty().await
    }

    /// Takes messages into an iterator.
    ///
    /// # Parameters
    /// - `this`: A mutable reference to an `Rx<T>` instance.
    ///
    /// # Returns
    /// An iterator over the taken messages.
    pub fn take_into_iter<'a, T: Sync + Send>(& mut self, this: &'a mut Rx<T>) -> impl Iterator<Item = T> + 'a
    {
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
    pub async fn call_async<F>(&self, operation: F) -> Option<F::Output>
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
        this.shared_send_async(a, self.ident, saturation).await
    }

    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    pub fn try_take<T>(&self, this: &mut Rx<T>) -> Option<T> {
        this.shared_try_take()
    }

    /// Attempts to take a message from the channel if available.
    ///
    /// # Returns
    /// An `Option<T>`, where `Some(T)` contains the message if available, or `None` if the channel is empty.
    pub async fn take_async<T>(&self, this: &mut Rx<T>) -> Option<T> {
        this.shared_take_async().await
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
        self.node_tx_rx.as_ref().map(|node_tx_rx| SideChannelResponder::new(node_tx_rx.clone(),self.oneshot_shutdown.clone() ))
    }


    /// Returns a side channel responder if available.
    ///
    /// # Returns
    /// An `Option` containing a `SideChannelResponder` if available.
    pub fn sidechannel_responder(&self) -> Option<SideChannelResponder> {
        // if we have no back channel plane then we cannot simulate the edges
        self.node_tx_rx.as_ref().map(|node_tx_rx| SideChannelResponder::new(node_tx_rx.clone(),self.oneshot_shutdown.clone() ))
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
        let (send_rx, send_tx, state) = if steady_config::TELEMETRY_HISTORY || steady_config::TELEMETRY_SERVER {
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
            args: self.args,
            #[cfg(test)]
            test_count: HashMap::new(),
        }
    }
}

/// Macro takes a SteadyContext and a list of Rx and Tx channels
/// and returns a LocalMonitor. The Monitor is the only way to produce
/// metrics for telemetry and prometheus.  The macro is used to ensure
/// specifically which channels are monitored.
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


/// Represents the behavior of the system when the channel is saturated (i.e., full).
///
/// The `SendSaturation` enum defines how the system should respond when attempting to send a message
/// to a channel that is already at capacity. This helps in managing backpressure and ensuring
/// the system behaves predictably under load.
#[derive(Default)]
pub enum SendSaturation {
    /// Ignore the saturation and wait until space is available in the channel.
    ///
    /// This option blocks the sender until there is space in the channel, ensuring that
    /// all messages are eventually sent. This can help maintain a reliable flow of messages
    /// but may lead to increased latency under high load.
    IgnoreAndWait,

    /// Ignore the saturation and return an error immediately.
    ///
    /// This option allows the sender to detect and handle the saturation condition
    /// without blocking. It returns an error, which can be used to implement custom
    /// backpressure handling or retry logic.
    IgnoreAndErr,

    /// Warn about the saturation condition but allow the message to be sent anyway.
    ///
    /// This is the default behavior. It logs a warning when saturation occurs,
    /// which can be useful for monitoring and diagnostics, but does not prevent the message
    /// from being sent. This can help maintain throughput but may lead to resource exhaustion
    /// if not monitored properly.
    #[default]
    Warn,

    /// Ignore the saturation condition entirely in release builds.
    ///
    /// This option is similar to `Warn`, but it does not generate warnings in release builds.
    /// This can be useful for performance-critical applications where logging overhead needs to be minimized.
    IgnoreInRelease,
}

/*
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
*/

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

/// Base Trait for all metrics for use on Telemetry and Prometheus.
pub trait Metric {}

/// Represents a Metric suitable for channels which transfer data.
pub trait DataMetric: Metric {}

/// Represents a Metric suitable for actors which perform computations.
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
