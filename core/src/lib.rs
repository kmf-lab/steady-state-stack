//! # Steady State Core - Easy Performant Async
//!  Steady State is a high performance, easy to use, actor based framework for building concurrent applications in Rust.
//!  Guarantee your SLA with telemetry, alerts and prometheus.
//!  Build low latency high volume solutions.

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
use std::cell::RefCell;

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
//use ringbuf::producer::Producer;
use async_ringbuf::producer::AsyncProducer;
//use ringbuf::traits::Observer;
//use ringbuf::consumer::Consumer;
use async_ringbuf::consumer::AsyncConsumer;
use async_ringbuf::traits::{Consumer, Observer, Producer};
use colored::Colorize;


use actor_builder::ActorBuilder;
use crate::monitor::{ActorMetaData, CALL_OTHER, CALL_SINGLE_READ, ChannelMetaData, RxMetaData, TxMetaData};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;// re-publish in public
use futures::*; // Provides the .fuse() method

//TODO: we must export a macro for blocking calls as needed.
//TODO: get big example to run.

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

pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;

pub type SteadyTxBundle<T,const GIRTH:usize> = Arc<[SteadyTx<T>;GIRTH]>;

pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;
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
    steady_logging_init(loglevel)
}


pub struct SteadyContext {
    pub(crate) ident: ActorIdentity,
    pub(crate) instance_id: u32,
    pub(crate) is_in_graph: bool,
    pub(crate) channel_count: Arc<AtomicUsize>,
    pub(crate) all_telemetry_rx: Arc<RwLock<Vec<CollectorDetail>>>,
    pub(crate) runtime_state: Arc<RwLock<GraphLiveliness>>,
    pub(crate) args: Arc<Box<dyn Any+Send+Sync>>,
    pub(crate) actor_metadata: Arc<ActorMetaData>,
    pub(crate) oneshot_shutdown_vec: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    pub(crate) oneshot_shutdown: Arc<Mutex<oneshot::Receiver<()>>>,
    pub(crate) last_periodic_wait: AtomicU64,
    pub(crate) actor_start_time: Instant,
    pub(crate) node_tx_rx: Option<Arc<Mutex<SideChannel>>>,
    pub(crate) frame_rate_ms: u64
}


impl SteadyContext {

    pub fn id(&self) -> usize {
        self.ident.id
    }
    pub fn name(&self) -> &str {
        self.ident.name
    }


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

    /// only needed if the channel consumer needs to know that a sender was restarted
    pub fn update_tx_instance<T>(&self, target: &mut Tx<T>) {
        target.tx_version.store(self.instance_id, Ordering::SeqCst);
    }
    /// only needed if the channel consumer needs to know that a sender was restarted
    pub fn update_tx_instance_bundle<T>(&self, target: &mut TxBundle<T>) {
        target.iter_mut().for_each(|tx| tx.tx_version.store(self.instance_id, Ordering::SeqCst));
    }


    /// only needed if the channel consumer needs to know that a receiver was restarted
    pub fn update_rx_instance<T>(&self, target: &mut Rx<T>) {
        target.rx_version.store(self.instance_id, Ordering::SeqCst);
    }
    /// only needed if the channel consumer needs to know that a receiver was restarted
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

    /// yield so other actors may be able to make use of this thread. Returns
    /// immediately if there is nothing scheduled to check.
    pub async fn yield_now(&self) {
        yield_now().await;
    }


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
    pub async fn send_async<T>(& mut self, this: & mut Tx<T>, a: T,saturation:SendSaturation ) -> Result<(), T> {
        #[cfg(debug_assertions)]
        this.direct_use_check_and_warn();
        this.shared_send_async(a, self.ident,saturation).await
    }

    pub fn try_take<T>(&self, this: & mut Rx<T>) -> Option<T> {
        this.shared_try_take()
    }

    pub async fn wait_avail_units<T>(& self, this: & mut Rx<T>, count:usize) -> bool {
        this.shared_wait_avail_units(count).await
    }

    pub async fn wait_vacant_units<T>(& self, this: & mut Tx<T>, count:usize) -> bool {
        this.shared_wait_vacant_units(count).await
    }

    pub fn edge_simulator(&self) -> Option<SideChannelResponder> {
        //if we have no back channel plane then we can not simulate edges
        self.node_tx_rx.as_ref().map(|node_tx_rx| SideChannelResponder::new(node_tx_rx.clone() ))
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

        //trace!("into monitor {:?}",self.ident);
        //only build telemetry channels if this feature is enabled
        let (send_rx, send_tx, state) = if config::TELEMETRY_HISTORY || config::TELEMETRY_SERVER {

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


      //  error!("return monitor {:?}",self.ident);

        // this is my fixed size version for this specific thread
        LocalMonitor::<RX_LEN, TX_LEN> {
            telemetry: SteadyTelemetry {
                send_rx, send_tx, state,
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
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        //this allows for 'arrays' of non homogneous types and avoids dyn
        let rx_meta = [$($rx.meta_data(),)*];
        let tx_meta = [$($tx.meta_data(),)*];

        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
    ($self:expr, [$($rx:expr),*], $tx_bundle:expr) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        //this allows for 'arrays' of non homogneous types and avoids dyn
        let rx_meta = [$($rx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, $tx_bundle.meta_data())
    }};
     ($self:expr, $rx_bundle:expr, [$($tx:expr),*] ) => {{
        #[allow(unused_imports)]
        use steady_rx::RxDef;
        #[allow(unused_imports)]
        use steady_tx::TxDef;
        //this allows for 'arrays' of non homogneous types and avoids dyn
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






const MONITOR_UNKNOWN: usize = usize::MAX;
const MONITOR_NOT: usize = MONITOR_UNKNOWN-1; //any value below this is a valid monitor index

/// common method to show stack trace
pub(crate) fn write_warning_to_console(dedupeset: &mut HashSet<String>) {

    let backtrace = Backtrace::capture();
    // Pretty print the backtrace if it's captured
    match backtrace.status() {
        BacktraceStatus::Captured => {
            //warn!("you called this without the monitor but monitoring for this channel is enabled. see backtrace and the the monitor version of this method");

            let backtrace_str = format!("{:#?}", backtrace);
            let mut call_seeker = 0;
            let mut tofix:String = "".to_string();
            let mut called:String = "".to_string();
            for line in backtrace_str.lines() {
                if line.contains("::direct_use_check_and_warn") {
                    call_seeker = 1;
                } else if 2== call_seeker { //we found it
                    if !line.contains("futures_util::future::") { //this is just a wrapper
                        if !line.contains("::pin::Pin") { //this is just a wrapper
                            tofix = line.to_string();
                            break;
                        }
                    }

                } else if 1== call_seeker {
                    called = line.to_string();
                    call_seeker = 2;
                }
            }

            //only report once for each location in the code.
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
    IgnoreInRelease
}


// TODO: bad message detection logic in progress.
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
            // Store or handle the hash as needed
            println!("Hash: {}", hash);
            Some(item)
        } else {
            None
        }
    }
}

//BoxFuture


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

// TODO: docs