
use std::str::FromStr;
use flexi_logger::{Logger, LogSpecBuilder, WriteMode};

use log::*;

// new Proactive IO
#[allow(unused_imports)]
use nuclei::*;
#[allow(unused_imports)]
use futures::io::SeekFrom;
#[allow(unused_imports)]
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
#[allow(unused_imports)]
use std::fs::{create_dir_all, File, OpenOptions};

/*
fn lock_if_some<'a, T: std::marker::Send + 'a>(opt_lock: &'a Option<Arc<Mutex<T>>>)
                                               -> Pin<Box<dyn Future<Output = Option<MutexGuard<'a, T>>> + Send + 'a>> {
    Box::pin(async move {
        match opt_lock {
            Some(lock) => Some(lock.lock().await),
            None => None,
        }
    })
}
*/


    /// Initializes logging for the application using the provided log level.
    ///
    /// This function sets up the logger based on the specified log level, which can be adjusted through
    /// command line arguments or environment variables. It's designed to be used both in the main
    /// application and in test cases. The function demonstrates the use of traditional Rust error
    /// propagation with the `?` operator. Note that actors do not initialize logging as it is done
    /// for them in `main` before they are started.
    ///
    /// # Parameters
    /// * `level`: A string slice (`&str`) that specifies the desired log level. The log level can be
    ///            dynamically set via environment variables or directly passed as an argument.
    ///
    /// # Returns
    /// This function returns a `Result<(), Box<dyn std::error::Error>>`. On successful initialization
    /// of the logger, it returns `Ok(())`. If an error occurs during initialization, it returns an
    /// `Err` with the error wrapped in a `Box<dyn std::error::Error>`.
    ///
    /// # Errors
    /// This function will return an error if the logger initialization fails for any reason, such as
    /// an invalid log level string or issues with logger setup.
    ///
    /// # Security Considerations
    /// Be cautious to never log any personally identifiable information (PII) or credentials. It is
    /// the responsibility of the developer to ensure that sensitive data is not exposed in the logs.
    ///
    pub fn steady_logging_init(level: &str) -> Result<(), Box<dyn std::error::Error>> {

        let mut builder = LogSpecBuilder::new();
        builder.default(LevelFilter::from_str(level)?); // Set the default level
        let log_spec = builder.build();

        Logger::with(log_spec)
            .format(flexi_logger::colored_with_thread)
            .write_mode(WriteMode::Direct)
            .start()?;

        /////////////////////////////////////////////////////////////////////
        // for all log levels use caution and never write any personal identifiable data
        // to the logs. YOU are always responsible to ensure credentials are never logged.
        ////////////////////////////////////////////////////////////////////
        if log::log_enabled!(log::Level::Trace) || log::log_enabled!(log::Level::Debug)  {

            trace!("Trace: deep application tracing");
            //Rationale: "Trace" level is typically used for detailed debugging information, often in a context where the flow through the system is being traced.

            debug!("Debug: complex part analysis");
            //Rationale: "Debug" is used for information useful in a debugging context, less detailed than trace, but more so than higher levels.

            warn!("Warn: recoverable issue, needs attention");
            //Rationale: Warnings indicate something unexpected but not necessarily fatal; it's a signal that something should be looked at but isn't an immediate failure.

            error!("Error: unexpected issue encountered");
            //Rationale: Errors signify serious issues, typically ones that are unexpected and may disrupt normal operation but are not application-wide failures.

            info!("Info: key event occurred");
            //Rationale: Info messages are for general, important but not urgent information. They should convey key events or state changes in the application.
        }
        Ok(())
    }

////////////////////////////////////////////
////////////////////////////////////////////


/// Logger module.
///
/// The `logger` module provides functionality for initializing the logging system used within
/// the Steady State framework. It ensures that the logging system is initialized only once
/// during the runtime of the application.
pub mod logger {
    use lazy_static::lazy_static;
    use parking_lot::Once;
    use crate::util;

    lazy_static! {
        static ref INIT: Once = Once::new();
    }

    /// Initializes the logger.
    ///
    /// This function initializes the logging system for the Steady State framework. It ensures
    /// that the logging system is initialized only once, even if this function is called multiple times.
    /// If the initialization fails, a warning message is printed.
    ///
    pub fn initialize() {
        INIT.call_once(|| {
            if let Err(e) = util::steady_logging_init("info") {
                print!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
            }
        });
    }
}
