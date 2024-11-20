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
mod abstract_executor;
#[cfg(test)]
mod test_panic_capture;
mod monitor_telemetry;
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

}
    /// module for testing full graphs of actors
pub mod graph_testing;
    /// module for all tx channel features
pub mod steady_tx;
    /// module for all rx channel features
pub mod steady_rx;
    /// module to yield in our actor
pub mod yield_now;
    /// module for all commands for channels used by actors
pub mod commander;

pub use graph_testing::GraphTestResult;
pub use monitor::LocalMonitor;
pub use channel_builder::Rate;
pub use channel_builder::Filled;
pub use channel_builder::LazySteadyRx;
pub use channel_builder::LazySteadyTx;
pub use actor_builder::MCPU;
pub use actor_builder::Work;
pub use actor_builder::Percentile;
pub use actor_builder::ActorTeam;
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
pub use commander::SteadyCommander;


use std::any::Any;
use std::time::{Duration, Instant};

use std::fmt::Debug;
use std::sync::Arc;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, AtomicUsize};
use futures::lock::Mutex;
use std::ops::DerefMut;
use std::pin::Pin;
#[allow(unused_imports)]
use log::*;

use crate::monitor::{ActorMetaData, ChannelMetaData};
use crate::telemetry::metrics_collector::CollectorDetail;
use crate::telemetry::setup;
use crate::util::steady_logging_init;
use futures::*;
use futures::channel::oneshot;
use futures::select;
use futures_util::future::{FusedFuture};
use futures_util::lock::MutexGuard;
use monitor_telemetry::SteadyTelemetry;
use crate::actor_builder::NodeTxRx;
use crate::yield_now::yield_now;

/// Type alias for a thread-safe steady state (S) wrapped in an `Arc` and `Mutex`.
///
/// Holds state of actors so it is not lost between restarts.
pub type SteadyState<S> = Arc<Mutex<Option<S>>>;

/// Create new SteadyState struct for holding state of actors across panics restarts.
/// Should only be called in main when creating the actors
///
pub fn new_state<S>() -> SteadyState<S> {
    Arc::new(Mutex::new(None))
}

/// Lock for use the SteadyState struct which holds state of an actor across panics restarts.
pub async fn steady_state<F,S>(steadystate: & SteadyState<S>, build_new_state: F) -> MutexGuard<Option<S>>
where
    S: Clone,
    F: FnOnce() -> S {
    let mut state_guard = steadystate.lock().await;
    *state_guard = Some(match state_guard.take() {
                            Some(s) => s.clone(),
                            None => build_new_state()
                        });
    state_guard
}

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

/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
/// This one is special because its clone will lazy create teh channels.
/// 
pub trait LazySteadyTxBundleClone<T, const GIRTH: usize> {
    /// Clone the bundle of transmitters. But MORE. This is the lazy init of the channel as well.
    fn clone(&self) -> SteadyTxBundle<T, GIRTH>;

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

}

/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
///
/// This type alias simplifies the usage of a bundle of receivers that can be shared across multiple threads.
pub type LazySteadyRxBundle<T, const GIRTH: usize> = [LazySteadyRx<T>; GIRTH];

/// Type alias for an array of thread-safe receivers (Rx) with a fixed size (GIRTH), wrapped in an `Arc`.
pub trait LazySteadyRxBundleClone<T, const GIRTH: usize> {
    /// Clone the bundle of receivers. But MORE. This is the lazy init of the channel as well.
    /// Use it as a normal clone the lazy init happens as an implementation detail.
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
///
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
///
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
    pub(crate) node_tx_rx: Option<Arc<NodeTxRx>>,
    pub(crate) frame_rate_ms: u64,
    pub(crate) team_id: usize,
    pub(crate) show_thread_info: bool
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
#[derive(Default,PartialEq,Eq,Debug)]
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

#[cfg(test)]
mod lib_tests {
    use super::*;
    use std::sync::Arc;
    use futures::channel::oneshot;
    use std::time::Instant;
    use std::sync::atomic::AtomicUsize;
    use crate::channel_builder::ChannelBuilder;
    use parking_lot::RwLock;
    use futures::lock::Mutex;
    use commander::SteadyCommander;

