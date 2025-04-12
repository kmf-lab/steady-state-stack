use flexi_logger::writers::*;
use flexi_logger::*;
use std::sync::{Arc, Mutex};
use std::error::Error;
use std::io;
use std::io::Write;
use log::Record;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::LogLevel;

/// A simple memory writer that stores log messages in a thread-local Vec<String>.
struct MemoryWriter {
    format: FormatFunction,
}

impl MemoryWriter {
    fn new(format: FormatFunction) -> Self {
        MemoryWriter { format }
    }
}

thread_local! {
    pub static LOG_BUFFER: RefCell<Vec<String>> = RefCell::new(Vec::new());
    pub static IS_CAPTURING: AtomicBool = AtomicBool::new(false);
}

impl LogWriter for MemoryWriter {
    fn write(&self, now: &mut DeferredNow, record: &Record) -> io::Result<()> {
        let is_capturing = IS_CAPTURING.with(|cap| cap.load(Ordering::SeqCst));
        if is_capturing {
            let mut buffer = Vec::new();
            (self.format)(&mut buffer, now, record)?;
            let formatted = String::from_utf8_lossy(&buffer).to_string();
            LOG_BUFFER.with(|buf| {
                buf.borrow_mut().push(formatted);
            });
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
fn steady_logging_init(level: LogLevel, test_mode: bool) -> Result<LoggerHandle, Box<dyn Error>> {
    let log_spec = LogSpecBuilder::new()
        .default(level.to_level_filter())
        .build();
    let format = colored_with_thread;
    let mut logger = Logger::with(log_spec);
    if test_mode {
        logger = logger
            .log_to_writer(Box::new(MemoryWriter::new(format)))
            .duplicate_to_stderr(flexi_logger::Duplicate::All);
    } else {
        logger = logger.log_to_stderr();
    }
    logger
        .format(format)
        .write_mode(WriteMode::Direct)
        .start()
        .map_err(|e| Box::new(e) as Box<dyn Error>)
}

pub mod steady_logger {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref LOGGER_HANDLE: Mutex<Option<LoggerHandle>> = Mutex::new(None);
    }

    /// Guard to manage log capturing in tests.
    pub struct LogCaptureGuard;

    impl Drop for LogCaptureGuard {
        fn drop(&mut self) {
            stop_capturing_logs();
        }
    }

    /// Starts capturing logs for the current thread, returns a guard.
    pub fn start_capture() -> LogCaptureGuard {
        IS_CAPTURING.with(|cap| cap.store(true, Ordering::SeqCst));
        LogCaptureGuard
    }

    /// Initializes the logger based on the compilation context with a default level.
    pub fn initialize() -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().expect("log init");
        if logger_handle.is_none() {
            let test_mode = cfg!(test);
            let level = LogLevel::Info; // Default level
            let handle = steady_logging_init(level, test_mode)?;
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
            let test_mode = cfg!(test);
            let handle = steady_logging_init(level, test_mode)?;
            *logger_handle = Some(handle);
            Ok(())
        }
    }

    /// Stops capturing logs and clears the thread-local log buffer.
    pub fn stop_capturing_logs() {
        IS_CAPTURING.with(|cap| cap.store(false, Ordering::SeqCst));
        LOG_BUFFER.with(|buf| buf.borrow_mut().clear());
    }
}

/// Assertion macro to check if texts appear in logs in order.
#[macro_export]
macro_rules! assert_in_logs {
    ($texts:expr) => {{
        let logged_messages = LOG_BUFFER.with(|buf| buf.borrow().clone());
        let texts = $texts;
        let mut text_index = 0;
        for msg in logged_messages.iter() {
            if text_index < texts.len() && msg.contains(texts[text_index]) {
                text_index += 1;
            }
        }
        if text_index != texts.len() {
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
    use lazy_static::lazy_static;

    lazy_static! {
        static ref TEST_LOGGER: () = {
            initialize().expect("Failed to initialize logger");
        };
    }

    #[test]
    fn test_log_capture() {
        let _ = &*TEST_LOGGER; // Ensure logger is initialized
        let _guard = start_capture();
        info!("Hello from test!");
        info!("Yet Again!");
        assert_in_logs!(["Hello from test!", "Yet Again!"]);
    }

    #[test]
    fn test_log_isolation() {
        let _ = &*TEST_LOGGER;
        let _guard = start_capture();
        info!("Test 2 log");
        assert_in_logs!(["Test 2 log"]);
    }

}