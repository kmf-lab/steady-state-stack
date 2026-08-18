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

/// Shared frame-based refresh/window bit sizing for actor and channel telemetry.
pub(crate) mod telemetry_window;

/// Internal module for graph visualization and DOT language integration.
pub(crate) mod dot;

/// Unified edge-slot merge for telemetry channel ids (`DotState.edges`).
///
/// Operators: conflicting endpoints are logged under target `steady_state::telemetry::dot`.
pub(crate) mod dot_unify;

/// Manages the lifecycle states of actor graphs.
///
/// This module provides utilities for ensuring the liveliness and proper shutdown of actor graphs.
// ss[related philosophy.structural-hierarchy]
mod graph;
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

// ss[related philosophy.structural-hierarchy]
pub(crate) use abstract_executor::core_exec;

/// Utilities for capturing panics during testing.
///
/// This module is only available in test configurations.
#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
mod test_panic_capture;

/// Property-test case count (shared with `proptest_support::SS_PROPCASES`).
pub const SS_PROPCASES: u32 = 2048;

/// Shared proptest strategies and channel harness helpers.
#[cfg(test)]
#[doc(hidden)]
pub mod proptest_support;

/// All property tests use 2048 cases via `proptest_support::default_config()`.
#[cfg(test)]
#[macro_export]
macro_rules! ss_proptest {
    ($($tt:tt)*) => {
        ::proptest::proptest! {
            #![proptest_config($crate::proptest_support::default_config())]
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
pub use state_management::new_state;/// Installation utilities for various deployment methods.
pub use state_management::new_persistent_state;
// ss[related philosophy.structural-hierarchy]
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
// ss[related philosophy.structural-hierarchy]
mod core_rx;
pub use crate::core_rx::RxCore;
pub use crate::core_rx::DoubleSlice;
// ss[related philosophy.structural-hierarchy]
pub use crate::core_rx::DoubleSliceCopy;
pub use crate::core_rx::QuadSlice;
pub use crate::core_rx::StreamQuadSliceCopy;

/// Low-level transmitter functionality.
///
/// This module contains internal implementations for sending messages.
// ss[related philosophy.structural-hierarchy]
mod core_tx;
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
pub use loop_driver::steady_select_two;
pub use loop_driver::steady_select_three;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_select_four;
pub use loop_driver::steady_select_five;
pub use loop_driver::steady_await_for_all_or_proceed_upon_two;
// ss[related philosophy.structural-hierarchy]
pub use loop_driver::steady_await_for_all_or_proceed_upon_three;
pub use loop_driver::steady_await_for_all_or_proceed_upon_four;
pub use loop_driver::steady_await_for_all_or_proceed_upon_five;

// Public re-exports for convenience
// ss[related philosophy.structural-hierarchy]
pub use clap::*;
pub use steady_actor::SendOutcome;
pub use steady_actor::index_wait_counts_uniform_usize;
// ss[related philosophy.structural-hierarchy]
pub use simulate_edge::SimRunner;
pub use steady_actor_shadow::*;
pub use futures_timer::Delay; // for easy use
// ss[related philosophy.structural-hierarchy]
pub use graph_testing::GraphTestResult;
pub use monitor::{RxMetaDataHolder, TxMetaDataHolder};
pub use channel_builder_units::Rate;
// ss[related philosophy.structural-hierarchy]
pub use channel_builder_units::Filled;
pub use actor_builder_units::MCPU;
pub use actor_builder_units::Work;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder_units::Percentile;
pub use actor_builder::Troupe;
pub use actor_builder::ScheduleAs;
// ss[related philosophy.structural-hierarchy]
pub use actor_builder::ScheduleAs::*;
pub use graph_liveliness::*;
pub use install::serviced::*;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::Rx;
pub use steady_tx::Tx;
pub use steady_rx::SteadyRxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_tx::SteadyTxBundleTrait;
pub use steady_rx::RxBundleTrait;
pub use steady_tx::TxBundleTrait;
// ss[related philosophy.structural-hierarchy]
pub use steady_rx::RxDone;
pub use steady_tx::TxDone;
pub use crate::distributed::aqueduct_builder::AqueductBuilder;
// ss[related philosophy.structural-hierarchy]
pub use steady_actor::SteadyActor;
pub use distributed::aeron_channel_structs::{
    media_driver_probe, media_driver_probe_default, media_driver_probe_with_reason,
    Channel, Endpoint, MediaType, MediaDriverProbeError,
};
pub use distributed::aeron_channel_builder::{AeronConfig, AqueTech};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{StreamEgress, StreamIngress};
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundle, LazySteadyStreamTxBundle};
pub use distributed::aqueduct_stream::{SteadyStreamRxBundle, SteadyStreamTxBundle};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{LazyStreamRx, LazyStreamTx};
pub use distributed::aqueduct_stream::{SteadyStreamRxBundleTrait, StreamRxBundleTrait};
pub use distributed::aqueduct_stream::{SteadyStreamTxBundleTrait, StreamTxBundleTrait};
// ss[related philosophy.structural-hierarchy]
pub use distributed::aqueduct_stream::{LazySteadyStreamRxBundleClone, LazySteadyStreamTxBundleClone};
pub use distributed::aqueduct_stream::{SteadyStreamRx, SteadyStreamTx, StreamRx, StreamTx};
pub use log::{debug, error, info, trace, warn};
// ss[related philosophy.structural-hierarchy]
pub use std::time::{Duration, Instant};
pub use std::error::Error;

// Dependencies and internal utilities (legacy `use crate::*` sites in this crate).
// ss[related philosophy.structural-hierarchy]
use futures::select;
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use std::io;
use std::sync::Arc;
use futures::lock::Mutex;
// ss[related philosophy.structural-hierarchy]
use std::ops::DerefMut;
#[allow(unused_imports)]
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
mod core_rx_guard;
mod core_rx_stream;
// ss[related philosophy.structural-hierarchy]
mod core_tx_stream;
mod channel_stats_tests;
mod channel_stats_labels;
// ss[related philosophy.structural-hierarchy]
mod actor_stats_tests;
mod actor_builder_units;
mod channel_builder_lazy;
// ss[related philosophy.structural-hierarchy]
mod dot_edge;
mod dot_node;

mod types;
mod guard_ext;
mod logging;
mod metrics;
mod runner;

/// meta macros for building our the spotlight
pub mod macros;

// ss[related philosophy.structural-hierarchy]
pub use crate::expression_steady_eye::LAST_FALSE;

pub use crate::logging_util::*;
// ss[related philosophy.structural-hierarchy]
use futures::AsyncRead;
use futures::AsyncWrite;
pub use futures::future::Future;
// ss[related philosophy.structural-hierarchy]
use futures::channel::oneshot;
use futures_util::lock::MutexGuard;
pub use steady_actor_spotlight::SteadyActorSpotlight;
// ss[related philosophy.structural-hierarchy]
pub use crate::steady_tx::TxMetaDataProvider;
pub use crate::steady_rx::RxMetaDataProvider;
pub use crate::macros::steady_rx_bundle;
// ss[related philosophy.structural-hierarchy]
pub use crate::macros::steady_tx_bundle;
pub use crate::macros::steady_rx_bundle_active;
pub use crate::macros::steady_tx_bundle_active;

// ss[related philosophy.structural-hierarchy]
pub use crate::yield_now::yield_now;

pub use types::*;
pub use guard_ext::SteadyChannelExt;
pub use logging::*;
pub use metrics::*;
pub use runner::SteadyRunner;
