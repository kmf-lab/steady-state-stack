
pub(crate) mod telemetry {
    pub(crate) mod metrics_collector;
    pub(crate) mod metrics_server;
    pub(crate) mod setup;
}

pub(crate) mod serialize {
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
    pub mod serviced;
    pub mod local_cli;
    pub mod container;
}
pub mod actor_builder;
mod actor_stats;
mod graph_testing;
mod graph_liveliness;
mod loop_driver;
mod yield_now;

pub use graph_testing::GraphTestResult;
pub use actor_builder::SupervisionStrategy;
pub use graph_testing::EdgeSimulationDirector;
pub use monitor::LocalMonitor;
pub use channel_builder::Rate;
pub use channel_builder::Filled;
pub use actor_builder::MCPU;
pub use actor_builder::Work;
pub use actor_builder::Percentile;
pub use graph_liveliness::*;
pub use loop_driver::*;
pub use install::serviced::*;

use std::any::{Any, type_name};


use std::time::{Duration, Instant};

#[cfg(test)]
use std::collections::HashMap;

use std::fmt::Debug;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use futures::lock::{Mutex, MutexLockFuture};
use std::ops::{Deref, DerefMut};
use std::future::ready;


use std::thread::sleep;
use log::*;
use channel_builder::{InternalReceiver, InternalSender};
use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
use ringbuf::traits::Observer;
use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use backtrace::Backtrace;

use nuclei::config::{IoUringConfiguration, NucleiConfig};
use actor_builder::ActorBuilder;
use crate::monitor::{ActorMetaData, ChannelMetaData, RxMetaData, TxMetaData};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;// re-publish in public
use futures::*; // Provides the .fuse() method

// Re-exporting Bastion blocking for use in actors
pub use bastion::blocking;
use bastion::run;
use futures::channel::oneshot;


use futures::select;
use futures_timer::Delay;
use futures_util::future::{BoxFuture, FusedFuture, select_all};
use futures_util::lock::MutexGuard;
use crate::graph_testing::EdgeSimulator;
use crate::yield_now::yield_now;

pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;

pub type SteadyTxBundle<T,const GIRTH:usize> = Arc<[SteadyTx<T>;GIRTH]>;
pub type SteadyRxBundle<T,const GIRTH:usize> = Arc<[SteadyRx<T>;GIRTH]>;

pub type TxBundle<'a, T> = Vec<MutexGuard<'a, Tx<T>>>;

pub type RxBundle<'a, T> = Vec<MutexGuard<'a, Rx<T>>>;


pub fn steady_tx_bundle<T,const GIRTH:usize>(internal_array: [SteadyTx<T>;GIRTH]) -> SteadyTxBundle<T, GIRTH> {
    Arc::new(internal_array)
}
pub fn steady_rx_bundle<T,const GIRTH:usize>(internal_array: [SteadyRx<T>;GIRTH]) -> SteadyRxBundle<T, GIRTH> {
    Arc::new(internal_array)
}

/// Initialize logging for the steady_state crate.
/// This is a convenience function that should be called at the beginning of main.
pub fn init_logging(loglevel: &str) -> Result<(), Box<dyn std::error::Error>> {

    //TODO: should probably be its own init function.
    init_nuclei();  //TODO: add some configs and a boolean to control this?

    steady_logging_init(loglevel)
}

pub struct SteadyContext {
    pub(crate) ident: ActorIdentity,
    pub(crate) ctx: Option<Arc<bastion::context::BastionContext>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) count_restarts: Arc<AtomicU32>,
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    pub(crate) oneshot_shutdown: Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) last_perodic_wait: AtomicU64,
    pub(crate) actor_start_time: Instant,
}


impl SteadyContext {

