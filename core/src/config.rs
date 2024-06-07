use std::env;

    #[cfg(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin"))]
    pub const TELEMETRY_SERVER: bool = true;
    #[cfg(not(any(feature = "telemetry_server_cdn", feature = "telemetry_server_builtin")))]
    pub const TELEMETRY_SERVER: bool = false;

    #[cfg(feature = "telemetry_history")]
    pub const TELEMETRY_HISTORY: bool = true;
    #[cfg(not(feature = "telemetry_history"))]
    pub const TELEMETRY_HISTORY: bool = false;

    //only set true if you want supervisors to restart actors while debug symbols are on
    //you normally want this false so you can debug the actor that failed while developing
    //supervisors will always restart actors for production release, no need to change this
    #[cfg(not(feature = "restart_actors_when_debugging"))]
    pub const DISABLE_DEBUG_FAIL_FAST:bool = false;
    #[cfg(feature = "restart_actors_when_debugging")]
    pub const DISABLE_DEBUG_FAIL_FAST:bool = true;

//////////////////////////////////////////////////////////

    const DEFAULT_TELEMETRY_SERVER_PORT:&str = "8080"; // env: TELEMETRY_SERVER_PORT
    pub(crate) fn telemetry_server_port() -> u16 {
        env::var("TELEMETRY_SERVER_PORT")
            .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_PORT.to_string())
            .parse::<u16>()
            .expect("TELEMETRY_SERVER_PORT must be a valid u16")
    }

    const DEFAULT_TELEMETRY_SERVER_IP:&str = "0.0.0.0"; // env: TELEMETRY_SERVER_IP
    pub(crate) fn telemetry_server_ip() -> String {
        env::var("TELEMETRY_SERVER_IP")
            .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_IP.to_string())
    }

//////////////////////////////////////////////////////////

    // to avoid filling logs with errors we will only show errors once every MAX_TELEMETRY_ERROR_RATE_SECONDS
    // this only applies to the exact same error on the same channel.
    pub const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 20;



    // granularity of the frames:
    // big values do consume a more memory. This controls the accuracy of the data
    // it could be off my as much as 1/VAL of a frame.
    // ALSO: the first half of the channel will fill at the expected 1/VAL rate
    // where VAL is this REAL length/2.  The latter half will fill more slowly as an
    // exponential backoff.
    pub const REAL_CHANNEL_LENGTH_TO_COLLECTOR:usize = 256;
    pub const CONSUMED_MESSAGES_BY_COLLECTOR:usize = REAL_CHANNEL_LENGTH_TO_COLLECTOR>>1; //larger values take up memory but allow faster capture rates
    pub const REAL_CHANNEL_LENGTH_TO_FEATURE:usize = 128; //allows features to fall behind with minimal latency


