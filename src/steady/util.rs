
use flexi_logger::{Logger, LogSpecification};
use log::*;

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
        let log_spec = LogSpecification::env_or_parse(level)?;

        Logger::with(log_spec)
            .format(flexi_logger::colored_with_thread)
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
