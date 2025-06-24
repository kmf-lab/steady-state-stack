//! The `util` module provides helper functions and types for logging initialization,
//! in-memory log capturing, and assertion macros for testing within the Steady framework.
//!
//! This module enables per-test log capture, dynamic log level control, and
//! robust assertions on log output, supporting both serial and parallel test execution.

use flexi_logger::writers::*;
use flexi_logger::*;
use std::sync::{Mutex, Arc};
use std::error::Error;
use std::io;
#[allow(unused_imports)]
use log::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::thread::{self, ThreadId};
use crate::LogLevel;
use lazy_static::lazy_static;

/// A simple memory writer that stores log messages per test context.
///
/// This writer is used to capture log output in memory for each test thread,
/// enabling assertions on log content during testing.
struct MemoryWriter {
    /// The formatting function used for log records.
    format: FormatFunction,
}

impl MemoryWriter {
    /// Creates a new `MemoryWriter` with the specified formatting function.
    fn new(format: FormatFunction) -> Self {
        MemoryWriter { format }
    }
}

/// State for capturing logs in a test context.
///
/// Each test thread has its own capture state, including a flag for
/// whether capturing is active and a buffer for captured log messages.
#[derive(Clone)]
pub struct TestCaptureState {
    /// Indicates if log capturing is currently active for this test.
    is_capturing: Arc<AtomicBool>,
    /// Buffer holding captured log messages for this test.
    pub log_buffer: Arc<Mutex<Vec<String>>>, // pub for the macro
}

lazy_static! {
    /// Global map of test contexts keyed by test thread ID.
    ///
    /// Each entry holds the capture state for a test running on a given thread.
    pub static ref TEST_CONTEXTS: Arc<Mutex<HashMap<ThreadId, TestCaptureState>>> = Arc::new(Mutex::new(HashMap::new())); // pub for the macro

    /// Flag to track if the logger is in test mode.
    static ref IS_TEST_MODE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
}

impl LogWriter for MemoryWriter {
    /// Writes a log record to all active test contexts' buffers.
    ///
    /// This method is called by the logger for each log record. In test mode,
    /// it appends the formatted log message to the buffer of every active test context.
    fn write(&self, now: &mut DeferredNow, record: &Record) -> io::Result<()> {
        let mut buffer = Vec::new();
        (self.format)(&mut buffer, now, record)?;
        let formatted = String::from_utf8_lossy(&buffer).to_string();
        // In test mode, write to ALL active test contexts
        if let Ok(contexts) = TEST_CONTEXTS.lock() {
            for (_key, test_state) in contexts.iter() {
                if test_state.is_capturing.load(Ordering::SeqCst) {
                    if let Ok(mut log_buf) = test_state.log_buffer.lock() {
                        log_buf.push(formatted.clone());
                    }
                }
            }
        } else {
            eprintln!("Error getting test contexts lock");
        }
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    fn max_log_level(&self) -> LevelFilter {
        LevelFilter::max()
    }
}

/// Initializes the FlexiLogger logger for the Steady framework.
///
/// This function sets up the logger with the specified log level, using a memory writer
/// for capturing logs and duplicating output to stderr. It is used internally by the
/// logger initialization routines.
fn steady_logging_init(level: LogLevel) -> Result<LoggerHandle, Box<dyn Error>> {
    let log_spec = LogSpecBuilder::new()
        .default(level.to_level_filter())
        .build();
    let format = colored_with_thread;
    let mut logger = Logger::with(log_spec);
    logger = logger
        .log_to_writer(Box::new(MemoryWriter::new(format)))
        .duplicate_to_stderr(flexi_logger::Duplicate::All);

    logger
        .format(format)
        .write_mode(WriteMode::Direct)
        .start()
        .map_err(|e| Box::new(e) as Box<dyn Error>)
}

/// Provides functions to initialize and manage the global logger, including log capturing for tests.
///
/// This submodule manages logger initialization, per-test log capture, and dynamic log level changes.
/// It is designed to support both normal operation and test environments.
pub mod steady_logger {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        /// Holds the global logger handle, if initialized.
        static ref LOGGER_HANDLE: Mutex<Option<LoggerHandle>> = Mutex::new(None);
    }

