
use std::str::FromStr;
use flexi_logger::{DeferredNow, Logger, LogSpecBuilder};
use flexi_logger::filter::{LogLineFilter, LogLineWriter};

use log::*;
use nuclei::Handle;

// new Proactive IO
use nuclei::*;
use futures::io::SeekFrom;
use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use std::fs::{create_dir_all, File, OpenOptions};

pub(crate) async fn all_to_file_async(file:File, data: &[u8]) -> Result<(), std::io::Error> {
    Handle::<File>::new(file)?.write_all(data).await?;
    Result::<(), std::io::Error>::Ok(())
}


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
    /// # Examples
    /// ```
    /// let log_level = "info"; // Typically obtained from command line arguments or env vars
    /// if let Err(e) = init_logging(log_level) {
    ///     eprintln!("Logger initialization failed: {:?}", e);
    ///     // Handle error appropriately (e.g., exit the program or fallback to a default configuration)
    /// }
    /// ```
    pub fn steady_logging_init(level: &str) -> Result<(), Box<dyn std::error::Error>> {

        let mut builder = LogSpecBuilder::new();
        builder.default(LevelFilter::from_str(level)?); // Set the default level
        let log_spec = builder.build();

        Logger::with(log_spec)
            .format(flexi_logger::colored_with_thread)
            // covert the tide info messages to trace
            .filter(Box::new(TideHide))
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

pub struct TideHide;
impl LogLineFilter for TideHide {
    fn write(&self, now: &mut DeferredNow
             , record: &log::Record
             , log_line_writer: &dyn LogLineWriter ) -> std::io::Result<()> {

        //NOTE: this is a hack to hide the tide info messages, revisit in 2025
        //NOTE: if this filter stops working perhaps Tide has
        //    fixed its code and we can use the proper way of filtering this.
        if Level::Info == record.level()
            //avoid more work if it can be  helped
           && (Some(87) == record.line() || Some(43) == record.line())
           && (Some("tide::log::middleware") == record.module_path()
            || Some("tide::fs::serve_dir") == record.module_path()) {
            if log_enabled!(Level::Trace) {
                //rewrite the log record to be at trace level
                let record = Record::builder()
                    .args(*record.args())
                    .level(Level::Trace)
                    .target(record.target())
                    .file(record.file())
                    .line(record.line())
                    .module_path(record.module_path())
                    .build();
                log_line_writer.write(now, &record)?;
            }
        } else {
            log_line_writer.write(now, record)?;
        }
        Ok(())
    }
}
////////////////////////////////////////////
////////////////////////////////////////////

////////////////////////////////////////////
////////////////////////////////////////////
#[cfg(test)]
pub(crate) mod util_tests {
    use lazy_static::lazy_static;
    use std::sync::Once;
    use crate::steady;
    lazy_static! {
            static ref INIT: Once = Once::new();
    }

    pub(crate) fn initialize_logger() {
        INIT.call_once(|| {
            if let Err(e) = steady::util::steady_logging_init("info") {
                print!("Warning: Logger initialization failed with {:?}. There will be no logging.", e);
            }
        });
    }

}