    pub(crate) fn is_liveliness_in(&self, target: &[GraphLivelinessState], upon_posion: bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_in_state(target)
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness read lock {}",e);
                upon_posion
            }
        }
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

        let now_nanos = self.actor_start_time.elapsed().as_nanos() as u64;
        let run_duration = now_nanos - self.last_perodic_wait.load(Ordering::Relaxed);
        let remaining_duration = duration_rate.saturating_sub( Duration::from_nanos(run_duration) );

        let mut operation = &mut Delay::new(remaining_duration).fuse();
        let result = select! {
                _= &mut one_down.deref_mut() => false,
                _= operation => true
        };
        self.last_perodic_wait.store(remaining_duration.as_nanos() as u64 + now_nanos, Ordering::Relaxed);
        result
    }

    /// Asynchronously waits for a specified duration.
    ///
    /// # Parameters
    /// - `duration`: The duration to wait.
    ///
    /// # Asynchronous
    pub async fn wait(&self, duration: Duration) {
        let one_down = &mut self.oneshot_shutdown.lock().await;
        select! { _ = one_down.deref_mut() => {}, _ =Delay::new(duration).fuse() => {} }
    }

    /// yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    pub async fn yield_now(&self) {
        yield_now().await;
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
    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T,saturation:SendSaturation ) -> Result<(), T> {
        #[cfg(debug_assertions)]
        this.direct_use_check_and_warn();
        this.shared_send_async(a, self.ident,saturation).await
    }

    pub fn edge_simulator(&self) -> Option<EdgeSimulator> {
        self.ctx.as_ref().map(|ctx| EdgeSimulator::new(ctx.clone()))
    }


    #[inline]
    pub fn is_running(&self, accept_fn: &mut dyn FnMut() -> bool) -> bool {
        match self.runtime_state.read() {
            Ok(liveliness) => {
                liveliness.is_running(self.ident, accept_fn)
            }
            Err(e) => {
                error!("internal error,unable to get liveliness read lock {}",e);
                true
            }
        }
    }

    #[inline]
    pub fn request_graph_stop(&self) -> bool {
        match self.runtime_state.write() {
            Ok(mut liveliness) => {
                liveliness.request_shutdown();
                true
            }
            Err(e) => {
                trace!("internal error,unable to get liveliness write lock {}",e);
                false //keep running as the default under error conditions
            }
        }
    }

    pub fn args<A: Any>(&self) -> Option<&A> {
        self.args.downcast_ref::<A>()
    }

    pub fn identity(&self) -> ActorIdentity {
        self.ident
    }


    //add methods from monitor



   pub fn into_monitor<const RX_LEN: usize, const TX_LEN: usize>(self
                              , rx_mons: [& dyn RxDef; RX_LEN]
                              , tx_mons: [& dyn TxDef; TX_LEN]
   ) -> LocalMonitor<RX_LEN,TX_LEN> {

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

    pub fn into_monitor_internal<const RX_LEN: usize, const TX_LEN: usize>(self
                                            , rx_mons: [RxMetaData; RX_LEN]
                                            , tx_mons: [TxMetaData; TX_LEN]
    ) -> LocalMonitor<RX_LEN,TX_LEN> {

        //only build telemetry channels if this feature is enabled
        let (telemetry_send_rx, telemetry_send_tx, telemetry_state) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {

            let mut rx_meta_data = Vec::new();
            let mut rx_inverse_local_idx = [0; RX_LEN];
            rx_mons.iter()
                .enumerate()
                .for_each(|(c, md)| {
                    assert!(md.0.id < usize::MAX);
                    rx_inverse_local_idx[c]=md.0.id;
                    rx_meta_data.push(md.0.clone());
                });

            let mut tx_meta_data = Vec::new();
            let mut tx_inverse_local_idx = [0; TX_LEN];
            tx_mons.iter()
                .enumerate()
                .for_each(|(c, md)| {
                    assert!(md.0.id < usize::MAX);
                    tx_inverse_local_idx[c]=md.0.id;
                    tx_meta_data.push(md.0.clone());
                });

            setup::construct_telemetry_channels(&self
                                                , rx_meta_data, rx_inverse_local_idx
                                                , tx_meta_data, tx_inverse_local_idx)
        } else {
            (None, None, None)
        };


        // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry_send_rx,
            telemetry_send_tx,
            telemetry_state,
            last_telemetry_send: Instant::now(),
            ident: self.ident,
            ctx: self.ctx,
            runtime_state: self.runtime_state,
            oneshot_shutdown: self.oneshot_shutdown,
            last_perodic_wait: Default::default(),
            actor_start_time: self.actor_start_time,

            #[cfg(test)]
            test_count: HashMap::new(),

        }
    }
}

/*
macro_rules! concat_tuples {
    (($($left:expr),*), ($($right:expr),*)) => {
        ($($left,)* $($right,)*)
    };
}

macro_rules! concat_arrays {
    ([$($left:expr),*], [$($right:expr),*]) => {
        [$($left,)* $($right,)*]
    };
}*/

#[macro_export]
macro_rules! into_monitor {
    ($self:expr, [$($rx:expr),*], [$($tx:expr),*]) => {{
        //this allows for 'arrays' of non homogneous types and avoids dyn
        let rx_meta = [$($rx.meta_data(),)*];
        let tx_meta = [$($tx.meta_data(),)*];

        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
    ($self:expr, [$($rx:expr),*], $tx_bundle:expr) => {{
        //this allows for 'arrays' of non homogneous types and avoids dyn
        let rx_meta = [$($rx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, $tx_bundle.meta_data())
    }};
     ($self:expr, $rx_bundle:expr, [$($tx:expr),*] ) => {{
        //this allows for 'arrays' of non homogneous types and avoids dyn
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal($rx_bundle.meta_data(), tx_meta)
    }};
    ($self:expr, $rx_bundle:expr, $tx_bundle:expr) => {{
        $self.into_monitor_internal($rx_bundle.meta_data(), $tx_bundle.meta_data())
    }};
    ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*], $($rx_bundle:expr),* ), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
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

/*
macro_rules! my_macro {
    ($a:expr) => { /* Handle single expression */ };
    ($a:expr, $b:expr) => { /* Handle two expressions */ };
}

