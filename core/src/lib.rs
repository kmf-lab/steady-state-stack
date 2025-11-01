//!
//! # Steady State Core - Easy Performant Async
//!
//! Steady State is a high-performance, easy-to-use, actor-based framework for building concurrent applications in Rust.
//! It provides a robust set of tools and utilities to guarantee your Service Level Agreements (SLAs) through comprehensive
//! telemetry, alerts, and integration with Prometheus. Designed for low-latency, high-volume solutions, Steady State
//! empowers developers to create scalable and resilient systems with ease.
//!
//! ## Key Features
//!
//! - **Actor Model**: Simplifies concurrent programming by encapsulating state and behavior within actors.
//! - **Telemetry and Monitoring**: Built-in support for metrics collection, alerts, and Prometheus integration.
//! - **Low Latency**: Optimized for high-performance applications with minimal overhead.
//! - **High Volume**: Capable of handling large-scale data processing and communication.
//!
//! ## Getting Started
//!
//! Add Steady State to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! steady_state = "0.1.0"  # Replace with the actual version
//! ```
//!
//! Explore the [documentation](https://docs.rs/steady_state) and examples in the repository for more details.
//!
//! TODO: look for cargo all testing coverage.

#![warn(missing_docs)]

/// Ensures mutual exclusivity of executor features to prevent incompatible configurations.
///
/// The following feature gate checks enforce that only one executor feature is enabled at a time.
#[cfg(all(feature = "proactor_nuclei", feature = "exec_async_std"))]
compile_error!("Cannot enable both 'proactor_nuclei' and 'exec_async_std' features at the same time");

#[cfg(all(feature = "proactor_nuclei", feature = "proactor_tokio"))]
compile_error!("Cannot enable both 'proactor_nuclei' and 'proactor_tokio' features at the same time");

#[cfg(all(feature = "exec_async_std", feature = "proactor_tokio"))]
compile_error!("Cannot enable both 'exec_async_std' and 'proactor_tokio' features at the same time");

/// Requires at least one executor feature to be enabled for the framework to function.
///
/// This check ensures that an executor is selected for running actors and futures.
#[cfg(not(any(feature = "proactor_nuclei", feature = "proactor_tokio", feature = "exec_async_std")))]
compile_error!("Must enable one executor feature: 'proactor_nuclei', 'proactor_tokio', or 'exec_async_std'");

/// Internal module for telemetry-related functionality.
///
/// This module contains submodules for collecting, consuming, and setting up telemetry in the Steady State framework.
pub(crate) mod telemetry {
    /// Collects runtime metrics for monitoring system performance.
    pub(crate) mod metrics_collector;

    /// Consumes collected metrics for Prometheus export or local telemetry server, and manages history files.
    pub(crate) mod metrics_server;

    /// Provides logic for integrating telemetry actors into an application graph.
    pub(crate) mod setup;
}

/// Internal module for serialization utilities.
///
/// This module provides tools for efficient data serialization, particularly for use in distributed systems.
pub(crate) mod serialize {
    /// Handles efficient packing of data into byte buffers.
    pub(crate) mod byte_buffer_packer;

    /// Implements packed integer/long serialization based on the FAST/FIX protocol.
    pub(crate) mod fast_protocol_packed;
}

/// Internal module for collecting channel statistics.
pub(crate) mod channel_stats;

/// Internal module for collecting actor statistics.
pub(crate) mod actor_stats;

/// Internal module for framework configuration settings.
pub(crate) mod steady_config;

/// Internal module for graph visualization and DOT language integration.
pub(crate) mod dot;

// TODO: check our errors returned and make them simp

/// Manages the lifecycle states of actor graphs.
///
/// This module provides utilities for ensuring the liveliness and proper shutdown of actor graphs.
mod graph_liveliness;

/// Utilities for managing loops and futures in actor execution.
///
/// This module offers functions for selecting and awaiting multiple futures in a controlled manner.
mod loop_driver;

