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

/// Internal module for telemetry-related functionality.
///
/// This module contains submodules for collecting, consuming, and setting up telemetry in the Steady State framework.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod telemetry {
    /// Collects runtime metrics for monitoring system performance.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mod metrics_collector;

    /// Consumes collected metrics for Prometheus export or local telemetry server, and manages history files.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mod metrics_server;

    /// Provides logic for integrating telemetry actors into an application graph.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mod setup;
}

/// Internal module for serialization utilities.
///
/// This module provides tools for efficient data serialization, particularly for use in distributed systems.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod serialize {
    /// Handles efficient packing of data into byte buffers.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mod byte_buffer_packer;

    /// Implements packed integer/long serialization based on the FAST/FIX protocol.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mod fast_protocol_packed;
}

/// Internal module for collecting channel statistics.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod channel_stats;

/// Internal module for collecting actor statistics.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod actor_stats;

/// Internal module for framework configuration settings.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod steady_config;

/// Shared frame-based refresh/window bit sizing for actor and channel telemetry.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod telemetry_window;

/// Internal module for graph visualization and DOT language integration.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod dot;

/// Unified edge-slot merge for telemetry channel ids (`DotState.edges`).
///
/// Operators: conflicting endpoints are logged under target `steady_state::telemetry::dot`.
// ss[related philosophy.structural-hierarchy]
pub(crate) mod dot_unify;

/// Manages the lifecycle states of actor graphs.
///
/// This module provides utilities for ensuring the liveliness and proper shutdown of actor graphs.
// ss[related philosophy.structural-hierarchy]
mod graph;
// ss[related philosophy.structural-hierarchy]
mod graph_liveliness;

/// Utilities for managing loops and futures in actor execution.
///
/// This module offers functions for selecting and awaiting multiple futures in a controlled manner.
// ss[related philosophy.structural-hierarchy]
mod loop_driver;

/// Bare-metal executor (`futures::executor::block_on` on OS threads).
///
/// With the `tokio` feature, `block_on` uses a current-thread Tokio runtime on that same thread.
// ss[impl platform.ringbuf-pin]
// ss[verify platform.ringbuf-pin]
// ss[impl platform.executor-features]
// ss[related philosophy.structural-hierarchy]
mod abstract_executor;

/// Tracey impl anchors for CI process requirements (`verify.process.*`).
mod verify_process;

// ss[related philosophy.structural-hierarchy]
pub(crate) use abstract_executor::core_exec;

/// Utilities for capturing panics during testing.
///
/// This module is only available in test configurations.
#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
mod test_panic_capture;

/// Property-test case count (shared with `proptest_support::SS_PROPCASES`).
// ss[related philosophy.structural-hierarchy]
pub const SS_PROPCASES: u32 = 2048;

/// Shared proptest strategies and channel harness helpers.
#[cfg(test)]
#[doc(hidden)]
pub mod proptest_support;

/// All property tests use 2048 cases via `proptest_support::default_config()`.
#[cfg(test)]
#[macro_export]
// ss[related philosophy.structural-hierarchy]
macro_rules! ss_proptest {
    ($($tt:tt)*) => {
        ::proptest::proptest! {
            #![proptest_config($crate::proptest_support::default_config())]
            $($tt)*
        }
    };
}

/// Property tests that call `eager_build` per case — 64 cases via `telemetry_eager_config()`.
#[cfg(test)]
#[macro_export]
// ss[related philosophy.structural-hierarchy]
macro_rules! ss_proptest_telemetry {
    ($($tt:tt)*) => {
        ::proptest::proptest! {
            #![proptest_config($crate::proptest_support::telemetry_eager_config())]
            $($tt)*
        }
    };
}

/// Integrates monitoring with telemetry systems.
///
/// This module provides the glue between runtime monitoring and telemetry output.
// ss[related philosophy.structural-hierarchy]
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
// ss[related philosophy.structural-hierarchy]
pub use actor_builder::CoreBalancer;