    #[test]
    fn test_std_dev_valid_values() {
        // Valid range: (0.0, 10.0)
        assert_eq!(StdDev::new(0.1), Some(StdDev(0.1)));
        assert_eq!(StdDev::new(1.0), Some(StdDev::one()));
        assert_eq!(StdDev::new(1.5), Some(StdDev::one_and_a_half()));
        assert_eq!(StdDev::new(2.0), Some(StdDev::two()));
        assert_eq!(StdDev::new(2.5), Some(StdDev::two_and_a_half()));
        assert_eq!(StdDev::new(3.0), Some(StdDev::three()));
        assert_eq!(StdDev::new(4.0), Some(StdDev::four()));
        assert_eq!(StdDev::new(5.0), Some(StdDev(5.0)));
        assert_eq!(StdDev::new(9.9), Some(StdDev(9.9)));


    }

    #[test]
    fn test_std_dev_invalid_values() {
        // Invalid range: <= 0.0 or >= 10.0
        assert_eq!(StdDev::new(0.0), None);
        assert_eq!(StdDev::new(10.0), None);
        assert_eq!(StdDev::new(10.1), None);
        assert_eq!(StdDev::new(-1.0), None);
    }

    #[test]
    fn test_std_dev_predefined_values() {
        // Test predefined StdDev values
        assert_eq!(StdDev::one(), StdDev(1.0));
        assert_eq!(StdDev::one_and_a_half(), StdDev(1.5));
        assert_eq!(StdDev::two(), StdDev(2.0));
        assert_eq!(StdDev::two_and_a_half(), StdDev(2.5));
        assert_eq!(StdDev::three(), StdDev(3.0));
        assert_eq!(StdDev::four(), StdDev(4.0));
    }

    #[test]
    fn test_std_dev_custom() {
        // Valid custom value
        assert_eq!(StdDev::custom(3.3), Some(StdDev(3.3)));
        // Invalid custom value
        assert_eq!(StdDev::custom(10.5), None);
    }

    #[test]
    fn test_std_dev_value() {
        // Test value retrieval
        let std_dev = StdDev(2.5);
        assert_eq!(std_dev.value(), 2.5);
    }

    #[test]
    fn test_new_state() {
        let state: SteadyState<i32> = new_state();
        assert!(state.try_lock().is_some());
        let guard = state.try_lock().unwrap();
        assert!(guard.is_none());
    }

    #[async_std::test]
    async fn test_steady_state_function() {
        let state: SteadyState<i32> = new_state();
        let guard = steady_state(&state, || 42).await;
        assert_eq!(*guard, Some(42));
    }
    // 
    // #[test]
    // fn test_init_logging() {
    //     let result = init_logging("info");
    //     assert!(result.is_ok());
    // }

    #[test]
    fn test_args_method() {
        let context = test_steady_context();
        let args = context.args::<()>();
        assert!(args.is_some());
    }

    #[test]
    fn test_identity_method() {
        let context = test_steady_context();
        let identity = context.identity();
        assert_eq!(identity.id, 0);
        assert_eq!(identity.label.name, "test_actor");
    }

    // #[test]
    // fn test_request_graph_stop() {
    //     let context = test_steady_context();
    //     let result = context.request_graph_stop();
    //     assert!(result);
    //     let liveliness = context.runtime_state.read();
    //     assert!(liveliness.is_in_state(&[GraphLivelinessState::StopRequested]));
    // }

    // #[test]
    // fn test_is_liveliness_in() {
    //     let context = test_steady_context();
    //     let result = context.is_liveliness_in(&[GraphLivelinessState::Running]);
    //     assert!(result);
    // }

    #[async_std::test]
    async fn test_wait_shutdown() {
        let context = test_steady_context();
        // To ensure the test completes, we manually trigger the shutdown signal.
        let mut shutdown_vec = context.oneshot_shutdown_vec.lock().await;
        if let Some(sender) = shutdown_vec.pop() {
            let _ = sender.send(());
        }
        let result = context.wait_shutdown().await;
        assert!(result);
    }

    #[test]
    fn test_sidechannel_responder() {
        let context = test_steady_context();
        let responder = context.sidechannel_responder();
        assert!(responder.is_none());
    }