/// Executor abstraction for Nuclei-based runtimes.
///
/// Available when either the `proactor_nuclei` or `proactor_tokio` feature is enabled.
#[cfg(any(feature = "proactor_nuclei", feature = "proactor_tokio"))]
mod abstract_executor_nuclei;

#[cfg(any(feature = "proactor_nuclei", feature = "proactor_tokio"))]
pub use abstract_executor_nuclei::*;

/// Executor abstraction for the `async-std` runtime.
///
/// Available when the `exec_async_std` feature is enabled.
#[cfg(feature = "exec_async_std")]
mod abstract_executor_async_std;

#[cfg(feature = "exec_async_std")]
use abstract_executor_async_std::*;

/// Utilities for capturing panics during testing.
///
/// This module is only available in test configurations.
#[cfg(test)]
mod test_panic_capture;

/// Integrates monitoring with telemetry systems.
///
/// This module provides the glue between runtime monitoring and telemetry output.
mod monitor_telemetry;

/// Monitoring utilities for inspecting channel and actor metrics at runtime.
///
/// The `monitor` module provides types and traits for gathering and representing runtime metadata about channels and actors,
/// enabling integration with telemetry systems and health checks.
pub mod monitor;

/// Channel construction and configuration utilities.
///
/// This module provides a builder-pattern API and macros for creating and configuring channels.
/// It is marked with `#[macro_use]` to allow macros defined within it to be used throughout the crate.
#[macro_use]
pub mod channel_builder;

/// Actor construction, configuration, and scheduling utilities.
///
/// The `actor_builder` module offers a builder-pattern API for defining actors, setting up their execution contexts,
/// core affinity, and telemetry.
pub mod actor_builder;
pub use actor_builder::CoreBalancer;

///
/// Manage state for actors scros failures and restarts
pub mod state_management;
pub use state_management::SteadyState;
pub use state_management::new_state;/// Installation utilities for various deployment methods.
pub use state_management::new_persistent_state;
pub use state_management::StateGuard;

pub use channel_builder_lazy::*;



///
/// This module contains submodules to support different installation strategies.
pub mod install {
    /// Supports creating and removing systemd service configurations.
    pub mod serviced;

    /// Supports creating local command-line applications.
    pub mod local_cli;
}

/// Components and builders for distributed systems.
///
/// This module provides tools for building distributed systems, including Aeron streams and pub/sub mechanisms.
pub mod distributed {
    /// Enums for constructing Aeron connection strings.
    pub mod aeron_channel_structs;

    /// Builder for creating serialized data channels with Aeron.
    pub mod aeron_channel_builder;

    /// Manages stream-based channels in distributed systems.
    pub mod aqueduct_stream;

    /// Publishes messages from streams to Aeron.
    pub mod aeron_publish_bundle;

    /// Subscribes to Aeron and forwards messages to streams.
    pub mod aeron_subscribe_bundle;

    /// Single channel publish
    pub mod aeron_publish;

    /// Single channel subscribe
    pub mod aeron_subscribe;

    /// Aqueduct builder
    pub mod aqueduct_builder;

    /// Utility for polling for messages on a stream
    pub mod polling;
}

/// Tools for simulating edge cases in testing.
///
/// This module provides utilities for testing the robustness of actors under various conditions.
pub mod simulate_edge;

/// Utilities for testing full graphs of actors.
///
/// This module offers tools to validate the behavior of complex actor networks.
pub mod graph_testing;

/// Transmitter channel features and utilities.
///
/// This module provides the core functionality for sending messages through channels.
pub mod steady_tx;

/// Receiver channel features and utilities.
///
/// This module provides the core functionality for receiving messages from channels.
pub mod steady_rx;

/// Utilities for yielding execution within actors.
///
/// This module allows actors to yield control back to the runtime, improving fairness and responsiveness.
pub mod yield_now;