    /// Guard object for managing log capture in tests.
    ///
    /// When dropped, this guard stops capturing logs for the current thread.
    pub struct LogCaptureGuard {
        thread_id: ThreadId,
    }

    impl Drop for LogCaptureGuard {
        /// Stops capturing logs when the guard is dropped.
        fn drop(&mut self) {
            stop_capturing_logs(self.thread_id);
        }
    }

    /// Starts capturing logs for the current test thread and returns a guard.
    ///
    /// This function enables test mode, initializes the logger if needed,
    /// and registers a new capture state for the current thread.
    pub fn start_log_capture() -> LogCaptureGuard {
        let thread_id = thread::current().id();

        // Enable test mode when capture starts
        IS_TEST_MODE.store(true, Ordering::SeqCst);

        // Initialize logger in test mode if not already done
        let _ = initialize();

        let test_state = TestCaptureState {
            is_capturing: Arc::new(AtomicBool::new(true)),
            log_buffer: Arc::new(Mutex::new(Vec::new())),
        };

        // Register this test context
        if let Ok(mut contexts) = TEST_CONTEXTS.lock() {
            contexts.insert(thread_id, test_state);
        }

        LogCaptureGuard { thread_id }
    }

    /// Initializes the logger with a default log level if not already initialized.
    ///
    /// This function is idempotent and safe to call multiple times.
    pub fn initialize() -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().expect("log init");
        if logger_handle.is_none() {
            let level = LogLevel::Info; // Default level
            let handle = steady_logging_init(level)?;
            *logger_handle = Some(handle);
        }
        Ok(())
    }

    /// Initializes the logger with a specific log level, or changes the level if already initialized.
    ///
    /// This function allows dynamic adjustment of the log level at runtime.
    pub fn initialize_with_level(level: LogLevel) -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().expect("log init with level");
        if let Some(handle) = logger_handle.as_ref() {
            // Dynamically adjust the log level
            handle.set_new_spec(
                LogSpecBuilder::new()
                    .default(level.to_level_filter())
                    .build()
            );
            Ok(())
        } else {
            let handle = steady_logging_init(level)?;
            *logger_handle = Some(handle);
            Ok(())
        }
    }

    /// Stops capturing logs and removes the test's log buffer for the given thread.
    ///
    /// This function is called automatically when a `LogCaptureGuard` is dropped.
    fn stop_capturing_logs(thread_id: ThreadId) {
        if let Ok(mut contexts) = TEST_CONTEXTS.lock() {
            if let Some(test_state) = contexts.get(&thread_id) {
                test_state.is_capturing.store(false, Ordering::SeqCst);
            }
            contexts.remove(&thread_id);
        }
    }
}

/// Asserts that the given sequence of texts appears in the captured logs in order.
///
/// This macro is intended for use in tests. It checks that each string in the provided
/// slice appears in the log buffer for the current test thread, in the specified order.
/// If any expected text is missing, the macro panics and prints the captured logs for debugging.
#[macro_export]
macro_rules! assert_in_logs {
    ($texts:expr) => {{
        // Add a small delay to ensure all async logs are processed
        std::thread::sleep(std::time::Duration::from_millis(10));

        let thread_id = std::thread::current().id();

        let logged_messages = if let Ok(contexts) = TEST_CONTEXTS.lock() {
            if let Some(test_state) = contexts.get(&thread_id) {
                if let Ok(buf) = test_state.log_buffer.lock() {
                    buf.clone()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let texts = $texts;
        let mut text_index = 0;
        for msg in logged_messages.iter() {
            if text_index < texts.len() && msg.contains(texts[text_index]) {
                text_index += 1;
            }
        }
        if text_index < texts.len() {
            for (i, msg) in logged_messages.iter().enumerate() {
                eprintln!("[{}]: {}", i, msg);
            }
            eprintln!("Expected texts: {:?}", texts);
            eprintln!("Found {} out of {} expected texts.", text_index, texts.len());

            panic!(
                "Assertion failed at {}:{}: expected texts {:?} in logs {:?}",
                file!(),
                line!(),
                texts,
                logged_messages
            );
        }
    }};
}