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

#[cfg(all(windows, any(feature = "proactor_nuclei", feature = "proactor_tokio")))]
compile_error!("The 'proactor_nuclei' and 'proactor_tokio' features are not supported on Windows due to upstream issues in the nuclei crate. Please use 'exec_async_std' instead.");

/// Requires at least one executor feature to be enabled for the framework to function.
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
pub(crate) use abstract_executor_nuclei::core_exec;

/// Executor abstraction for the `async-std` runtime.
///
/// Available when the `exec_async_std` feature is enabled.
#[cfg(all(feature = "exec_async_std", not(any(feature = "proactor_nuclei", feature = "proactor_tokio"))))]
mod abstract_executor_async_std;

use std::any::Any;
#[cfg(all(feature = "exec_async_std", not(any(feature = "proactor_nuclei", feature = "proactor_tokio"))))]
pub(crate) use abstract_executor_async_std::core_exec;

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

/// Alias for the actor shadow context used throughout the framework.
pub type SteadyContext = SteadyActorShadow;

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
use std::io;
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

/// meta macros for building our the spotlight
pub mod macros;

pub use crate::expression_steady_eye::LAST_FALSE;

pub use crate::logging_util::*;
use futures::AsyncRead;
use futures::AsyncWrite;
pub use futures::future::Future;
use futures::channel::oneshot;
use futures_util::lock::MutexGuard;
pub use steady_actor_spotlight::SteadyActorSpotlight;
pub use crate::steady_tx::TxMetaDataProvider;
pub use crate::steady_rx::RxMetaDataProvider;
pub use crate::macros::steady_rx_bundle;
pub use crate::macros::steady_tx_bundle;
pub use crate::macros::steady_rx_bundle_active;
pub use crate::macros::steady_tx_bundle_active;

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



/// Initializes logging for the Steady State crate.
///
/// This convenience function should be called at the beginning of `main` to set up logging.
///
/// # Parameters
/// - `loglevel`: The desired logging level (e.g., `Info`, `Debug`).
/// - `file_config`: Optional configuration for file-based logging with rotation.
///
/// # Returns
/// - `Result<(), Box<dyn std::error::Error>>`: Ok if successful, or an error if initialization fails.
pub fn init_logging(loglevel: LogLevel, file_config: Option<LogFileConfig>) -> Result<(), Box<dyn std::error::Error>> {
    steady_logger::initialize_with_level_and_file(loglevel, file_config)
}

/// Configuration for file-based logging with rotation.
#[derive(Clone, Debug)]
pub struct LogFileConfig {
    /// Directory where log files will be stored.
    pub directory: String,
    /// Base name for the log files.
    pub base_name: String,
    /// Maximum size of a log file in bytes before rotation.
    pub max_size_bytes: u64,
    /// Number of historical log files to keep.
    pub keep_count: usize,
    /// Whether to delete old logs on startup.
    pub delete_old_on_start: bool,
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

/// Constant representing an unknown monitor state.
///
/// Used in monitoring logic to indicate an undefined or uninitialized state.
pub const MONITOR_UNKNOWN: usize = usize::MAX;

/// Constant representing a "not monitored" state.
///
/// Used in monitoring logic to differentiate from `MONITOR_UNKNOWN`.
pub const MONITOR_NOT: usize = MONITOR_UNKNOWN - 1;

/// Represents the behavior of the system when a channel is saturated (i.e., full).
///
/// Defines how the system responds when attempting to send to a full channel, managing backpressure.
#[derive(Default, PartialEq, Eq, Debug, Copy, Clone)]
pub enum SendSaturation {
    /// Blocks the sender until space is available in the channel.
    AwaitForRoom,