    #[async_std::test]
    async fn test_send_async() {
        let (tx, _rx) = create_test_channel::<i32>();
        let mut context = test_steady_context();
        let tx = tx.clone();
        let guard = tx.try_lock();
        if let Some(mut tx_guard) = guard {
            let result = context
                .send_async(&mut tx_guard, 42, SendSaturation::Warn)
                .await;
            assert!(result.is_ok());
        }
    }

    // #[async_std::test]
    // async fn test_wait_periodic() {
    //     let context = test_steady_context();
    //     // Ensure that the method returns by limiting the wait duration
    //     let result = context.wait_periodic(Duration::from_millis(10)).await;
    //     assert!(result);
    // }

    #[test]
    fn test_into_monitor_macro() {
        // Prepare context and channels
        let context = test_steady_context();
        let (tx, rx) = create_test_channel::<i32>();
        // Use the macro
        let _monitor = into_monitor!(context, [rx.clone()], [tx.clone()]);
        // Since we cannot directly test the internal state of the monitor,
        // we ensure that the macro compiles and runs without errors
        assert!(true);
    }

    // Helper method to build tx and rx arguments
    fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyContext {
        let (_tx, rx) = build_tx_rx();
        SteadyContext {
            runtime_state: Arc::new(RwLock::new(GraphLiveliness::new(
                Default::default(),
                Default::default()
            ))),
            channel_count: Arc::new(AtomicUsize::new(0)),
            ident: ActorIdentity::new(0, "test_actor", None),
            args: Arc::new(Box::new(())),
            all_telemetry_rx: Arc::new(RwLock::new(Vec::new())),
            actor_metadata: Arc::new(ActorMetaData::default()),
            oneshot_shutdown_vec: Arc::new(Mutex::new(Vec::new())),
            oneshot_shutdown: Arc::new(Mutex::new(rx)),
            node_tx_rx: None,
            instance_id: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            show_thread_info: false,
            team_id: 0,
        }
    }

    fn create_rx<T: std::fmt::Debug>(data: Vec<T>) -> Arc<Mutex<Rx<T>>> {
        let (tx, rx) = create_test_channel();

        let send = tx.clone();
        if let Some(ref mut send_guard) = send.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        rx.clone()
    }

    fn create_tx<T: std::fmt::Debug>(data: Vec<T>) -> Arc<Mutex<Tx<T>>> {
        let (tx, _rx) = create_test_channel();

        let send = tx.clone();
        if let Some(ref mut send_guard) = send.try_lock() {
            for item in data {
                let _ = send_guard.shared_try_send(item);
            }
        }
        tx.clone()
    }

    fn create_test_channel<T: std::fmt::Debug>() -> (LazySteadyTx<T>, LazySteadyRx<T>) {
        let builder = ChannelBuilder::new(
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            Instant::now(),
            40);

        builder.build::<T>()
    }

    // Test for try_peek
    #[test]
    fn test_try_peek() {
        let rx = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let result = context.try_peek(&mut rx);
            assert_eq!(result, Some(&1));
        };
    }

    // Test for take_slice
    #[test]
    fn test_take_slice() {
        let rx = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let mut context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let count = context.take_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }

    // Test for try_peek_slice
    #[test]
    fn test_try_peek_slice() {
        let rx = create_rx(vec![1, 2, 3, 4, 5]);
        let mut slice = [0; 3];
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let count = context.try_peek_slice(&mut rx, &mut slice);
            assert_eq!(count, 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }



    // Test wait_while_running method
    #[async_std::test]
    async fn test_wait_while_running() {
        let context = test_steady_context();
        let fut = context.wait_while_running();
        assert_eq!(fut.await, Ok(()));

    }

    // Test wait_avail_units_bundle method
    #[async_std::test]
    async fn test_wait_avail_units_bundle() {
        let context = test_steady_context();
        let mut rx_bundle = RxBundle::<i32>::new();
        let fut = context.wait_shutdown_or_avail_units_bundle(&mut rx_bundle, 1, 1);
        assert!(fut.await);
    }

    // Test wait_avail_units_bundle method
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_bundle() {
        let context = test_steady_context();
        let mut rx_bundle = RxBundle::<i32>::new();
        let fut = context.wait_closed_or_avail_units_bundle(&mut rx_bundle, 1, 1);
        assert!(fut.await);
    }

    // Test wait_vacant_units_bundle method
    #[async_std::test]
    async fn test_wait_vacant_units_bundle() {
        let context = test_steady_context();
        let mut tx_bundle = TxBundle::<i32>::new();
        let fut = context.wait_shutdown_or_vacant_units_bundle(&mut tx_bundle, 1, 1);
        assert!(fut.await);

    }

    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units() {
        let context = test_steady_context();
        let rx = create_rx::<i32>(vec![1, 2, 3]);
        let guard = rx.try_lock();
        if let Some(mut rx) = guard {
            let result = context.wait_shutdown_or_avail_units(&mut rx, 2).await;
            assert!(result); // Ensure it waits for units or shutdown
        }
    }

    #[async_std::test]
    async fn test_wait_closed_or_avail_units() {
        let context = test_steady_context();
        let rx = create_rx::<i32>(vec![1, 2, 3]);
        let guard = rx.try_lock();
        if let Some(mut rx) = guard {
            let result = context.wait_closed_or_avail_units(&mut rx, 2).await;
            assert!(result); // Ensure it waits for availability or closure
        }
    }

    // #[async_std::test]
    // async fn test_wait_avail_units() {
    //     let context = test_steady_context();
    //     let rx = create_rx::<i32>(vec![]);
    //     let guard = rx.try_lock();
    //     if let Some(mut rx) = guard {
    //         let result = context.wait_avail_units(&mut rx, 1).await;
    //         assert!(!result); // Ensure availability waiting works
    //     }
    // }

    #[async_std::test]
    async fn test_wait_shutdown_or_vacant_units() {
        let context = test_steady_context();
        let tx = create_tx::<i32>(vec![]);
        let guard = tx.try_lock();
        if let Some(mut tx) = guard {
            let result = context.wait_shutdown_or_vacant_units(&mut tx, 2).await;
            assert!(result); // Should succeed if vacant units or shutdown occur
        }
    }

    #[async_std::test]
    async fn test_wait_vacant_units() {
        let context = test_steady_context();
        let tx = create_tx::<i32>(vec![]);
        let guard = tx.try_lock();
        if let Some(mut tx) = guard {
            let result = context.wait_vacant_units(&mut tx, 1).await;
            assert!(result); // Ensure it waits for vacancy correctly
        }
    }

    // #[async_std::test]
    // async fn test_wait_future_void() {
    //     let context = test_steady_context();
    //     let fut = async { /* Simulate a task */ };
    //     let result = context.wait_future_void(Box::pin(fut.fuse())).await;
    //     assert!(result); // Ensure it handles shutdown while waiting for a future
    // }


    // Test is_empty method
    #[test]
    fn test_is_empty() {
        let context = test_steady_context();
        let rx = create_rx::<String>(vec![]); // Creating an empty Rx
        if let Some(mut rx) = rx.try_lock() {
            assert!(context.is_empty(&mut rx));
        };
    }

    // Test avail_units method
    #[test]
    fn test_avail_units() {
        let context = test_steady_context();
        let rx = create_rx(vec![1, 2, 3]);
        if let Some(mut rx) = rx.try_lock() {
            assert_eq!(context.avail_units(&mut rx), 3);
        };
    }

    // Test for try_peek_iter
    #[test]
    fn test_try_peek_iter() {
        let rx = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let mut iter = context.try_peek_iter(&mut rx);
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async_iter
    #[async_std::test]
    async fn test_peek_async_iter() {
        let rx = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let mut iter = context.peek_async_iter(&mut rx, 3).await;
            assert_eq!(iter.next(), Some(&1));
            assert_eq!(iter.next(), Some(&2));
            assert_eq!(iter.next(), Some(&3));
        };
    }

    // Test for peek_async
    #[async_std::test]
    async fn test_peek_async() {
        let rx = create_rx(vec![1, 2, 3]);
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let result = context.peek_async(&mut rx).await;
            assert_eq!(result, Some(&1));
        };
    }