/// Commands and utilities for channels used by actors.
///
/// This module defines the core actor logic and channel interactions.
pub mod steady_actor;

/// Low-level receiver functionality.
///
/// This module contains internal implementations for receiving messages.
mod core_rx;
pub use crate::core_rx::RxCore;
pub use crate::core_rx::DoubleSlice;
pub use crate::core_rx::DoubleSliceCopy;
pub use crate::core_rx::QuadSlice;
pub use crate::core_rx::StreamQuadSliceCopy;

/// Low-level transmitter functionality.
///
/// This module contains internal implementations for sending messages.
mod core_tx;
pub use crate::core_tx::TxCore;

pub use crate::distributed::aqueduct_stream::StreamControlItem;

/// Shadow utilities for steady actors.
///
/// This module provides additional functionality for managing actor shadows.
pub mod steady_actor_shadow;

/// Spotlight utilities for steady actors.
///
/// This module provides tools for highlighting or managing actor execution.
pub mod steady_actor_spotlight;


/// Tests for executor abstractions.
///
/// This module contains tests for ensuring executor compatibility.
mod abstract_executor_tests;

/// Utilities for managing concurrent execution of futures.
///
/// These exports from `loop_driver` provide functions for selecting and awaiting multiple futures.
pub use loop_driver::steady_fuse_future;
pub use loop_driver::steady_select_two;
pub use loop_driver::steady_select_three;
pub use loop_driver::steady_select_four;
pub use loop_driver::steady_select_five;
pub use loop_driver::steady_await_for_all_or_proceed_upon_two;
pub use loop_driver::steady_await_for_all_or_proceed_upon_three;
pub use loop_driver::steady_await_for_all_or_proceed_upon_four;
pub use loop_driver::steady_await_for_all_or_proceed_upon_five;

// Public re-exports for convenience
pub use clap::*;
pub use steady_actor::SendOutcome;
pub use simulate_edge::SimRunner;
pub use steady_actor_shadow::*;
pub use futures_timer::Delay; // for easy use
pub use graph_testing::GraphTestResult;
pub use monitor::{RxMetaDataHolder, TxMetaDataHolder};
pub use channel_builder_units::Rate;
pub use channel_builder_units::Filled;
pub use actor_builder_units::MCPU;
pub use actor_builder_units::Work;
pub use actor_builder_units::Percentile;
pub use actor_builder::Troupe;
pub use actor_builder::ScheduleAs;
pub use actor_builder::ScheduleAs::*;
pub use graph_liveliness::*;
pub use install::serviced::*;
pub use steady_rx::Rx;
pub use steady_tx::Tx;
pub use steady_rx::SteadyRxBundleTrait;
pub use steady_tx::SteadyTxBundleTrait;
pub use steady_rx::RxBundleTrait;
pub use steady_tx::TxBundleTrait;
pub use steady_rx::RxDone;
pub use steady_tx::TxDone;
pub use crate::distributed::aqueduct_builder::AqueductBuilder;
pub use steady_actor::SteadyActor;
pub use distributed::aeron_channel_structs::{Channel, Endpoint, MediaType};
pub use distributed::aeron_channel_builder::{AeronConfig, AqueTech};
pub use distributed::aqueduct_stream::{StreamEgress, StreamIngress};
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundle, LazySteadyStreamTxBundle};
pub use distributed::aqueduct_stream::{SteadyStreamRxBundle, SteadyStreamTxBundle};
pub use distributed::aqueduct_stream::{LazyStreamRx, LazyStreamTx};
pub use distributed::aqueduct_stream::{SteadyStreamRxBundleTrait, StreamRxBundleTrait};
pub use distributed::aqueduct_stream::{SteadyStreamTxBundleTrait, StreamTxBundleTrait};
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone};
pub use distributed::aqueduct_stream::{SteadyStreamRx, SteadyStreamTx, StreamRx, StreamTx};
pub use log::{debug, error, info, trace, warn};
pub use std::time::{Duration, Instant};
pub use std::error::Error;

