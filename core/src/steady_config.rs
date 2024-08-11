//! Configuration module for the SteadyState project.
//!
//! This module defines various constants and utility functions for configuring telemetry, debugging behavior,
//! and other settings within the SteadyState project. The configuration options are set using compile-time
//! features and environment variables.

use std::env;

/// Indicates whether the telemetry server is enabled.
/// This is determined by the presence of any of the following features:
/// - `telemetry_server_cdn`
/// - `telemetry_server_builtin`
/// - `prometheus_metrics`
#[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin", feature = "prometheus_metrics"))]
pub const TELEMETRY_SERVER: bool = true;

#[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin", feature = "prometheus_metrics")))]
pub const TELEMETRY_SERVER: bool = false;

pub const SHOW_ACTORS: bool = false; //if we want to see each actor ID logged upon creation
pub const BACKPLANE_CAPACITY: usize = 16; //for test messages

/// Indicates whether telemetry history is enabled.
/// This is determined by the presence of the `telemetry_history` feature.
#[cfg(feature = "telemetry_history")]
pub const TELEMETRY_HISTORY: bool = true;

#[cfg(not(feature = "telemetry_history"))]
pub const TELEMETRY_HISTORY: bool = false;

/// Determines if supervisors should restart actors while debugging.
/// When set to `false`, it allows debugging of the actor that failed.
/// When set to `true`, supervisors will always restart actors, even in debug mode.
/// This is controlled by the `restart_actors_when_debugging` feature.
#[cfg(not(feature = "restart_actors_when_debugging"))]
pub const DISABLE_DEBUG_FAIL_FAST: bool = false;

#[cfg(feature = "restart_actors_when_debugging")]
pub const DISABLE_DEBUG_FAIL_FAST: bool = true;

//////////////////////////////////////////////////////////

/// Default port for the telemetry server.
const DEFAULT_TELEMETRY_SERVER_PORT: &str = "9100";

/// Retrieves the telemetry server port from the environment variable `TELEMETRY_SERVER_PORT`.
/// If not set, it defaults to `DEFAULT_TELEMETRY_SERVER_PORT`.
///
/// # Panics
///
/// Panics if the `TELEMETRY_SERVER_PORT` is not a valid `u16`.
pub(crate) fn telemetry_server_port() -> u16 {
    env::var("TELEMETRY_SERVER_PORT")
        .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_PORT.to_string())
        .parse::<u16>()
        .expect("TELEMETRY_SERVER_PORT must be a valid u16")
}

/// Default IP address for the telemetry server.
const DEFAULT_TELEMETRY_SERVER_IP: &str = "0.0.0.0";

/// Retrieves the telemetry server IP address from the environment variable `TELEMETRY_SERVER_IP`.
/// If not set, it defaults to `DEFAULT_TELEMETRY_SERVER_IP`.
pub(crate) fn telemetry_server_ip() -> String {
    env::var("TELEMETRY_SERVER_IP")
        .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_IP.to_string())
}

//////////////////////////////////////////////////////////

/// The maximum rate in seconds at which the same telemetry error will be reported.
/// This avoids filling logs with repeated errors for the same issue on the same channel.
pub const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 20;

//////////////////////////////////////////////////////////

/// Granularity of the frames for telemetry data collection.
/// Larger values consume more memory but allow for faster capture rates and higher accuracy.
pub const REAL_CHANNEL_LENGTH_TO_COLLECTOR: usize = 256;

/// Number of messages consumed by the collector.
/// Larger values take up memory but allow faster capture rates.
pub const CONSUMED_MESSAGES_BY_COLLECTOR: usize = REAL_CHANNEL_LENGTH_TO_COLLECTOR >> 1;

/// Length of the channel for feature processing.
/// Allows features to fall behind with minimal latency.
pub const REAL_CHANNEL_LENGTH_TO_FEATURE: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_telemetry_server_port_env_var() {
        env::remove_var("TELEMETRY_SERVER_PORT");
        assert_eq!(telemetry_server_port(), 9100);
        env::set_var("TELEMETRY_SERVER_PORT", "9200");
        assert_eq!(telemetry_server_port(), 9200);
        env::remove_var("TELEMETRY_SERVER_PORT");
        assert_eq!(telemetry_server_port(), 9100);

    }

    #[test]
    fn test_telemetry_server_ip_env_var() {
        env::remove_var("TELEMETRY_SERVER_IP");
        assert_eq!(telemetry_server_ip(), "0.0.0.0");
        env::set_var("TELEMETRY_SERVER_IP", "127.0.0.1");
        assert_eq!(telemetry_server_ip(), "127.0.0.1");
        env::remove_var("TELEMETRY_SERVER_IP");

    }
}