macro_rules! into_monitor_macro2 {
    ($self:expr, $rx_mons:expr, $tx_mons:expr) => {{
        let rx_meta = $rx_mons
            .iter()
            .map(|rx| rx.meta_data())
            .collect::<Vec<_>>();
        let rx_meta: [_; $rx_mons.len()] = rx_meta.try_into().expect("Length mismatch should never occur");

        let tx_meta = $tx_mons
            .iter()
            .map(|tx| tx.meta_data())
            .collect::<Vec<_>>();
        let tx_meta: [_; $tx_mons.len()] = tx_meta.try_into().expect("Length mismatch should never occur");

        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
}
*/

pub struct Graph  {
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) monitor_count: Arc<AtomicUsize>,
    //used by collector but could grow if we get new actors at runtime
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
}

impl Graph {

    /// needed for testing only, this monitor assumes we are running without a full graph
    /// and will not be used in production
    pub fn new_test_monitor(&mut self, name: & 'static str ) -> SteadyContext
    {
        // assert that we are NOT in release mode
        assert!(cfg!(debug_assertions), "This function is only for testing");

        let channel_count    = self.channel_count.clone();
        let all_telemetry_rx = self.all_telemetry_rx.clone();

        let oneshot_shutdown = {
            let (send_shutdown_notice_to_periodic_wait, oneshot_shutdown) = oneshot::channel();
            let mut one_shots = run!(self.oneshot_shutdown_vec.lock());
            one_shots.push(send_shutdown_notice_to_periodic_wait);
            oneshot_shutdown
        };
        let oneshot_shutdown = Arc::new(Mutex::new(oneshot_shutdown));
        let now = Instant::now();

        let count_restarts   = Arc::new(AtomicU32::new(0));
        SteadyContext {
            channel_count,
            ident: ActorIdentity{name, id: self.monitor_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst)},
            args: self.args.clone(),
            ctx: None, //this is key, we are not running in a graph by design
            actor_metadata: Arc::new(ActorMetaData::default()),
            all_telemetry_rx,
            runtime_state: self.runtime_state.clone(),
            count_restarts,
            oneshot_shutdown_vec: self.oneshot_shutdown_vec.clone(),
            oneshot_shutdown,
            last_perodic_wait: Default::default(),
            actor_start_time: now,
        }
    }
}


fn init_nuclei() {
    let nuclei_config = NucleiConfig {
        iouring: IoUringConfiguration::interrupt_driven(1 << 6),
        //iouring: IoUringConfiguration::kernel_poll_only(1 << 6),
        //iouring: IoUringConfiguration::low_latency_driven(1 << 6),
        //iouring: IoUringConfiguration::io_poll(1 << 6),
    };
    let _ = nuclei::Proactor::with_config(nuclei_config);

    if cfg!(target_arch = "wasm32") {
        //we can not write log files when in wasm
    } else {
        nuclei::spawn_blocking(|| {
            nuclei::drive(async {
                loop {
                    sleep(Duration::from_secs(10));
                    ready(()).await;
                };
            });
        }).detach();
    }

}

const MONITOR_UNKNOWN: usize = usize::MAX;
const MONITOR_NOT: usize = MONITOR_UNKNOWN-1; //any value below this is a valid monitor index

pub struct Tx<T> {
    pub(crate) tx: InternalSender<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) last_error_send: Instant,
    pub(crate) make_closed: Option<oneshot::Sender<()>>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
}

pub struct Rx<T> {
    pub(crate) rx: InternalReceiver<T>,
    pub(crate) channel_meta_data: Arc<ChannelMetaData>,
    pub(crate) local_index: usize,
    pub(crate) is_closed: oneshot::Receiver<()>,
    pub(crate) oneshot_shutdown: oneshot::Receiver<()>,
}

////////////////////////////////////////////////////////////////
impl<T> Tx<T> {

    ///the Rx should not expect any more messages than those already found
    ///on the channel. This is a signal that this actor has probably stopped
    pub fn mark_closed(&mut self) -> bool {
        if let Some(c) = self.make_closed.take() {
            match c.send(()) {
                Ok(_) => {true},
                Err(_) => {false}
            }
        } else {
            true //already closed
        }
    }


    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// This method is useful for understanding the size constraints of the channel
    /// and for configuring buffer sizes or for performance tuning.

    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Attempts to send a message to the channel without blocking.
    /// If the channel is full, the send operation will fail and return the message.
    ///
    /// # Parameters
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates successful send and `Err(T)` returns the message if the channel is full.
    ///
    /// # Example Usage
    /// Use this method for non-blocking send operations where immediate feedback on send success is required.
    /// Not suitable for scenarios where ensuring message delivery is critical without additional handling for failed sends.
    pub fn try_send(& mut self, msg: T) -> Result<(), T> {
         self.shared_try_send(msg)
    }

