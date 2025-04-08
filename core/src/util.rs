use flexi_logger::writers::*;
use flexi_logger::*;
use std::sync::{Arc, Mutex};
use std::error::Error;
use crate::LogLevel;
use std::io;
use std::io::Write;  // Ensure we have this for potential write() usage
use log::Record;
use std::cell::RefCell;

/// A simple memory writer that stores all log messages (after formatting) in a thread-local `Vec<String>`.
struct MemoryWriter {
    format: FormatFunction, // Store the format function
}

impl MemoryWriter {
    fn new(format: FormatFunction) -> Self {
        MemoryWriter { format }
    }
}

thread_local! {
    //one buffer per thread ensures each actor can test independently without cross mixing text
    pub static LOG_BUFFER: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

impl LogWriter for MemoryWriter {
    fn write(&self, now: &mut DeferredNow, record: &Record) -> io::Result<()> {
        let mut buffer = Vec::new();
        (self.format)(&mut buffer, now, record)?;
        let formatted = String::from_utf8_lossy(&buffer).to_string();
        LOG_BUFFER.with(|buf| {
            buf.borrow_mut().push(formatted);
        });
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    fn max_log_level(&self) -> LevelFilter {
        LevelFilter::max()
    }
}

/// Core helper that initializes FlexiLogger with either stderr logging or
/// combined stderr + in-memory logging (for test).
fn steady_logging_init(
    level: LogLevel,
    test_mode: bool,
) -> Result<LoggerHandle, Box<dyn Error>> {

    //eprintln!("steady_logging_init {} ----------------------------------------",test_mode);

    let log_spec = LogSpecBuilder::new()
        .default(level.to_level_filter())
        .build();

    let format = flexi_logger::colored_with_thread;

    let mut logger = Logger::with(log_spec);
    if test_mode {
        //eprintln!("test mode enabled -----------------------------------------");
        logger = logger.log_to_writer(Box::new(MemoryWriter::new(format)))
                       .duplicate_to_stderr(flexi_logger::Duplicate::All);
    } else {
        logger = logger.log_to_stderr();
    }
    let mut logger = logger.format(format)
                            .write_mode(WriteMode::Direct);
    logger.start().map_err(|e| Box::new(e) as Box<dyn Error>)
}

// Logger module with all the public-facing initialization and teardown functions.
pub mod steady_logger {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        /// Holds the global logger handle, if any.
        static ref LOGGER_HANDLE: Mutex<Option<LoggerHandle>> = Mutex::new(None);
    }

    /// Initializes the logger with default "info" level for normal operation.
    /// Does nothing if a logger is already set.
    pub fn initialize() {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        if logger_handle.is_none() {
            match super::steady_logging_init(LogLevel::Info, false) {
                Ok(handle) => *logger_handle = Some(handle),
                Err(e) => eprintln!("Warning: Logger initialization failed: {}", e),
            }
        }
    }
    /// Initializes the logger for test mode with in-memory log capturing.
    /// If the logger is already set, it does nothing.
    /// This function has been significantly changed to not return the log buffer.
    pub fn initialize_for_test(level: LogLevel) -> Result<(), Box<dyn Error>> {
        //eprintln!("initialize_for_test start -----------------------------------------");
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        if logger_handle.is_none() {
            let handle = super::steady_logging_init(level, true)?;
            *logger_handle = Some(handle);
        }
        //eprintln!("initialize_for_test stop-----------------------------------------");
        Ok(())
    }

    /// Initializes the logger with a specific level for normal operation.
    /// If a logger is already running, it changes the level dynamically.
    pub fn initialize_with_level(level: LogLevel) -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        if logger_handle.is_none() {
            let handle = super::steady_logging_init(level, false)?;
            *logger_handle = Some(handle);
            Ok(())
        } else if let Some(handle) = logger_handle.as_ref() {
            // Dynamically adjust the log level
            handle.set_new_spec(
                LogSpecBuilder::new()
                    .default(level.to_level_filter())
                    .build()
            );
            Ok(())
        } else {
            Err("Logger level change failed".into())
        }
    }



    /// Clears the thread-local log buffer for the current test.
    pub fn clear_test_logs() {
        LOG_BUFFER.with(|buf| {
            buf.borrow_mut().clear();
        });
    }

    /// Stops capturing logs in memory during a test by clearing out the logs buffer.
    /// This is an alias for clear_test_logs for consistency with existing API.
    pub fn stop_capturing_logs() {
        clear_test_logs();
    }

}

/// Assertion macro that checks if all specified texts appear in the logs.
/// It accesses the thread-local log buffer directly.
#[macro_export]
macro_rules! assert_in_logs {
    ($texts:expr) => {{
        let logged_messages = LOG_BUFFER.with(|buf| buf.borrow().clone());
        let texts = $texts;
        let mut text_index = 0;
        //eprintln!("----------------------------------------");
        for msg in logged_messages.iter() {
            //eprintln!("{:?}", msg);
            if text_index < texts.len() && msg.contains(texts[text_index]) {
                text_index += 1;
            }
        }
        //eprintln!("----------------------------------------");

        if text_index != texts.len() {
            eprintln!(
                "Assertion failed: expected texts {:?} in logs {:?} at {}:{}",
                texts, logged_messages, file!(), line!()
            );
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
    use steady_logger::*; //typical import for captured log tests
    use log::info;

    #[test]
    fn test_assert_in_logs_macro() {
        initialize_for_test(LogLevel::Info).expect("Failed to initialize test logger");
        clear_test_logs();
        eprintln!("=====================================");

        info!("Hello from test!");
        info!("Yet Again!");

        assert_in_logs!(["Hello from test!","Yet Again!"]);
    }

    #[test]
    #[should_panic(expected = "Assertion failed at")]
    fn test_assert_in_logs_macro_failure() {
        initialize_for_test(LogLevel::Info).expect("Failed to initialize test logger");
        clear_test_logs();
        assert_in_logs!(["This text does not exist in logs"]);
    }
}