///
/// Manage state for actors scros failures and restarts
pub mod state_management;
// ss[related philosophy.structural-hierarchy]
pub use state_management::SteadyState;
// ss[related philosophy.structural-hierarchy]
pub use state_management::new_state;/// Installation utilities for various deployment methods.
// ss[related philosophy.structural-hierarchy]
pub use state_management::new_persistent_state;
// ss[related philosophy.structural-hierarchy]
pub use state_management::StateGuard;

// ss[related philosophy.structural-hierarchy]
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
// ss[related philosophy.structural-hierarchy]
mod core_rx;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::RxCore;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::DoubleSlice;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::DoubleSliceCopy;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::QuadSlice;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::StreamQuadSliceCopy;

/// Low-level transmitter functionality.
///
/// This module contains internal implementations for sending messages.
// ss[related philosophy.structural-hierarchy]
mod core_tx;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_tx::TxCore;

// ss[related philosophy.structural-hierarchy]
pub use crate::distributed::aqueduct_stream::StreamControlItem;

/// Shadow utilities for steady actors.
///
/// This module provides additional functionality for managing actor shadows.
pub mod steady_actor_shadow;

/// Alias for the actor shadow context used throughout the framework.
// ss[related philosophy.structural-hierarchy]
pub type SteadyContext = SteadyActorShadow;

/// Spotlight utilities for steady actors.
///
/// This module provides tools for highlighting or managing actor execution.
pub mod steady_actor_spotlight;

/// Core logic shared between shadow and spotlight actors.
///
/// This module defines the `SteadyActorCore` struct and its methods, which are
/// used by both `SteadyActorShadow` and `SteadyActorSpotlight` to avoid code duplication.
// ss[related philosophy.structural-hierarchy]
mod steady_actor_core;

/// Utilities for managing concurrent execution of futures.
///
/// These exports from `loop_driver` provide functions for selecting and awaiting multiple futures.
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_fuse_future;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_select_two;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_select_three;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_select_four;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_select_five;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_await_for_all_or_proceed_upon_two;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_await_for_all_or_proceed_upon_three;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_await_for_all_or_proceed_upon_four;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_await_for_all_or_proceed_upon_five;

// Public re-exports for convenience
// ss[related philosophy.structural-hierarchy]
pub use clap::*;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor::SendOutcome;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor::index_wait_counts_uniform_usize;
// ss[related philosophy.structural-hierarchy]
pub use simulate_edge::SimRunner;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor_shadow::*;
// ss[related philosophy.structural-hierarchy]
pub use futures_timer::Delay; // for easy use
// ss[related philosophy.structural-hierarchy]
pub use graph_testing::GraphTestResult;
// ss[related philosophy.structural-hierarchy]
pub use monitor::{RxMetaDataHolder, TxMetaDataHolder};
// ss[related philosophy.structural-hierarchy]
pub use channel_builder_units::Rate;
// ss[related philosophy.structural-hierarchy]
pub use channel_builder_units::Filled;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder_units::MCPU;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder_units::Work;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder_units::Percentile;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder::Troupe;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder::ScheduleAs;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder::ScheduleAs::*;
// ss[related philosophy.structural-hierarchy]
pub use graph_liveliness::*;
// ss[related philosophy.structural-hierarchy]
pub use install::serviced::*;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::Rx;
// ss[related philosophy.structural-hierarchy]
pub use steady_tx::Tx;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::SteadyRxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_tx::SteadyTxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::RxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_tx::TxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::RxDone;
// ss[related philosophy.structural-hierarchy]
pub use steady_tx::TxDone;
// ss[related philosophy.structural-hierarchy]
pub use crate::distributed::aqueduct_builder::AqueductBuilder;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor::SteadyActor;
// ss[related philosophy.structural-hierarchy]
pub use distributed::aeron_channel_structs::{
    media_driver_probe, media_driver_probe_default, media_driver_probe_with_reason,
    Channel, Endpoint, MediaType, MediaDriverProbeError,
};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aeron_channel_builder::{AeronConfig, AqueTech};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{StreamEgress, StreamIngress};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundle, LazySteadyStreamTxBundle};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{SteadyStreamRxBundle, SteadyStreamTxBundle};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{LazyStreamRx, LazyStreamTx};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{SteadyStreamRxBundleTrait, StreamRxBundleTrait};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{SteadyStreamTxBundleTrait, StreamTxBundleTrait};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{SteadyStreamRx, SteadyStreamTx, StreamRx, StreamTx};
// ss[related philosophy.structural-hierarchy]
pub use log::{debug, error, info, trace, warn};
// ss[related philosophy.structural-hierarchy]
pub use std::time::{Duration, Instant};
// ss[related philosophy.structural-hierarchy]
pub use std::error::Error;