    // Test for send_slice_until_full
    #[test]
    fn test_send_slice_until_full() {
        let (tx, _rx) = create_test_channel();
        let mut context = test_steady_context();
        let slice = [1, 2, 3];
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = context.send_slice_until_full(&mut tx, &slice);
            assert_eq!(sent_count, slice.len());
        };
    }

    // Test for send_iter_until_full
    #[test]
    fn test_send_iter_until_full() {
        let (tx, _rx) = create_test_channel();
        let mut context = test_steady_context();
        let iter = vec![1, 2, 3].into_iter();
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let sent_count = context.send_iter_until_full(&mut tx, iter);
            assert_eq!(sent_count, 3);
        };
    }

    // Test for try_send
    #[test]
    fn test_try_send() {
        let (tx, _rx) = create_test_channel();
        let mut context = test_steady_context();
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let result = context.try_send(&mut tx, 42);
            assert!(result.is_ok());
        };
    }

    // Test for is_full
    #[test]
    fn test_is_full() {
        let (tx, _rx) = create_test_channel::<String>();
        let context = test_steady_context();
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            assert!(!context.is_full(&mut tx));
        };
    }

    // Test for vacant_units
    #[test]
    fn test_vacant_units() {
        let (tx, _rx) = create_test_channel::<String>();
        let context = test_steady_context();
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let vacant_units = context.vacant_units(&mut tx);
            assert_eq!(vacant_units, 64); // Assuming only one unit can be vacant
        };
    }

    // Test for wait_empty
    #[async_std::test]
    async fn test_wait_empty() {
        let (tx, _rx) = create_test_channel::<String>();
        let context = test_steady_context();
        let tx  = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let empty = context.wait_empty(&mut tx).await;
            assert!(empty);
        };
    }

    // Test for take_into_iter
    #[test]
    fn test_take_into_iter() {
        let rx = create_rx(vec![1, 2, 3]);
        let mut context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let mut iter = context.take_into_iter(&mut rx);
            assert_eq!(iter.next(), Some(1));
            assert_eq!(iter.next(), Some(2));
            assert_eq!(iter.next(), Some(3));
        };
    }

    // Test for call_async
    #[async_std::test]
    async fn test_call_async() {
        let context = test_steady_context();
        let fut = async { 42 };
        let result = context.call_async(fut).await;
        assert_eq!(result, Some(42));
    }

}

#[cfg(test)]
mod enum_tests {
    use crate::channel_builder::{ChannelBuilder, LazyChannel};
    use super::*;

    #[test]
    fn test_send_saturation_default() {
        let saturation = SendSaturation::default();
        assert_eq!(saturation, SendSaturation::Warn);
    }

    #[test]
    fn test_send_saturation_variants() {
        let wait = SendSaturation::IgnoreAndWait;
        let err = SendSaturation::IgnoreAndErr;
        let warn = SendSaturation::Warn;
        let ignore = SendSaturation::IgnoreInRelease;

        match wait {
            SendSaturation::IgnoreAndWait => assert!(true),
            _ => assert!(false, "Expected IgnoreAndWait"),
        }

        match err {
            SendSaturation::IgnoreAndErr => assert!(true),
            _ => assert!(false, "Expected IgnoreAndErr"),
        }

        match warn {
            SendSaturation::Warn => assert!(true),
            _ => assert!(false, "Expected Warn"),
        }

        match ignore {
            SendSaturation::IgnoreInRelease => assert!(true),
            _ => assert!(false, "Expected IgnoreInRelease"),
        }
    }

    #[test]
    fn test_std_dev_creation() {
        let valid_std_dev = StdDev::new(5.0);
        assert_eq!(valid_std_dev, Some(StdDev(5.0)));

        let invalid_std_dev = StdDev::new(10.5);
        assert_eq!(invalid_std_dev, None);
    }

    #[test]
    fn test_std_dev_predefined() {
        assert_eq!(StdDev::one(), StdDev(1.0));
        assert_eq!(StdDev::one_and_a_half(), StdDev(1.5));
        assert_eq!(StdDev::two(), StdDev(2.0));
        assert_eq!(StdDev::two_and_a_half(), StdDev(2.5));
        assert_eq!(StdDev::three(), StdDev(3.0));
        assert_eq!(StdDev::four(), StdDev(4.0));
    }

    #[test]
    fn test_std_dev_custom() {
        let std_dev = StdDev::custom(2.5);
        assert_eq!(std_dev, Some(StdDev(2.5)));

        let invalid_std_dev = StdDev::custom(10.1);
        assert_eq!(invalid_std_dev, None);
    }

