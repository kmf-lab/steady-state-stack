//! The `util` module provides helper functions and types for logging initialization,
//! memory-based log capturing, and assertion macros for testing within the Steady framework.
use flexi_logger::writers::*;
use flexi_logger::*;
use std::sync::{Mutex, Arc};
use std::error::Error;
use std::io;
use log::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::thread::{self, ThreadId};
use crate::LogLevel;
use lazy_static::lazy_static;

/// A simple memory writer that stores log messages per test context.
struct MemoryWriter {
    format: FormatFunction,
}

impl MemoryWriter {
    fn new(format: FormatFunction) -> Self {
        MemoryWriter { format }
    }
}

/// Per-test capture state
#[derive(Clone)]
pub struct TestCaptureState {
    is_capturing: Arc<AtomicBool>,
    pub log_buffer: Arc<Mutex<Vec<String>>>, // pub for the macro
}

lazy_static! {
    /// Global map of test contexts keyed by test thread ID
    pub static ref TEST_CONTEXTS: Arc<Mutex<HashMap<ThreadId, TestCaptureState>>> = Arc::new(Mutex::new(HashMap::new())); //pub for the macro
    /// Flag to track if we're in test mode
    static ref IS_TEST_MODE: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
}

impl LogWriter for MemoryWriter {
    fn write(&self, now: &mut DeferredNow, record: &Record) -> io::Result<()> {
        let mut buffer = Vec::new();
        (self.format)(&mut buffer, now, record)?;
        let thread_id = thread::current().id();
        let formatted = String::from_utf8_lossy(&buffer).to_string();
        // In test mode, write to ALL active test contexts
        if let Ok(contexts) = TEST_CONTEXTS.lock() {
            for (_key,test_state) in contexts.iter() {
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

/// Core helper to initialize FlexiLogger.
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
pub mod steady_logger {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref LOGGER_HANDLE: Mutex<Option<LoggerHandle>> = Mutex::new(None);
    }

    /// Guard to manage log capturing in tests.
    pub struct LogCaptureGuard {
        thread_id: ThreadId,
        start_time: std::time::Instant,
    }

    impl Drop for LogCaptureGuard {
        fn drop(&mut self) {
            stop_capturing_logs(self.thread_id);
        }
    }

    /// Starts capturing logs for the current test thread, returns a guard.
    pub fn start_log_capture() -> LogCaptureGuard {
        let thread_id = thread::current().id();
        //eprintln!("a thread {:?} ",thread_id);

        let start_time = std::time::Instant::now();

        // Enable test mode when capture starts
        IS_TEST_MODE.store(true, Ordering::SeqCst);

        // Initialize logger in test mode if not already done
        let _ = initialize_with_level(LogLevel::Trace);

        let test_state = TestCaptureState {
            is_capturing: Arc::new(AtomicBool::new(true)),
            log_buffer: Arc::new(Mutex::new(Vec::new())),
        };

        // Register this test context
        if let Ok(mut contexts) = TEST_CONTEXTS.lock() {
            contexts.insert(thread_id, test_state);
        }

        LogCaptureGuard { thread_id, start_time }
    }

    /// Initializes the logger based on the compilation context with a default level.
    pub fn initialize() -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().expect("log init");
        if logger_handle.is_none() {
            let level = LogLevel::Info; // Default level
            let handle = steady_logging_init(level)?;
            *logger_handle = Some(handle);
        }
        Ok(())
    }

    /// Initializes the logger with a specific level or changes the level if already initialized.
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

    /// Stops capturing logs and clears the test's log buffer.
    fn stop_capturing_logs(thread_id: ThreadId) {
        if let Ok(mut contexts) = TEST_CONTEXTS.lock() {
            if let Some(test_state) = contexts.get(&thread_id) {
                test_state.is_capturing.store(false, Ordering::SeqCst);
                if let Ok(buf) = test_state.log_buffer.lock() {
                    if !buf.is_empty() {
                        eprintln!("Test logs captured: {:?}", *buf);
                    }
                }
            }
            contexts.remove(&thread_id);
        }
    }
}

/// Asserts that the given sequence of texts appears in the captured logs in order.
///
/// Takes a slice of string literals and verifies each occurs in the current test's
/// log buffer in the specified order, panicking with detailed log output if
/// any expected text is missing.
#[macro_export]
macro_rules! assert_in_logs {
    ($texts:expr) => {{
        // Add a small delay to ensure all async logs are processed
        std::thread::sleep(std::time::Duration::from_millis(10));

        let thread_id = std::thread::current().id();
        // eprintln!("thread f {:?} ",thread_id);

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
        eprintln!("logged_messages texts: {:?}", logged_messages);

        let texts = $texts;
        let mut text_index = 0;
        for msg in logged_messages.iter() {
            if text_index < texts.len() && msg.contains(texts[text_index]) {
                text_index += 1;
            }
        }
        if text_index < texts.len() {
            // Print all the content for easier check with index
            eprintln!("Captured logs ({} messages):", logged_messages.len());
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

#[cfg(test)]
mod test_log_tests {
    use super::*;
    use steady_logger::*;
    use log::info;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_log_capture() {
        let _guard = start_log_capture();
        info!("Hello from test!");
        info!("Yet Again!");
        assert_in_logs!(["Hello from test!", "Yet Again!"]);
    }

    #[test]
    fn test_log_isolation() {
        let _guard = start_log_capture();
        info!("Test 2 log");
        assert_in_logs!(["Test 2 log"]);
    }

    #[test]
    fn test_cross_thread_capture() {
        let _guard = start_log_capture();

        info!("Main thread log");

        // Spawn a worker thread that should also be captured
        let handle = thread::spawn(|| {
            thread::sleep(Duration::from_millis(10));
            info!("Worker thread log");
        });

        handle.join().expect("internal error");
        thread::sleep(Duration::from_millis(50)); // Allow log to be processed

        assert_in_logs!(["Main thread log", "Worker thread log"]);
    }

    #[test]
    fn test_parallel_test_isolation_a() {
        let _guard = start_log_capture();

        info!("Parallel test A");
        thread::sleep(Duration::from_millis(10)); // Small delay to ensure log is processed

        // This might see logs from other parallel tests, but should at least see its own
        let logged_messages = if let Ok(contexts) = TEST_CONTEXTS.lock() {
            if let Some(test_state) = contexts.get(&thread::current().id()) {
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

        let has_own_log = logged_messages.iter().any(|msg| msg.contains("Parallel test A"));
        assert!(has_own_log, "Should capture its own log message");
    }

    #[test]
    fn test_parallel_test_isolation_b() {
        let _guard = start_log_capture();

        info!("Parallel test B");
        thread::sleep(Duration::from_millis(10)); // Small delay to ensure log is processed

        // This might see logs from other parallel tests, but should at least see its own
        let logged_messages = if let Ok(contexts) = TEST_CONTEXTS.lock() {
            if let Some(test_state) = contexts.get(&thread::current().id()) {
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

        let has_own_log = logged_messages.iter().any(|msg| msg.contains("Parallel test B"));
        assert!(has_own_log, "Should capture its own log message");
    }

    #[test]
    fn test_no_capture_without_guard() {
        // Reset test mode for this test
        IS_TEST_MODE.store(false, Ordering::SeqCst);

        // Initialize in non-test mode
        let _ = initialize();

        info!("This should not be captured");

        // Since no guard exists, there should be no test context
        let thread_id = thread::current().id();
        let has_context = if let Ok(contexts) = TEST_CONTEXTS.lock() {
            contexts.contains_key(&thread_id)
        } else {
            false
        };

        assert!(!has_context, "No test context should exist without guard");
    }

    #[test]
    fn test_multiple_captures_same_thread() {
        {
            let _guard1 = start_log_capture();
            info!("First capture");
            assert_in_logs!(["First capture"]);
        } // guard1 drops here

        {
            let _guard2 = start_log_capture();
            info!("Second capture");
            assert_in_logs!(["Second capture"]);
        } // guard2 drops here
    }

    #[test]
    fn test_sequential_captures() {
        // Test that captures don't interfere with each other
        for i in 0..3 {
            let _guard = start_log_capture();
            info!("Sequential test {}", i);
            assert_in_logs!([&format!("Sequential test {}", i)]);
        }
    }
}
