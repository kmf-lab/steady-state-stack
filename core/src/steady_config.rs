//! Configuration options for the SteadyState project.
//!
//! Provides compile-time and runtime configuration for telemetry, debugging behavior,
//! and other internal system settings.

use std::env;

/// Whether the telemetry server is enabled.
/// Enabled if any of the features:
/// - `telemetry_server_cdn`
/// - `telemetry_server_builtin`
/// - `prometheus_metrics`
#[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin", feature = "prometheus_metrics"))]
pub const TELEMETRY_SERVER: bool = true;

#[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin", feature = "prometheus_metrics")))]
pub const TELEMETRY_SERVER: bool = false;

/// Whether actor IDs are logged on creation (for debugging).
pub const SHOW_ACTORS: bool = false;

/// Capacity of the backplane channel for test messages.
pub const BACKPLANE_CAPACITY: usize = 16;

/// Whether telemetry history is enabled (controlled by `telemetry_history` feature).
#[cfg(feature = "telemetry_history")]
pub const TELEMETRY_HISTORY: bool = true;

#[cfg(not(feature = "telemetry_history"))]
pub const TELEMETRY_HISTORY: bool = false;

/// Whether debug-mode failure fast is disabled (controlled by
/// `restart_actors_when_debugging` feature).
///
/// - `true`: supervisors will restart actors on failure even in debug mode.
/// - `false`: failure will halt for debugging.
#[cfg(feature = "restart_actors_when_debugging")]
pub const DISABLE_DEBUG_FAIL_FAST: bool = true;

#[cfg(not(feature = "restart_actors_when_debugging"))]
pub const DISABLE_DEBUG_FAIL_FAST: bool = false;

/// Maximum seconds between repeated telemetry error reports.
pub const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 20;

/// Number of slots in the real channel for telemetry collection.
pub const REAL_CHANNEL_LENGTH_TO_COLLECTOR: usize = 64;

/// Number of messages consumed by the collector (half of collector channel length).
pub const CONSUMED_MESSAGES_BY_COLLECTOR: usize = REAL_CHANNEL_LENGTH_TO_COLLECTOR / 2;


//should be big enought to hold one message for every actor, on graph def we need this much space
//for large graphs this will be fine as we consume and produce and await until it is done.
//this must be large enought to hold all actors which may panic at the same moment.
pub const REAL_CHANNEL_LENGTH_TO_FEATURE: usize = 256;

// Default values for runtime configuration
const DEFAULT_TELEMETRY_SERVER_PORT: u16 = 9900;
const DEFAULT_TELEMETRY_SERVER_IP: &str = "0.0.0.0";

/// Retrieves the telemetry server port, reading from the `TELEMETRY_SERVER_PORT` environment
/// variable. Falls back to a sensible default if the variable is unset or invalid.
///
/// # Behavior
/// - If `TELEMETRY_SERVER_PORT` is unset, returns `DEFAULT_TELEMETRY_SERVER_PORT`.
/// - If the variable is set but cannot be parsed as `u16`, returns the default.
pub(crate) fn telemetry_server_port() -> u16 {
    env::var("TELEMETRY_SERVER_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_TELEMETRY_SERVER_PORT)
}

/// Retrieves the telemetry server IP address, reading from the `TELEMETRY_SERVER_IP`
/// environment variable. Falls back to a sensible default if the variable is unset.
///
/// # Behavior
/// - If `TELEMETRY_SERVER_IP` is unset, returns `DEFAULT_TELEMETRY_SERVER_IP`.
pub(crate) fn telemetry_server_ip() -> String {
    env::var("TELEMETRY_SERVER_IP").unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_IP.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_default_constants() {
        // Under default compilation (no special features)
        assert!(TELEMETRY_SERVER);
        assert!(!SHOW_ACTORS);
        assert_eq!(BACKPLANE_CAPACITY, 16);
        assert!(!TELEMETRY_HISTORY);
        assert!(!DISABLE_DEBUG_FAIL_FAST);
        assert_eq!(MAX_TELEMETRY_ERROR_RATE_SECONDS, 20);
        assert_eq!(REAL_CHANNEL_LENGTH_TO_COLLECTOR, 64);
        assert_eq!(CONSUMED_MESSAGES_BY_COLLECTOR, 32);
        assert_eq!(REAL_CHANNEL_LENGTH_TO_FEATURE, 32);
    }

    #[test]
    fn test_telemetry_server_port_env_handling() {
        unsafe {
            env::remove_var("TELEMETRY_SERVER_PORT");
            assert_eq!(telemetry_server_port(), DEFAULT_TELEMETRY_SERVER_PORT);

            env::set_var("TELEMETRY_SERVER_PORT", "9100");
            assert_eq!(telemetry_server_port(), 9100);

            env::set_var("TELEMETRY_SERVER_PORT", "not_a_number");
            // invalid values fall back to default
            assert_eq!(telemetry_server_port(), DEFAULT_TELEMETRY_SERVER_PORT);
        }
    }

    #[test]
    fn test_telemetry_server_ip_env_handling() {
        unsafe {
            env::remove_var("TELEMETRY_SERVER_IP");
            assert_eq!(telemetry_server_ip(), DEFAULT_TELEMETRY_SERVER_IP);

            env::set_var("TELEMETRY_SERVER_IP", "127.0.0.1");
            assert_eq!(telemetry_server_ip(), "127.0.0.1");
        }
    }
}
