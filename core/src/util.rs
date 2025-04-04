use flexi_logger::writers::*;
use flexi_logger::*;
use log::*;
use std::sync::{Arc, Mutex};
use std::error::Error;
use crate::LogLevel;
use std::io;

struct MemoryWriter {
    logs: Arc<Mutex<Vec<String>>>,
    format: FormatFunction, // Store the format function
}

impl MemoryWriter {
    fn new(format: FormatFunction) -> Self {
        MemoryWriter {
            logs: Arc::new(Mutex::new(Vec::new())),
            format,
        }
    }
}

impl LogWriter for MemoryWriter {
    fn write(&self, now: &mut DeferredNow, record: &Record) -> io::Result<()> {
        // Use a Vec<u8> as a temporary writer
        let mut buffer = Vec::new();
        // Call the format function with the buffer as the writer
        (self.format)(&mut buffer, now, record)?;
        // Convert the buffer to a String
        let formatted = String::from_utf8_lossy(&buffer).to_string();
        // Lock and push to logs
        let mut logs = self.logs.lock().map_err(|_| io::Error::new(
            io::ErrorKind::Other,
            "Failed to lock logs"
        ))?;
        logs.push(formatted);
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        Ok(())
    }

    fn max_log_level(&self) -> LevelFilter {
        LevelFilter::max()
    }
}

fn steady_logging_init(level: LogLevel, test_mode: bool) -> Result<(LoggerHandle, Option<Arc<Mutex<Vec<String>>>>), Box<dyn Error>> {
    let log_spec = LogSpecBuilder::new().default(level.to_level_filter()).build();
    let format = flexi_logger::colored_with_thread;

    if test_mode {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let memory_writer = MemoryWriter::new(format);
        let handle = Logger::with(log_spec)
            .format(format)
            .log_to_stderr() // Primary output to stderr
            .add_writer("memory", Box::new(memory_writer) ) // Capture in memory
            .write_mode(WriteMode::Direct)
            .start()?;
        Ok((handle, Some(logs)))
    } else {
        let handle = Logger::with(log_spec)
            .format(format)
            .log_to_stderr()
            .write_mode(WriteMode::Direct)
            .start()?;
        Ok((handle, None))
    }
}
// Logger module
pub mod steady_logger {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref LOGGER_HANDLE: Mutex<Option<LoggerHandle>> = Mutex::new(None);
        static ref TEST_LOGS: Mutex<Option<Arc<Mutex<Vec<String>>>>> = Mutex::new(None);
    }

    /// Initializes the logger with default "info" level for normal operation.
    pub fn initialize() {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        if logger_handle.is_none() {
            match steady_logging_init(LogLevel::Info, false) {
                Ok((handle, _)) => *logger_handle = Some(handle),
                Err(e) => eprintln!("Warning: Logger initialization failed: {}", e),
            }
        }
    }

    /// Initializes the logger with a specific level for normal operation.
    pub fn initialize_with_level(level: LogLevel) -> Result<(), Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        if logger_handle.is_none() {
            let (handle, _) = steady_logging_init(level, false)?;
            *logger_handle = Some(handle);
            Ok(())
        } else if let Some(handle) = logger_handle.as_ref() {
            handle.set_new_spec(LogSpecBuilder::new().default(level.to_level_filter()).build());
            Ok(())
        } else {
            Err("Logger level change failed".into())
        }
    }

    /// Initializes the logger for test mode with in-memory log capturing.
    pub fn initialize_for_test(level: LogLevel) -> Result<Arc<Mutex<Vec<String>>>, Box<dyn Error>> {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        let mut test_logs = TEST_LOGS.lock().unwrap();

        if logger_handle.is_none() {
            let (handle, logs_opt) = steady_logging_init(level, true)?;
            *logger_handle = Some(handle);
            let logs = logs_opt.ok_or("Test logs not initialized")?;
            *test_logs = Some(logs.clone());
            Ok(logs)
        } else {
            Err("Logger already initialized".into())
        }
    }

    /// Stops capturing logs in memory during a test.
    pub fn stop_capturing_logs() {
        if let Some(logs) = TEST_LOGS.lock().unwrap().as_ref() {
            logs.lock().unwrap().clear();
        }
    }

    /// Restores the logger to normal operation mode with "info" level.
    pub fn restore_logger() {
        let mut logger_handle = LOGGER_HANDLE.lock().unwrap();
        let mut test_logs = TEST_LOGS.lock().unwrap();

        if let Some(handle) = logger_handle.take() {
            drop(handle); // Shutdown current logger
        }
        *test_logs = None;

        match steady_logging_init(LogLevel::Info, false) {
            Ok((handle, _)) => *logger_handle = Some(handle),
            Err(e) => eprintln!("Warning: Logger restoration failed: {}", e),
        }
    }
}

// Assertion macro with restore_logger() just before error!
#[macro_export]
macro_rules! assert_in_logs {
    ($logs:expr, $texts:expr) => {{
        let logged_messages = $logs.lock().unwrap();
        let texts = $texts;
        let mut text_index = 0;
        for msg in logged_messages.iter() {
            if text_index < texts.len() && msg.contains(texts[text_index]) {
                text_index += 1;
            }
        }
        if text_index != texts.len() {
            crate::steady_logger::restore_logger(); // Call just before error!
            error!(
                "Assertion failed: expected texts {:?} in logs {:?} at {}:{}",
                texts, *logged_messages, file!(), line!()
            );
            panic!(
                "Assertion failed at {}:{}: expected texts {:?} in logs {:?}",
                file!(), line!(), texts, *logged_messages
            );
        }
    }};
}