    /// Returns an error immediately if the channel is full.
    #[deprecated(note = "Use try_send instead")]
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
    pub fn new(value: f32) -> Option<Self> {
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

/// Builder for the orchestration environment, ensuring sufficient stack size.
#[derive(Clone, Debug)]
pub struct SteadyRunner {
    stack_size: usize,
    name: String,
    loglevel: Option<LogLevel>,
    log_file_config: Option<LogFileConfig>,
    default_actor_stack_size: Option<usize>,
    barrier_size: Option<usize>,
    telemetry_rate_ms: Option<u64>,
    telemetry_colors: Option<(String, String)>,
    for_test: bool,
    bundle_floor_size: Option<usize>,
}

impl SteadyRunner {
    /// Creates a new SteadyRunner with default settings.
    pub fn test_build() -> Self {
        Self {
            stack_size: 16 * 1024 * 1024, // 16 MiB default for main
            name: "steady-orchestrator".to_string(),
            loglevel: None,
            log_file_config: None,
            default_actor_stack_size: Some(2 * 1024 * 1024), // 2 MiB default for each actor
            barrier_size: None,
            telemetry_rate_ms: None,
            telemetry_colors: None,
            for_test: true,
            bundle_floor_size: None,
        }
    }
    /// Creates a new SteadyRunner with default settings.
    pub fn release_build() -> Self {
        Self {
            stack_size: 16 * 1024 * 1024, // 16 MiB default for main
            name: "steady-orchestrator".to_string(),
            loglevel: None,
            log_file_config: None,
            default_actor_stack_size: Some(2 * 1024 * 1024), // 2 MiB default for each actor
            barrier_size: None,
            telemetry_rate_ms: None,
            telemetry_colors: None,
            for_test: false,
            bundle_floor_size: None,
        }
    }


    /// Sets the stack size for the orchestration thread.
    pub fn with_stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = bytes;
        self
    }

    /// Sets the logging level for the application.
    pub fn with_logging(mut self, level: LogLevel) -> Self {
        self.loglevel = Some(level);
        self
    }

    /// Sets the file logging configuration for the application.
    pub fn with_file_logging(mut self, directory: &str, base_name: &str, max_size_bytes: u64, keep_count: usize, delete_old_on_start: bool) -> Self {
        self.log_file_config = Some(LogFileConfig {
            directory: directory.to_string(),
            base_name: base_name.to_string(),
            max_size_bytes,
            keep_count,
            delete_old_on_start,
        });
        self
    }

    /// Sets the default actor stack size for the graph.
    pub fn with_default_actor_stack_size(mut self, size: usize) -> Self {
        self.default_actor_stack_size = Some(size);
        self
    }

    /// Sets the size of the shutdown barrier for the graph.
    pub fn with_shutdown_barrier(mut self, size: usize) -> Self {
        self.barrier_size = Some(size);
        self
    }

    /// Sets the telemetry rate for the graph.
    pub fn with_telemetry_rate_ms(mut self, ms: u64) -> Self {
        self.telemetry_rate_ms = Some(ms);
        self
    }

    /// Sets the telemetry top bar colors (primary and secondary hex strings).
    pub fn with_telemetry_colors(mut self, primary_color: &str, secondary_color: &str) -> Self {
        self.telemetry_colors = Some((primary_color.to_string(), secondary_color.to_string()));
        self
    }

    /// Sets the bundle floor size for the graph.
    pub fn with_bundle_floor_size(mut self, size: usize) -> Self {
        self.bundle_floor_size = Some(size);
        self
    }

    /// Spawns a guarded thread, initializes a production graph, and executes the provided closure.
    /// The result (including errors) from the closure is propagated back to the caller as a boxed,
    /// thread-safe error. Panics in the thread are unwound (propagated) to the calling thread.
    pub fn run<A, F>(
        self,
        args: A,
        f: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        A: Any + Send + Sync + 'static,
        F: FnOnce(Graph) -> Result<(), Box<dyn std::error::Error>> + std::marker::Send + 'static,
    {

        let builder = std::thread::Builder::new()
                .name(self.name)
                .stack_size(self.stack_size);

        // Spawn the thread and capture its join handle
        let handle = builder.spawn(move || {
            // Initialize logging if specified; ignore errors to avoid masking closure failures
            if let Some(level) = self.loglevel {
                let _ = init_logging(level, self.log_file_config);
            }

            let mut graph = if self.for_test {
                               GraphBuilder::for_testing()
                             } else {
                               GraphBuilder::for_production()
                            };


            if let Some(size) = self.default_actor_stack_size {
                graph = graph.with_default_actor_stack_size(size);
            }
            if let Some(size) = self.barrier_size {
                graph = graph.with_shutdown_barrier(size);
            }
            if let Some(rate) = self.telemetry_rate_ms {
                graph = graph.with_telemtry_production_rate_ms(rate);
            }
            if let Some((ref c1, ref c2)) = self.telemetry_colors {
                graph = graph.with_telemetry_colors(c1, c2);
            }
            if let Some(size) = self.bundle_floor_size {
                graph = graph.with_bundle_floor_size(size);
            }

            let graph = graph.build(args);

            // Execute the user closure and return its result directly
            // (This propagates the Result from f, allowing errors to cross the thread boundary safely)
            match f(graph) {
                Ok(()) => Ok(()),
                Err(e) => {
                    let err_msg = e.to_string();
                    Err(Box::new(io::Error::new(io::ErrorKind::Other, err_msg)) as Box<dyn std::error::Error + Send + Sync + 'static>)
                }
            }
        })
            .expect("Failed to spawn production orchestrator thread");

        // Block and retrieve the thread's result: Inner is the closure's Result, outer is panic info
        // Unwrap the outer Result; if the thread panicked, resume unwinding to propagate it
        // If successful, return the inner Result (Ok(()) or Err(Box<dyn Error + Send + Sync + 'static>))
        match handle.join() {
            Ok(inner_result) => match inner_result {
                            Ok(()) => Ok(()),
                            Err(e) => {
                                let err_msg = e.to_string();
                                Err(Box::new(io::Error::new(io::ErrorKind::Other, err_msg)) as Box<dyn std::error::Error + Send + Sync + 'static>)
                            }
                        },
            Err(panic) => {
                // Thread panicked: Resume unwinding to propagate the panic to the caller
                std::panic::resume_unwind(panic);
            }
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