// Dependencies and internal utilities (legacy `use crate::*` sites in this crate).
// ss[related philosophy.structural-hierarchy]
use futures::select;
// ss[related philosophy.structural-hierarchy]
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use std::io;
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.structural-hierarchy]
use futures::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use std::ops::DerefMut;
#[allow(unused_imports)]
// ss[related philosophy.structural-hierarchy]
use log::*;
// ss[related philosophy.structural-hierarchy]
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


// ss[related philosophy.structural-hierarchy]
mod core_tx_guard;
// ss[related philosophy.structural-hierarchy]
mod core_rx_guard;
// ss[related philosophy.structural-hierarchy]
mod core_rx_stream;
// ss[related philosophy.structural-hierarchy]
mod core_tx_stream;
// ss[related philosophy.structural-hierarchy]
mod channel_stats_tests;
// ss[related philosophy.structural-hierarchy]
mod channel_stats_labels;
// ss[related philosophy.structural-hierarchy]
mod actor_stats_tests;
// ss[related philosophy.structural-hierarchy]
mod actor_builder_units;
// ss[related philosophy.structural-hierarchy]
mod channel_builder_lazy;
// ss[related philosophy.structural-hierarchy]
mod dot_edge;
// ss[related philosophy.structural-hierarchy]
mod dot_node;

// ss[related philosophy.structural-hierarchy]
mod types;
// ss[related philosophy.structural-hierarchy]
mod guard_ext;
// ss[related philosophy.structural-hierarchy]
mod logging;
// ss[related philosophy.structural-hierarchy]
mod metrics;
// ss[related philosophy.structural-hierarchy]
mod runner;

/// meta macros for building our the spotlight
pub mod macros;

// ss[related philosophy.structural-hierarchy]
pub use crate::expression_steady_eye::LAST_FALSE;

// ss[related philosophy.structural-hierarchy]
pub use crate::logging_util::*;
// ss[related philosophy.structural-hierarchy]
use futures::AsyncRead;
// ss[related philosophy.structural-hierarchy]
use futures::AsyncWrite;
// ss[related philosophy.structural-hierarchy]
pub use futures::future::Future;
// ss[related philosophy.structural-hierarchy]
use futures::channel::oneshot;
// ss[related philosophy.structural-hierarchy]
use futures_util::lock::MutexGuard;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor_spotlight::SteadyActorSpotlight;
// ss[related philosophy.structural-hierarchy]
pub use crate::steady_tx::TxMetaDataProvider;
// ss[related philosophy.structural-hierarchy]
pub use crate::steady_rx::RxMetaDataProvider;
// ss[related philosophy.structural-hierarchy]
pub use crate::macros::steady_rx_bundle;
// ss[related philosophy.structural-hierarchy]
pub use crate::macros::steady_tx_bundle;
// ss[related philosophy.structural-hierarchy]
pub use crate::macros::steady_rx_bundle_active;
// ss[related philosophy.structural-hierarchy]
pub use crate::macros::steady_tx_bundle_active;

// ss[related philosophy.structural-hierarchy]
pub use crate::yield_now::yield_now;

// ss[related philosophy.structural-hierarchy]
pub use types::*;
// ss[related philosophy.structural-hierarchy]
pub use guard_ext::SteadyChannelExt;
// ss[related philosophy.structural-hierarchy]
pub use logging::*;
// ss[related philosophy.structural-hierarchy]
pub use metrics::*;
// ss[related philosophy.structural-hierarchy]
pub use runner::SteadyRunner;