    #[test]
    fn test_std_dev_value() {
        let std_dev = StdDev(3.5);
        assert_eq!(std_dev.value(), 3.5);
    }

    #[test]
    fn test_alert_color_variants() {
        let yellow = AlertColor::Yellow;
        let orange = AlertColor::Orange;
        let red = AlertColor::Red;

        assert_eq!(yellow, AlertColor::Yellow);
        assert_eq!(orange, AlertColor::Orange);
        assert_eq!(red, AlertColor::Red);
    }

    #[test]
    fn test_trigger_variants() {
        use std::time::Duration;

        let avg_above = Trigger::AvgAbove(Duration::from_secs(1));
        let avg_below = Trigger::AvgBelow(Duration::from_secs(2));
        let std_devs_above = Trigger::StdDevsAbove(StdDev::two(), Duration::from_secs(3));
        let std_devs_below = Trigger::StdDevsBelow(StdDev::one(), Duration::from_secs(4));
        let percentile_above = Trigger::PercentileAbove(Percentile(90.0), Duration::from_secs(5));
        let percentile_below = Trigger::PercentileBelow(Percentile(10.0), Duration::from_secs(6));

        match avg_above {
            Trigger::AvgAbove(val) => assert_eq!(val, Duration::from_secs(1)),
            _ => assert!(false, "Expected AvgAbove"),
        }

        match avg_below {
            Trigger::AvgBelow(val) => assert_eq!(val, Duration::from_secs(2)),
            _ => assert!(false, "Expected AvgBelow"),
        }

        match std_devs_above {
            Trigger::StdDevsAbove(std_dev, val) => {
                assert_eq!(std_dev, StdDev::two());
                assert_eq!(val, Duration::from_secs(3));
            },
            _ => assert!(false, "Expected StdDevsAbove"),
        }

        match std_devs_below {
            Trigger::StdDevsBelow(std_dev, val) => {
                assert_eq!(std_dev, StdDev::one());
                assert_eq!(val, Duration::from_secs(4));
            },
            _ => assert!(false, "Expected StdDevsBelow"),
        }

        match percentile_above {
            Trigger::PercentileAbove(percentile, val) => {
                assert_eq!(percentile, Percentile(90.0));
                assert_eq!(val, Duration::from_secs(5));
            },
            _ => assert!(false, "Expected PercentileAbove"),
        }

        match percentile_below {
            Trigger::PercentileBelow(percentile, val) => {
                assert_eq!(percentile, Percentile(10.0));
                assert_eq!(val, Duration::from_secs(6));
            },
            _ => assert!(false, "Expected PercentileBelow"),
        }
    }

    #[test]
    fn test_lazy_steady_tx_bundle_clone() {
        // Define a simple data type for the test
        type TestType = i32;

        let cb = ChannelBuilder::new(
                    Arc::new(Default::default()),
                    Arc::new(Default::default()),
                    Instant::now(),
                    40,
        );

        let lazy1 = Arc::new(LazyChannel::new(&cb));
        let lazy2 = Arc::new(LazyChannel::new(&cb));
        let lazy3 = Arc::new(LazyChannel::new(&cb));

        // Create a LazySteadyTxBundle with a fixed size of 3
        let tx1 = LazySteadyTx::<TestType>::new(lazy1.clone());
        let tx2 = LazySteadyTx::<TestType>::new(lazy2.clone());
        let tx3 = LazySteadyTx::<TestType>::new(lazy3.clone());
        let lazy_bundle: LazySteadyTxBundle<TestType, 3> = [tx1, tx2, tx3];

        // Clone the LazySteadyTxBundle
        let cloned_bundle = lazy_bundle.clone();

        assert_eq!(
            Arc::as_ptr(&cloned_bundle[0]),
            Arc::as_ptr(&lazy_bundle[0].clone())
        );
        assert_eq!(
            Arc::as_ptr(&cloned_bundle[1]),
            Arc::as_ptr(&lazy_bundle[1].clone())
        );
        assert_eq!(
            Arc::as_ptr(&cloned_bundle[2]),
            Arc::as_ptr(&lazy_bundle[2].clone())
        );
    }