    /// Sends messages from an iterator to the channel until the channel is full.
    /// Each message is sent in a non-blocking manner.
    ///
    /// # Parameters
    /// - `iter`: An iterator that yields messages of type `T`.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Ideal for batch sending operations where partial success is acceptable.
    /// Less suitable when all messages must be sent without loss, as this method does not guarantee all messages are sent.
    pub fn send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
         self.shared_send_iter_until_full(iter)
    }

    /// Sends a message to the channel asynchronously, waiting if necessary until space is available.
    ///
    /// # Parameters
    /// - `ident`: The identity of the actor sending the message. Get this from the SteadyContext.
    /// - `msg`: The message to be sent.
    ///
    /// # Returns
    /// A `Result<(), T>`, where `Ok(())` indicates that the message was successfully sent, and `Err(T)` if the send operation could not be completed.
    ///
    /// # Example Usage
    /// Suitable for scenarios where it's critical that a message is sent, and the sender can afford to wait.
    /// Not recommended for real-time systems where waiting could introduce unacceptable latency.
    pub async fn send_async(&mut self, ident:ActorIdentity, a: T, saturation:SendSaturation) -> Result<(), T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_send_async(a, ident, saturation).await
    }

    /// Sends messages from a slice to the channel until the channel is full.
    /// Operates in a non-blocking manner, similar to `send_iter_until_full`, but specifically for slices.
    /// This method requires `T` to implement the `Copy` trait, ensuring that messages can be copied into the channel without taking ownership.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This allows the method to efficiently handle the messages without needing to manage ownership or deal with borrowing complexities.
    ///
    /// # Parameters
    /// - `slice`: A slice of messages to be sent.
    ///
    /// # Returns
    /// The number of messages successfully sent before the channel became full.
    ///
    /// # Example Usage
    /// Useful for efficiently sending multiple messages when partial delivery is sufficient.
    /// May not be appropriate for use cases requiring guaranteed delivery of all messages.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Efficiency**: Allows for the fast and straightforward copying of messages into the channel, leveraging the stack when possible.
    /// - **Simplicity**: Simplifies message handling by avoiding the complexities of ownership and borrowing rules inherent in more complex types.
    /// - **Predictability**: Ensures that messages remain unchanged after being sent, providing clear semantics for message passing.
    ///
    /// ## Cons:
    /// - **Flexibility**: Restricts the types of messages that can be sent through the channel to those that implement `Copy`, potentially limiting the use of the channel with more complex data structures.
    /// - **Resource Use**: For types where `Copy` might involve deep copying (though typically `Copy` is for inexpensive-to-copy types), it could lead to unintended resource use.
    /// - **Design Constraints**: Imposes a design constraint on the types used with the channel, which might not always align with the broader application architecture or data handling strategies.
    ///
    /// By requiring `T: Copy`, this method optimizes for scenarios where messages are lightweight and can be copied without significant overhead, making it an excellent choice for high-throughput, low-latency applications.
    pub fn send_slice_until_full(&mut self, slice: &[T]) -> usize
       where T: Copy {
        self.shared_send_slice_until_full(slice)
    }

    /// Checks if the channel is currently full.
    ///
    /// # Returns
    /// `true` if the channel is full and cannot accept more messages, otherwise `false`.
    ///
    /// # Example Usage
    /// Can be used to avoid calling `try_send` on a full channel, or to implement backpressure mechanisms.
    pub fn is_full(&self) -> bool {
        self.shared_is_full()
    }

    /// Returns the number of vacant units in the channel.
    /// Indicates how many more messages the channel can accept before becoming full.
    ///
    /// # Returns
    /// The number of messages that can still be sent before the channel is full.
    ///
    /// # Example Usage
    /// Useful for dynamically adjusting the rate of message sends or for implementing custom backpressure strategies.
    pub fn vacant_units(&self) -> usize {
        self.shared_vacant_units()
    }

    /// Asynchronously waits until at least a specified number of units are vacant in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of vacant units to wait for.
    ///
    /// # Example Usage
    /// Use this method to delay message sending until there's sufficient space, suitable for scenarios where message delivery must be paced or regulated.
    pub async fn wait_vacant_units(&mut self, count: usize) -> bool {
        self.shared_wait_vacant_units(count).await
    }

    /// Asynchronously waits until the channel is empty.
    /// This method can be used to ensure that all messages have been processed before performing further actions.
    ///
    /// # Example Usage
    /// Ideal for scenarios where a clean state is required before proceeding, such as before shutting down a system or transitioning to a new state.
    pub async fn wait_empty(&mut self) -> bool {
        self.shared_wait_empty().await
    }

    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            write_warning_to_console(backtrace::Backtrace::new());

        }
    }
    ////////////////////////////////////////////////////////////////
    // Shared implmentations, if you need to swap out the channel it is done here
    ////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.tx.capacity().get()
    }

    #[inline]
    fn shared_try_send(& mut self, msg: T) -> Result<(), T> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())}
            Err(m) => {Err(m)}
        }
    }
    #[inline]
    fn shared_send_iter_until_full<I: Iterator<Item = T>>(&mut self, iter: I) -> usize {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        self.tx.push_iter(iter)
    }
    #[inline]
    fn shared_send_slice_until_full(&mut self, slice: &[T]) -> usize
        where T: Copy {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        self.tx.push_slice(slice)
    }
    #[inline]
    fn shared_is_full(&self) -> bool {
        self.tx.is_full()
    }
    #[inline]
    fn shared_vacant_units(&self) -> usize {
        self.tx.vacant_len()
    }
    #[inline]
    async fn shared_wait_vacant_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.tx.wait_vacant(count);
        select! { _ = one_down => false, _ = operation => true, }
    }

    #[inline]
    async fn shared_wait_empty(&mut self) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.tx.wait_vacant(usize::from(self.tx.capacity()));
        select! { _ = one_down => false, _ = operation => true, }
    }

    #[inline]
    async fn shared_send_async(& mut self, msg: T, ident: ActorIdentity, saturation: SendSaturation) -> Result<(), T> {
        if self.make_closed.is_none() {
            warn!("Send called after channel marked closed");
        }
        //by design, we ignore the runtime state because even if we are shutting down we
        //still want to wait for room for our very last message

        match self.tx.try_push(msg) {
            Ok(_) => {Ok(())},
            Err(msg) => {
                //caller can decide this is ok and not a warning
                match saturation {
                    SendSaturation::IgnoreAndWait => {
                       //never warn just await
                       //nothing to do
                    }
                    SendSaturation::IgnoreAndErr => {
                       //return error to try again
                       return Err(msg);
                    }
                    SendSaturation::Warn => {
                        //always warn
                        self.report_tx_full_warning(ident);
                    }
                    SendSaturation::IgnoreInRelease => {
                        //check release status and if not then warn
                        #[cfg(debug_assertions)]
                        {
                            self.report_tx_full_warning(ident);
                        }
                    }
                }

                //push when space becomes available
                match self.tx.push(msg).await {
                    Ok(_) => {Ok(())}
                    Err(_) => { //should not happen, internal error
                                error!("channel is closed");
                                Ok(())
                    }
                }
            }
        }
    }

    fn report_tx_full_warning(&mut self, ident: ActorIdentity) {
        if self.last_error_send.elapsed().as_secs() > 10 {
            let type_name = type_name::<T>().split("::").last();
            warn!("{:?} tx full channel #{} {:?} cap:{:?} type:{:?} "
                                   , ident
                                   , self.channel_meta_data.id
                                   , self.channel_meta_data.labels
                                   , self.tx.capacity(), type_name);
            self.last_error_send = Instant::now();
        }
    }
}