// Dependencies and internal utilities
use futures_util::FutureExt;
use futures::select;
use std::fmt::Debug;
use std::sync::Arc;
use futures::lock::Mutex;
use std::ops::DerefMut;
#[allow(unused_imports)]
use log::*;
use crate::monitor::{ActorMetaData, ChannelMetaData};

/// Miscellaneous utility functions.
///
/// This module contains various helper functions used throughout the framework.
pub mod logging_util;

/// Utilities for inspecting short boolean sequences.
///
/// This module provides tools for analyzing short sequences of boolean values.
pub mod expression_steady_eye;

/// Telemetry details and unit structs for channels
pub mod channel_builder_units;


mod core_tx_guard;
mod core_rx_guard;
mod core_rx_stream;
mod core_tx_stream;
mod channel_stats_tests;
mod channel_stats_labels;
mod actor_stats_tests;
mod actor_builder_units;
mod channel_builder_lazy;
mod dot_edge;
mod dot_node;

pub use crate::expression_steady_eye::LAST_FALSE;

pub use crate::logging_util::*;
use futures::AsyncRead;
use futures::AsyncWrite;
pub use futures::future::Future;
use futures::channel::oneshot;
use futures_util::lock::MutexGuard;
pub use steady_actor_spotlight::SteadyActorSpotlight;

use crate::yield_now::yield_now;



/// Type alias for a thread-safe transmitter wrapped in an `Arc` and `Mutex`.
///
/// Simplifies the usage of a transmitter that can be shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
pub type SteadyTx<T> = Arc<Mutex<Tx<T>>>;

/// Type alias for an array of thread-safe transmitters with a fixed size, wrapped in an `Arc`.
///
/// Simplifies the usage of a bundle of transmitters shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
/// - `GIRTH`: The fixed size of the transmitter array.
pub type SteadyTxBundle<T, const GIRTH: usize> = Arc<[SteadyTx<T>; GIRTH]>;

/// Type alias for a thread-safe receiver wrapped in an `Arc` and `Mutex`.
///
/// Simplifies the usage of a receiver that can be shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being received.
pub type SteadyRx<T> = Arc<Mutex<Rx<T>>>;

/// Type alias for an array of thread-safe receivers with a fixed size, wrapped in an `Arc`.
///
/// Simplifies the usage of a bundle of receivers shared across threads.
///
/// # Type Parameters
/// - `T`: The type of data being received.
/// - `GIRTH`: The fixed size of the receiver array.
pub type SteadyRxBundle<T, const GIRTH: usize> = Arc<[SteadyRx<T>; GIRTH]>;

/// Type alias for a vector of `MutexGuard` references to transmitters.
///
/// Simplifies batch operations over multiple transmitter guards.
///
/// # Type Parameters
/// - `T`: The type of data being transmitted.
pub type TxBundle<'a, T> = Vec<MutexGuard<'a, Tx<T>>>;

/// Type alias for a vector of `MutexGuard` references to receivers.
///
/// Simplifies batch operations over multiple receiver guards.
///
/// # Type Parameters
/// - `T`: The type of data being received.
pub type RxBundle<'a, T> = Vec<MutexGuard<'a, Rx<T>>>;

/// Bundle of `TxCore` guards for batch locking of transmitters.
///
/// Represents a collection of locked transmitter guards for batch operations.
///
/// # Type Parameters
/// - `T`: The type implementing `TxCore`.
#[allow(type_alias_bounds)]
pub type TxCoreBundle<'a, T: TxCore> = Vec<MutexGuard<'a, T>>;

/// Bundle of `RxCore` guards for batch locking of receivers.
///
/// Represents a collection of locked receiver guards for batch operations.
///
/// # Type Parameters
/// - `T`: The type implementing `RxCore`.
#[allow(type_alias_bounds)]
pub type RxCoreBundle<'a, T: RxCore> = Vec<MutexGuard<'a, T>>;

