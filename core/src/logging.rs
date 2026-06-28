//! Logging initialization and configuration for Steady State applications.

use clap::ValueEnum;

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
// ss[related philosophy.structural-hierarchy]
pub fn init_logging(
    loglevel: LogLevel,
    file_config: Option<LogFileConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
    crate::logging_util::steady_logger::initialize_with_level_and_file(loglevel, file_config)
}

/// Configuration for file-based logging with rotation.
#[derive(Clone, Debug)]
// ss[related philosophy.structural-hierarchy]
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
// ss[related philosophy.structural-hierarchy]
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

// ss[related philosophy.structural-hierarchy]
impl LogLevel {
    /// Converts this `LogLevel` to the corresponding `log::LevelFilter`.
    ///
    /// # Returns
    /// - `log::LevelFilter`: The matching filter level for logging.
    // ss[related philosophy.structural-hierarchy]
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

#[cfg(test)]
mod tests {
    use super::{init_logging, LogFileConfig, LogLevel};

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn log_level_to_level_filter_maps_all_variants() {
        assert_eq!(LogLevel::Off.to_level_filter(), log::LevelFilter::Off);
        assert_eq!(LogLevel::Error.to_level_filter(), log::LevelFilter::Error);
        assert_eq!(LogLevel::Warn.to_level_filter(), log::LevelFilter::Warn);
        assert_eq!(LogLevel::Info.to_level_filter(), log::LevelFilter::Info);
        assert_eq!(LogLevel::Debug.to_level_filter(), log::LevelFilter::Debug);
        assert_eq!(LogLevel::Trace.to_level_filter(), log::LevelFilter::Trace);
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn init_logging_without_file_config_succeeds() {
        init_logging(LogLevel::Warn, None).expect("init logging");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn log_file_config_fields_roundtrip() {
        let cfg = LogFileConfig {
            directory: "/tmp/steady_logs".into(),
            base_name: "graph".into(),
            max_size_bytes: 1_048_576,
            keep_count: 5,
            delete_old_on_start: true,
        };
        assert_eq!(cfg.directory, "/tmp/steady_logs");
        assert_eq!(cfg.base_name, "graph");
        assert_eq!(cfg.max_size_bytes, 1_048_576);
        assert_eq!(cfg.keep_count, 5);
        assert!(cfg.delete_old_on_start);
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn init_logging_with_file_config_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = LogFileConfig {
            directory: dir.path().to_string_lossy().into_owned(),
            base_name: "steady_test".into(),
            max_size_bytes: 4096,
            keep_count: 2,
            delete_old_on_start: false,
        };
        init_logging(LogLevel::Debug, Some(cfg)).expect("file logging init");
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn init_logging_reinit_with_same_level_succeeds() {
        init_logging(LogLevel::Warn, None).expect("first init");
        init_logging(LogLevel::Warn, None).expect("reinit same level");
    }
}