fn write_warning_to_console(stack: Backtrace) {
    warn!("you called this without the monitor but monitoring for this channel is enabled. see the monitor version of this method");

    let mut count:usize = 0;
    for frame in stack.frames() {
        let symbols = frame.symbols();
        for symbol in symbols {
            if let Some(filename) = symbol.filename() {
                let filename_str = filename.to_string_lossy();
                // Filter based on a marker in the path
                // Replace "my_project_marker" with something that uniquely identifies your project
                // if filename_str.contains("steady") {
                // Print only the relevant part of the stack
                if let Some(name) = symbol.name() {
                    //hide the fuse and pin stuff which will distract from finding the issue
                    if   !name.to_string().contains("fuse::Fuse<Fut>")
                      && !name.to_string().contains("core::pin::Pin")
                      && !name.to_string().contains("future::FutureExt")
                    {
                        if count > 0 {
                            println!("{} {:?} \n   at {}:{}", count, name, filename_str, symbol.lineno().unwrap_or(0));
                        }
                        count += 1;
                    }

                   if 3 == count {
                       return;
                   }
                }
                //  }
            }
        }
    }

}


pub enum SendSaturation {
    IgnoreAndWait,
    IgnoreAndErr,
    Warn,
    IgnoreInRelease
}
impl Default for SendSaturation {
    fn default() -> Self {
        SendSaturation::Warn
    }
}

impl<T> Rx<T> {

    /// only for use in unit tests
    pub fn block_until_not_empty(&self, duration: Duration) {
        assert!(cfg!(debug_assertions), "This function is only for testing");
        let start = Instant::now();
        loop {
            if !self.is_empty() {
                break;
            }
            if start.elapsed() > duration {
                error!("timeout waiting for channel to not be empty");
                break;
            }
            std::thread::yield_now();
        }
    }

    pub fn is_closed(&mut self) -> bool {
        if self.is_closed.is_terminated() {
            true
        } else {
            // Temporarily create a context to poll the receiver
            let waker = task::noop_waker();
            let mut context = task::Context::from_waker(&waker);
            // Non-blocking check if the receiver can resolve
            self.is_closed.poll_unpin(&mut context).is_ready()
        }
    }


    /// Returns the total capacity of the channel.
    /// This method retrieves the maximum number of messages the channel can hold.
    ///
    /// # Returns
    /// A `usize` indicating the total capacity of the channel.
    ///
    /// # Example Usage
    /// Useful for initial configuration and monitoring of channel capacity to ensure it aligns with expected load.
    pub fn capacity(&self) -> usize {
        self.shared_capacity()
    }

    /// Attempts to peek at a slice of messages from the channel without removing them.
    /// This operation is non-blocking and allows multiple messages to be inspected simultaneously.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `elems`: A mutable slice to store the peeked messages. Its length determines the maximum number of messages to peek.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Ideal for scenarios where processing or decision making is based on a batch of incoming messages without consuming them.
    pub fn try_peek_slice(&self, elems: &mut [T]) -> usize
           where T: Copy {
           self.shared_try_peek_slice(elems)
    }