/// Creates a bundle of thread-safe transmitters with a fixed size, wrapped in an `Arc`.
///
/// Wraps an array of transmitters in an `Arc` for shared ownership across threads.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyTx<T>` with a fixed size `GIRTH`.
///
/// # Returns
/// - `SteadyTxBundle<T, GIRTH>`: A bundle of transmitters wrapped in an `Arc`.
pub fn steady_tx_bundle<T, const GIRTH: usize>(internal_array: [SteadyTx<T>; GIRTH]) -> SteadyTxBundle<T, GIRTH> {
    Arc::new(internal_array)
}

/// Creates a bundle of thread-safe receivers with a fixed size, wrapped in an `Arc`.
///
/// Wraps an array of receivers in an `Arc` for shared ownership across threads.
///
/// # Parameters
/// - `internal_array`: An array of `SteadyRx<T>` with a fixed size `GIRTH`.
///
/// # Returns
/// - `SteadyRxBundle<T, GIRTH>`: A bundle of receivers wrapped in an `Arc`.
pub fn steady_rx_bundle<T, const GIRTH: usize>(internal_array: [SteadyRx<T>; GIRTH]) -> SteadyRxBundle<T, GIRTH> {
    Arc::new(internal_array)
}

/// Initializes logging for the Steady State crate.
///
/// This convenience function should be called at the beginning of `main` to set up logging.
///
/// # Parameters
/// - `loglevel`: The desired logging level (e.g., `Info`, `Debug`).
///
/// # Returns
/// - `Result<(), Box<dyn std::error::Error>>`: Ok if successful, or an error if initialization fails.
pub fn init_logging(loglevel: LogLevel) -> Result<(), Box<dyn std::error::Error>> {
    steady_logger::initialize_with_level(loglevel)
}

/// Logging levels for controlling verbosity of the crate's logging output.
///
/// Maps to standard `log::LevelFilter` values for configuring application logging.
#[derive(Copy, Clone, Debug, PartialEq, ValueEnum)]
pub enum LogLevel {
    /// Disables all logging output.
    Off,
    /// Logs only errors.
    Error,
    /// Logs warnings and errors.
    Warn,
    /// Logs informational messages, warnings, and errors.
    Info,
    /// Logs debug messages, informational messages, warnings, and errors.
    Debug,
    /// Logs all messages, including trace-level details.
    Trace,
}

impl LogLevel {
    /// Converts this `LogLevel` to the corresponding `log::LevelFilter`.
    ///
    /// # Returns
    /// - `log::LevelFilter`: The matching filter level for logging.
    pub fn to_level_filter(&self) -> log::LevelFilter {
        match self {
            LogLevel::Off => log::LevelFilter::Off,
            LogLevel::Error => log::LevelFilter::Error,
            LogLevel::Warn => log::LevelFilter::Warn,
            LogLevel::Info => log::LevelFilter::Info,
            LogLevel::Debug => log::LevelFilter::Debug,
            LogLevel::Trace => log::LevelFilter::Trace,
        }
    }
}