    #[test]
    fn test_lazy_steady_rx_bundle_clone() {
        // Define a simple data type for the test
        type TestType = i32;

        let cb = ChannelBuilder::new(
            Arc::new(Default::default()),
            Arc::new(Default::default()),
            Instant::now(),
            40,
        );

        let lazy1 = Arc::new(LazyChannel::new(&cb));
        let lazy2 = Arc::new(LazyChannel::new(&cb));
        let lazy3 = Arc::new(LazyChannel::new(&cb));

        // Create a LazySteadyTxBundle with a fixed size of 3
        let rx1 = LazySteadyRx::<TestType>::new(lazy1.clone());
        let rx2 = LazySteadyRx::<TestType>::new(lazy2.clone());
        let rx3 = LazySteadyRx::<TestType>::new(lazy3.clone());
        let lazy_bundle: LazySteadyRxBundle<TestType, 3> = [rx1, rx2, rx3];

        // Clone the LazySteadyTxBundle
        let cloned_bundle = lazy_bundle.clone();

        assert_eq!(
            Arc::as_ptr(&cloned_bundle[0]),
            Arc::as_ptr(&lazy_bundle[0].clone())
        );
        assert_eq!(
            Arc::as_ptr(&cloned_bundle[1]),
            Arc::as_ptr(&lazy_bundle[1].clone())
        );
        assert_eq!(
            Arc::as_ptr(&cloned_bundle[2]),
            Arc::as_ptr(&lazy_bundle[2].clone())
        );
    }


    #[test]
    fn test_alert_color_equality() {
        assert_eq!(AlertColor::Yellow, AlertColor::Yellow);
        assert_eq!(AlertColor::Orange, AlertColor::Orange);
        assert_eq!(AlertColor::Red, AlertColor::Red);
    }

    #[test]
    fn test_alert_color_inequality() {
        assert_ne!(AlertColor::Yellow, AlertColor::Orange);
        assert_ne!(AlertColor::Orange, AlertColor::Red);
        assert_ne!(AlertColor::Red, AlertColor::Yellow);
    }

    #[test]
    fn test_trigger_avg_above() {
        let trigger = Trigger::AvgAbove(Duration::from_secs(1));
        if let Trigger::AvgAbove(val) = trigger {
            assert_eq!(val, Duration::from_secs(1));
        } else {
            panic!("Expected Trigger::AvgAbove");
        }
    }

    #[test]
    fn test_trigger_avg_below() {
        let trigger = Trigger::AvgBelow(Duration::from_secs(2));
        if let Trigger::AvgBelow(val) = trigger {
            assert_eq!(val, Duration::from_secs(2));
        } else {
            panic!("Expected Trigger::AvgBelow");
        }
    }

    #[test]
    fn test_trigger_std_devs_above() {
        let trigger = Trigger::StdDevsAbove(StdDev::two(), Duration::from_secs(3));
        if let Trigger::StdDevsAbove(std_dev, val) = trigger {
            assert_eq!(std_dev, StdDev::two());
            assert_eq!(val, Duration::from_secs(3));
        } else {
            panic!("Expected Trigger::StdDevsAbove");
        }
    }

    #[test]
    fn test_trigger_std_devs_below() {
        let trigger = Trigger::StdDevsBelow(StdDev::one(), Duration::from_secs(4));
        if let Trigger::StdDevsBelow(std_dev, val) = trigger {
            assert_eq!(std_dev, StdDev::one());
            assert_eq!(val, Duration::from_secs(4));
        } else {
            panic!("Expected Trigger::StdDevsBelow");
        }
    }

    #[test]
    fn test_trigger_percentile_above() {
        let trigger = Trigger::PercentileAbove(Percentile(90.0), Duration::from_secs(5));
        if let Trigger::PercentileAbove(percentile, val) = trigger {
            assert_eq!(percentile, Percentile(90.0));
            assert_eq!(val, Duration::from_secs(5));
        } else {
            panic!("Expected Trigger::PercentileAbove");
        }
    }

    #[test]
    fn test_trigger_percentile_below() {
        let trigger = Trigger::PercentileBelow(Percentile(10.0), Duration::from_secs(6));
        if let Trigger::PercentileBelow(percentile, val) = trigger {
            assert_eq!(percentile, Percentile(10.0));
            assert_eq!(val, Duration::from_secs(6));
        } else {
            panic!("Expected Trigger::PercentileBelow");
        }
    }


}