    /// Asynchronously waits to peek at a slice of messages from the channel without removing them.
    /// Waits until the specified number of messages is available or the channel is closed.
    ///
    /// Requires T: Copy but this is still up for debate as it is not clear if this is the best way to handle this
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before peeking.
    /// - `elems`: A mutable slice to store the peeked messages.
    ///
    /// # Returns
    /// The number of messages actually peeked and stored in `elems`.
    ///
    /// # Example Usage
    /// Suitable for use cases requiring a specific number of messages to be available for batch processing.
    pub async fn peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
       where T: Copy {
        self.shared_peek_async_slice(wait_for_count, elems).await
    }

    /// Retrieves and removes a slice of messages from the channel.
    /// This operation is blocking and will remove the messages from the channel.
    /// Note: May take fewer messages than the slice length if the channel is not full.
    /// This method requires `T` to implement the `Copy` trait, ensuring efficient handling of message data.
    ///
    /// # Requirements
    /// - `T` must implement the `Copy` trait. This constraint ensures that messages can be efficiently copied out of the channel without ownership issues.
    ///
    /// # Parameters
    /// - `elems`: A mutable slice where the taken messages will be stored.
    ///
    /// # Returns
    /// The number of messages actually taken and stored in `elems`.
    ///
    /// # Example Usage
    /// Useful for batch processing where multiple messages are consumed at once for efficiency. Particularly effective in scenarios where processing overhead needs to be minimized.
    ///
    /// # Pros and Cons of `T: Copy`
    /// ## Pros:
    /// - **Performance Optimization**: Facilitates quick, lightweight operations by leveraging the ability to copy data directly, without the overhead of more complex memory management.
    /// - **Simplicity in Message Handling**: Reduces the complexity of managing message lifecycles and ownership, streamlining channel operations.
    /// - **Reliability**: Ensures that the operation does not inadvertently alter or lose message data during transfer, maintaining message integrity.
    ///
    /// ## Cons:
    /// - **Type Limitation**: Limits the use of the channel to data types that implement `Copy`, potentially excluding more complex or resource-managed types that might require more nuanced handling.
    /// - **Overhead for Larger Types**: While `Copy` is intended for lightweight types, using it with larger, though still `Copy`, data types could introduce unnecessary copying overhead.
    /// - **Design Rigidity**: Imposes a strict design requirement on the types that can be used with the channel, which might not align with all application designs or data models.
    ///
    /// Incorporating the `T: Copy` requirement, `take_slice` is optimized for use cases prioritizing efficiency and simplicity in handling a series of lightweight messages, making it a key method for high-performance message processing tasks.
    pub fn take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_slice(elems)
    }

    /// Attempts to take a single message from the channel without blocking.
    ///
    /// # Returns
    /// An `Option<T>` which is `Some(T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Ideal for non-blocking single message consumption, allowing for immediate feedback on message availability.
    pub fn try_take(& mut self) -> Option<T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_take()
    }

    /// Asynchronously retrieves and removes a single message from the channel.
    ///
    /// # Returns
    /// A `Result<T, String>`, where `Ok(T)` is the message if available, and `Err(String)` contains an error message if the retrieval fails.
    ///
    /// # Example Usage
    /// Suitable for async processing where messages are consumed one at a time as they become available.
    pub async fn take_async(& mut self) -> Option<T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_take_async().await
    }

    /// Attempts to peek at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message is available, or `None` if the channel is empty.
    ///
    /// # Example Usage
    /// Can be used to inspect the next message without consuming it, useful in decision-making scenarios.
    pub fn try_peek(&self) -> Option<&T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_peek()
    }

    /// Returns an iterator over the messages currently in the channel without removing them.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Enables iterating over messages for inspection or conditional processing without consuming them.
    pub fn try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_try_peek_iter()
    }

    /// Asynchronously peeks at the next message in the channel without removing it.
    ///
    /// # Returns
    /// An `Option<&T>` which is `Some(&T)` if a message becomes available, or `None` if the channel is closed.
    ///
    /// # Example Usage
    /// Useful for async scenarios where inspecting the next message without consuming it is required.
    pub async fn peek_async(& mut self) -> Option<&T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_peek_async().await
    }

    /// Asynchronously returns an iterator over the messages in the channel, waiting for a specified number of messages to be available.
    ///
    /// # Parameters
    /// - `wait_for_count`: The number of messages to wait for before returning the iterator.
    ///
    /// # Returns
    /// An iterator over the messages in the channel.
    ///
    /// # Example Usage
    /// Ideal for async batch processing where a specific number of messages are needed for processing.
    pub async fn peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_peek_async_iter(wait_for_count).await
    }

    /// Checks if the channel is currently empty.
    ///
    /// # Returns
    /// `true` if the channel has no messages available, otherwise `false`.
    ///
    /// # Example Usage
    /// Useful for determining if the channel is empty before attempting to consume messages.
    pub fn is_empty(&self) -> bool {
        //not async and immutable so no need to check
        self.shared_is_empty()
    }

    /// Returns the number of messages currently available in the channel.
    ///
    /// # Returns
    /// A `usize` indicating the number of available messages.
    ///
    /// # Example Usage
    /// Enables monitoring of the current load or backlog of messages in the channel for adaptive processing strategies.
    pub fn avail_units(& mut self) -> usize {
        //not async and immutable so no need to check
        self.shared_avail_units()
    }

    /// Asynchronously waits for a specified number of messages to be available in the channel.
    ///
    /// # Parameters
    /// - `count`: The number of messages to wait for.
    ///
    /// # Example Usage
    /// Suitable for scenarios requiring batch processing where a certain number of messages are needed before processing begins.
    #[inline]
    pub async fn wait_avail_units(& mut self, count: usize) -> bool {
        #[cfg(debug_assertions)]
        self.direct_use_check_and_warn();
        self.shared_wait_avail_units(count).await;
        false
    }
    //////////////////////////////////////////////////////////////////
    //////////////////////////////////////////////////////////////////

    fn direct_use_check_and_warn(&self) {
        if self.channel_meta_data.expects_to_be_monitored {
            write_warning_to_console(backtrace::Backtrace::new());

        }
    }

    ///////////////////////////////////////////////////////////////////
    // these are the shared internal private implementations
    // if you want to swap out the channel implementation you can do it here
    ///////////////////////////////////////////////////////////////////

    #[inline]
    fn shared_capacity(&self) -> usize {
        self.rx.capacity().get()
    }

    #[inline]
    fn shared_try_peek_slice(&self, elems: &mut [T]) -> usize
        where T: Copy {

        // TODO: rewrite when the new version is out

        let mut last_index = 0;
        for (i, e) in self.rx.iter().enumerate() {
            if i < elems.len() {
                elems[i] = *e; // Assuming e is a reference and needs dereferencing
                last_index = i;
            } else {
                break;
            }
        }
        // Return the count of elements written, adjusted for 0-based indexing
        last_index + 1
    }

    #[inline]
    async fn shared_peek_async_slice(&mut self, wait_for_count: usize, elems: &mut [T]) -> usize
        where T: Copy {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.rx.wait_occupied(wait_for_count);
        select! { _ = one_down => {}, _ = operation => {}, };
        self.shared_try_peek_slice(elems)
    }

    #[inline]
    fn shared_take_slice(&mut self, elems: &mut [T]) -> usize
        where T: Copy {
        self.rx.pop_slice(elems)
    }

    /*
    #[inline]
    fn shared_take_into_iter(&mut self) -> impl Iterator<Item = & T>
        where T: Copy {
        //TODO: may need to wait until next release.

        let len = self.rx.occupied_len()

        //(0..len)
        //let item: Option<T> = self.rx.try_pop();


    } */


    #[inline]
    async fn shared_take_async(& mut self) -> Option<T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.rx.pop();
        select! { _ = one_down => self.rx.try_pop(), p = operation => p, }
    }

    #[inline]
    fn shared_try_peek_iter(& self) -> impl Iterator<Item = & T>  {
        self.rx.iter()
    }

    #[inline]
    async fn shared_peek_async(& mut self) -> Option<&T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.rx.wait_occupied(1);
        select! { _ = one_down => {}, _ = operation => {}, };
        self.rx.first()
    }

    #[inline]
    async fn shared_peek_async_iter(& mut self, wait_for_count: usize) -> impl Iterator<Item = & T> {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.rx.wait_occupied(wait_for_count);
        select! { _ = one_down => {}, _ = operation => {}, };
        self.rx.iter()
    }

    #[inline]
    fn shared_is_empty(&self) -> bool {
        self.rx.is_empty()
    }
    #[inline]
    fn shared_avail_units(& mut self) -> usize {
        self.rx.occupied_len()
    }

    #[inline]
    async fn shared_wait_avail_units(&mut self, count: usize) -> bool {
        let mut one_down = &mut self.oneshot_shutdown;
        let mut operation = &mut self.rx.wait_occupied(count);
        select! { _ = one_down => false, _ = operation => true}
    }

    #[inline]
    fn shared_try_take(& mut self) -> Option<T> {
        self.rx.try_pop()
    }

    #[inline]
    fn shared_try_peek(&self) -> Option<&T> {
        self.rx.first()
    }

}