/// Macro for creating a LocalMonitor from channels.
///
/// Takes a `SteadyContext` and lists of Rx and Tx channels, returning a `LocalMonitor` for telemetry and Prometheus metrics.
#[macro_export]
macro_rules! into_monitor {
    ($self:expr, [$($rx:expr),*], [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        use steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use steady_tx::TxMetaDataProvider;
        let rx_meta = [$($rx.meta_data(),)*];
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, tx_meta)
    }};
    ($self:expr, [$($rx:expr),*], $tx_bundle:expr) => {{
        #[allow(unused_imports)]
        use steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use steady_tx::TxMetaDataProvider;
        let rx_meta = [$($rx.meta_data(),)*];
        $self.into_monitor_internal(rx_meta, $tx_bundle.meta_data())
    }};
    ($self:expr, $rx_bundle:expr, [$($tx:expr),*]) => {{
        #[allow(unused_imports)]
        use steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use steady_tx::TxMetaDataProvider;
        let tx_meta = [$($tx.meta_data(),)*];
        $self.into_monitor_internal($rx_bundle.meta_data(), tx_meta)
    }};
    ($self:expr, $rx_bundle:expr, $tx_bundle:expr) => {{
        $self.into_monitor_internal($rx_bundle.meta_data(), $tx_bundle.meta_data())
    }};
    ($self:expr, ($rx_channels_to_monitor:expr, [$($rx:expr),*], $($rx_bundle:expr),* ), ($tx_channels_to_monitor:expr, [$($tx:expr),*], $($tx_bundle:expr),* )) => {{
        #[allow(unused_imports)]
        use steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use steady_tx::TxMetaDataProvider;
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
        use steady_rx::RxMetaDataProvider;
        #[allow(unused_imports)]
        use steady_tx::TxMetaDataProvider;
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

/// Constant representing an unknown monitor state.
///
/// Used in monitoring logic to indicate an undefined or uninitialized state.
const MONITOR_UNKNOWN: usize = usize::MAX;

/// Constant representing a "not monitored" state.
///
/// Used in monitoring logic to differentiate from `MONITOR_UNKNOWN`.
const MONITOR_NOT: usize = MONITOR_UNKNOWN - 1;

/// Represents the behavior of the system when a channel is saturated (i.e., full).
///
/// Defines how the system responds when attempting to send to a full channel, managing backpressure.
#[derive(Default, PartialEq, Eq, Debug)]
pub enum SendSaturation {
    /// Blocks the sender until space is available in the channel.
    AwaitForRoom,

    /// Returns an error immediately if the channel is full.
    ReturnBlockedMsg,

    /// Logs a warning and waits for space (default behavior).
    #[default]
    WarnThenAwait,

    /// Logs a debug warning and waits, optimized for release builds.
    DebugWarnThenAwait,
}

/// Represents a standard deviation value for metrics and alerts.
///
/// Encapsulates a standard deviation within a valid range (0.0, 10.0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StdDev(f32);

impl StdDev {
    /// Creates a new `StdDev` if the value is within (0.0, 10.0).
    fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Creates a `StdDev` of 1.0.
    pub fn one() -> Self {
        Self(1.0)
    }

    /// Creates a `StdDev` of 1.5.
    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    /// Creates a `StdDev` of 2.0.
    pub fn two() -> Self {
        Self(2.0)
    }

    /// Creates a `StdDev` of 2.5.
    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    /// Creates a `StdDev` of 3.0.
    pub fn three() -> Self {
        Self(3.0)
    }

    /// Creates a `StdDev` of 4.0.
    pub fn four() -> Self {
        Self(4.0)
    }

    /// Creates a custom `StdDev` if within (0.0, 10.0).
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    /// Retrieves the standard deviation value.
    pub fn value(&self) -> f32 {
        self.0
    }
}

/// Base trait for all metrics used in telemetry and Prometheus.
pub trait Metric: PartialEq {}

/// Trait for metrics suitable for data channels.
pub trait DataMetric: Metric {}

/// Trait for metrics suitable for computational actors.
pub trait ComputeMetric: Metric {}

impl Metric for Duration {}

/// Represents the color of an alert.
///
/// Indicates the severity of an alert in the Steady State framework.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertColor {
    /// Warning level alert (non-critical).
    Yellow,

    /// Elevated alert level (serious).
    Orange,

    /// Critical alert level (immediate action required).
    Red,
}

/// Represents a trigger condition for a metric.
///
/// Defines conditions that trigger alerts based on metric values.
///
/// # Type Parameters
/// - `T`: The metric type implementing `Metric`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger<T>
where
    T: Metric,
{
    /// Triggers when the average exceeds the threshold.
    AvgAbove(T),

    /// Triggers when the average falls below the threshold.
    AvgBelow(T),

    /// Triggers when above mean plus standard deviations.
    StdDevsAbove(StdDev, T),

    /// Triggers when below mean minus standard deviations.
    StdDevsBelow(StdDev, T),

    /// Triggers when above a percentile threshold.
    PercentileAbove(Percentile, T),

    /// Triggers when below a percentile threshold.
    PercentileBelow(Percentile, T),
}
#[cfg(test)]
mod lib_tests {
    use super::*;
    use std::sync::{Arc, OnceLock};
    use futures::channel::oneshot;
    use std::time::Instant;
    use std::sync::atomic::AtomicUsize;
    use crate::channel_builder::ChannelBuilder;
    use parking_lot::RwLock;
    use futures::lock::Mutex;
    use steady_actor::SteadyActor;
    use crate::core_rx::DoubleSlice;
    use crate::steady_actor_shadow::SteadyActorShadow;
    use crate::core_tx::TxCore;

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
    // fn test_request_shutdown() {
    //     let context = test_steady_context();
    //     let result = context.request_shutdown().await;
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
                .send_async(&mut tx_guard, 42, SendSaturation::WarnThenAwait)
                .await;
            assert!(result.is_sent());
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
        let tx = tx.clone();
        let rx = rx.clone();
        // Use the macro
        let _monitor = context.into_spotlight([&rx], [&tx]);
        // Since we cannot directly test the internal state of the monitor,
        // we ensure that the macro compiles and runs without errors
        assert!(true);
    }

    // Helper method to build tx and rx arguments
    fn build_tx_rx() -> (oneshot::Sender<()>, oneshot::Receiver<()>) {
        oneshot::channel()
    }

    // Common function to create a test SteadyContext
    fn test_steady_context() -> SteadyActorShadow {
        let (_tx, rx) = build_tx_rx();
        SteadyActorShadow {
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
            regeneration: 0,
            last_periodic_wait: Default::default(),
            is_in_graph: true,
            actor_start_time: Instant::now(),
            frame_rate_ms: 1000,
            show_thread_info: false,
            team_id: 0,
            aeron_meda_driver: OnceLock::new(),
            use_internal_behavior: true,
            shutdown_barrier: None,

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
            40);

        builder.build_channel::<T>()
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
            assert_eq!(count.item_count(), 3);
            assert_eq!(slice, [1, 2, 3]);
        };
    }

    // Test for try_peek_slice
    #[test]
    fn test_try_peek_slice() {
        let rx = create_rx(vec![1, 2, 3, 4, 5]);
        let context = test_steady_context();
        if let Some(mut rx) = rx.try_lock() {
            let slice = context.peek_slice(&mut rx);
            assert_eq!(slice.total_len(), 5);
            assert_eq!(slice.to_vec(), [1, 2, 3, 4, 5]);
            let mut buf = [0;3];
            slice.copy_into_slice(&mut buf);
            assert_eq!(buf, [1, 2, 3]);
        };
    }


    // Test wait_avail_units_bundle method
    #[async_std::test]
    async fn test_wait_avail_units_bundle() {
        let context = test_steady_context();
        let mut rx_bundle = RxBundle::<i32>::new();
        let fut = context.wait_avail_bundle(&mut rx_bundle, 1, 1);
        assert!(fut.await);
    }

    // Test wait_avail_units_bundle method
    #[async_std::test]
    async fn test_wait_closed_or_avail_units_bundle() {
        let context = test_steady_context();
        let mut rx_bundle = RxBundle::<i32>::new();
        let fut = context.wait_avail_bundle(&mut rx_bundle, 1, 1);
        assert!(fut.await);
    }

    // Test wait_vacant_units_bundle method
    #[async_std::test]
    async fn test_wait_vacant_units_bundle() {
        let context = test_steady_context();
        let mut tx_bundle = TxBundle::<i32>::new();
        let fut = context.wait_vacant_bundle(&mut tx_bundle, 1, 1);
        assert!(fut.await);

    }

    #[async_std::test]
    async fn test_wait_shutdown_or_avail_units() {
        let context = test_steady_context();
        let rx = create_rx::<i32>(vec![1, 2, 3]);
        let guard = rx.try_lock();
        if let Some(mut rx) = guard {
            let result = context.wait_avail(&mut rx, 2).await;
            assert!(result); // Ensure it waits for units or shutdown
        }
    }

    #[async_std::test]
    async fn test_wait_closed_or_avail_units() {
        let context = test_steady_context();
        let rx = create_rx::<i32>(vec![1, 2, 3]);
        let guard = rx.try_lock();
        if let Some(mut rx) = guard {
            let result = context.wait_avail(&mut rx, 2).await;
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
            let result = context.wait_vacant(&mut tx, 2).await;
            assert!(result); // Should succeed if vacant units or shutdown occur
        }
    }

    #[async_std::test]
    async fn test_wait_vacant_units() {
        let context = test_steady_context();
        let tx = create_tx::<i32>(vec![]);
        let guard = tx.try_lock();
        if let Some(mut tx) = guard {
            let result = context.wait_vacant(&mut tx, 1).await;
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
    fn test_send_slice() {
        let (tx, _rx) = create_test_channel();
        let mut context = test_steady_context();
        let slice = [1, 2, 3];
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let done = context.send_slice(&mut tx, &slice);
            assert_eq!(done.item_count(), slice.len());
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
        let (tx, _rx) = create_test_channel::<usize>();
        let mut context = test_steady_context();
        let tx = tx.clone();
        if let Some(mut tx) = tx.try_lock() {
            let result = context.try_send(&mut tx, 42).is_sent();
            assert!(result);
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


}

#[cfg(test)]
mod enum_tests {
    use channel_builder_lazy::{LazySteadyRxBundleClone, LazySteadyTxBundleClone};
    use crate::channel_builder::ChannelBuilder;
    use crate::channel_builder_lazy::{LazyChannel, LazySteadyRxBundle, LazySteadyTxBundle};
    use super::*;

    #[test]
    fn test_send_saturation_default() {
        let saturation = SendSaturation::default();
        assert_eq!(saturation, SendSaturation::WarnThenAwait);
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
        assert_eq!(None, invalid_std_dev);
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
            _ => unreachable!("Expected AvgAbove"),
        }

        match avg_below {
            Trigger::AvgBelow(val) => assert_eq!(val, Duration::from_secs(2)),
            _ => unreachable!("Expected AvgBelow"),
        }

        match std_devs_above {
            Trigger::StdDevsAbove(std_dev, val) => {
                assert_eq!(std_dev, StdDev::two());
                assert_eq!(val, Duration::from_secs(3));
            },
            _ => unreachable!("Expected StdDevsAbove"),
        }

        match std_devs_below {
            Trigger::StdDevsBelow(std_dev, val) => {
                assert_eq!(std_dev, StdDev::one());
                assert_eq!(val, Duration::from_secs(4));
            },
            _ => unreachable!("Expected StdDevsBelow"),
        }

        match percentile_above {
            Trigger::PercentileAbove(percentile, val) => {
                assert_eq!(percentile, Percentile(90.0));
                assert_eq!(val, Duration::from_secs(5));
            },
            _ => unreachable!("Expected PercentileAbove"),
        }

        match percentile_below {
            Trigger::PercentileBelow(percentile, val) => {
                assert_eq!(percentile, Percentile(10.0));
                assert_eq!(val, Duration::from_secs(6));
            },
            _ => unreachable!("Expected PercentileBelow"),
        }
    }

    #[test]
    fn test_lazy_steady_tx_bundle_clone() {
        // Define a simple data type for the test
        type TestType = i32;

        let cb = ChannelBuilder::new(
                    Arc::new(Default::default()),
                    Arc::new(Default::default()),
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