pub trait TxDef: Debug {
    fn meta_data(&self) -> TxMetaData;
}
pub trait RxDef: Debug + Send {
    fn meta_data(&self) -> RxMetaData;
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, ()>;

    }

//BoxFuture

impl <T> TxDef for SteadyTx<T> {
    fn meta_data(&self) -> TxMetaData {
        let guard = bastion::run!(self.lock());
        let this = guard.deref();
        TxMetaData(this.channel_meta_data.clone())
    }
}

impl <T: Send + Sync > RxDef for SteadyRx<T>  {
    fn meta_data(&self) -> RxMetaData {
        let guard = bastion::run!(self.lock());
        let this = guard.deref();
        RxMetaData(this.channel_meta_data.clone())
    }

    #[inline]
    fn wait_avail_units(&self, count: usize) -> BoxFuture<'_, ()> {
        async move {
            let mut guard = self.lock().await;
            guard.deref_mut().shared_wait_avail_units(count).await;
        }.boxed() // Use the `.boxed()` method to convert the future into a BoxFuture
    }


}


//TODO: better design with full lock feature for simple in place use!!!!!

pub trait SteadyTxBundleTrait<T,const GIRTH:usize> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>>;
    fn def_slice(&self) -> [& dyn TxDef; GIRTH];
    fn meta_data(&self) -> [TxMetaData; GIRTH];
    fn wait_vacant_units(&self
                                  , avail_count: usize
                                  , ready_channels: usize) -> impl std::future::Future<Output = ()> + Send;
}




impl<T: Sync+Send, const GIRTH: usize> SteadyTxBundleTrait<T, GIRTH> for SteadyTxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Tx<T>>> {
        //by design we always get the locks in the same order
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }

    fn def_slice(&self) -> [& dyn TxDef; GIRTH] {
        self.iter()
            .map(|x| x as &dyn TxDef)
            .collect::<Vec<&dyn TxDef>>()
            .try_into()
            .expect("Internal Error")
    }

    fn meta_data(&self) -> [TxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data())
            .collect::<Vec<TxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }



    async fn wait_vacant_units(&self
                                                          , avail_count: usize
                                                          , ready_channels: usize)
        {
        let futures = self.iter().map(|tx| {
            let tx = tx.clone();
            async move {
                let mut tx = tx.lock().await;
                tx.wait_vacant_units(avail_count).await;
            }.boxed() // Box the future to make them the same type
        });
        let mut futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);

        while !futures.is_empty() {
            // Wait for the first future to complete
            let (_result, _index, remaining) = select_all(futures).await;
            futures = remaining;
            count_down -= 1;
            if 0 == count_down {
                break;
            }
        }
    }

}

pub trait SteadyRxBundleTrait<T, const GIRTH: usize> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>>;
    fn def_slice(&self) -> [& dyn RxDef; GIRTH];
    fn meta_data(&self) -> [RxMetaData; GIRTH];

    fn wait_avail_units(&self
                                 , avail_count: usize
                                 , ready_channels: usize) -> impl std::future::Future<Output = ()> + Send;
}
impl<T: std::marker::Send + std::marker::Sync, const GIRTH: usize> crate::SteadyRxBundleTrait<T, GIRTH> for SteadyRxBundle<T, GIRTH> {
    fn lock(&self) -> futures::future::JoinAll<MutexLockFuture<'_, Rx<T>>> {
        //by design we always get the locks in the same order
        futures::future::join_all(self.iter().map(|m| m.lock()))
    }
    fn def_slice(&self) -> [& dyn RxDef; GIRTH]
    {
        self.iter()
            .map(|x| x as &dyn RxDef)
            .collect::<Vec<&dyn RxDef>>()
            .try_into()
            .expect("Internal Error")
    }

    fn meta_data(&self) -> [RxMetaData; GIRTH] {
        self.iter()
            .map(|x| x.meta_data())
            .collect::<Vec<RxMetaData>>()
            .try_into()
            .expect("Internal Error")
    }
    async fn wait_avail_units(&self
                                                         , avail_count: usize
                                                         , ready_channels: usize)
         {
        let futures = self.iter().map(|rx| {
            let rx = rx.clone();
            async move {
                let mut guard = rx.lock().await;
                guard.wait_avail_units(avail_count).await;
            }
                .boxed() // Box the future to make them the same type
        });

        let futures: Vec<_> = futures.collect();

        let mut count_down = ready_channels.min(GIRTH);
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

    }




}


pub trait RxBundleTrait {
    fn is_closed(&mut self) -> bool;
}
impl<T> crate::RxBundleTrait for RxBundle<'_, T> {
    fn is_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.is_closed() )
    }
}


pub trait TxBundleTrait {
    fn mark_closed(&mut self) -> bool;
}
impl<T> crate::TxBundleTrait for TxBundle<'_, T> {
    fn mark_closed(&mut self) -> bool {
        self.iter_mut().all(|f| f.mark_closed() )
    }
}




#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StdDev(f32);

impl StdDev {
    // Private constructor to directly set the value inside the struct.
    // This is private to ensure that all public constructors go through validation.
    fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn one() -> Self {
        Self(1.0)
    }

    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    pub fn two() -> Self {
        Self(2.0)
    }

    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    pub fn three() -> Self {
        Self(3.0)
    }

    pub fn four() -> Self {
        Self(4.0)
    }

    // Allows custom values within the valid range.
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    // Getter to access the inner f32 value.
    pub fn value(&self) -> f32 {
        self.0
    }
}

////////////////

////////////////////
pub trait Metric {
}
pub trait DataMetric: Metric {
}
pub trait ComputeMetric: Metric {
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertColor {
    Yellow,
    Orange,
    Red,
}

impl Metric for Duration {}

#[derive(Clone, Copy, Debug)]
pub enum Trigger<T> where T: Metric {
    AvgAbove(T),
    AvgBelow(T),
    StdDevsAbove(StdDev, T), // above mean+(std*factor)
    StdDevsBelow(StdDev, T), // below mean-(std*factor)
    PercentileAbove(Percentile, T),
    PercentileBelow(Percentile, T),
}

///////////////////////////////

