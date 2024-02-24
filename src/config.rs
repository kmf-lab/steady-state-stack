use std::env;

#[cfg(feature = "telemetry_server")]
    pub const TELEMETRY_SERVER: bool = !std::env::var("DOCS_RS").is_ok();
    #[cfg(not(feature = "telemetry_server"))]
    pub const TELEMETRY_SERVER: bool = false;

    #[cfg(feature = "telemetry_history")]
    pub const TELEMETRY_HISTORY: bool = !std::env::var("DOCS_RS").is_ok();
    #[cfg(not(feature = "telemetry_history"))]
    pub const TELEMETRY_HISTORY: bool = false;

    //only set true if you want supervisors to restart actors while debug symbols are on
    //you normally want this false so you can debug the actor that failed while developing
    //supervisors will always restart actors for production release, no need to change this
    #[cfg(not(feature = "restart_actors_when_debugging"))]
    pub const DISABLE_DEBUG_FAIL_FAST:bool = false;
    #[cfg(feature = "restart_actors_when_debugging")]
    pub const DISABLE_DEBUG_FAIL_FAST:bool = true;

    const DEFAULT_TELEMETRY_SERVER_PORT:&str = "8080";
    pub(crate) fn telemetry_server_port() -> u16 {
        env::var("TELEMETRY_SERVER_PORT")
            .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_PORT.to_string())
            .parse::<u16>()
            .expect("TELEMETRY_SERVER_PORT must be a valid u16")
    }

    pub const DEFAULT_TELEMETRY_SERVER_IP:&str = "127.0.01"; // need to move to env?
    pub(crate) fn telemetry_server_ip() -> String {
        env::var("TELEMETRY_SERVER_IP")
            .unwrap_or_else(|_| DEFAULT_TELEMETRY_SERVER_IP.to_string())
}



    pub const TELEMETRY_FOR_ACTORS:bool = true; //TODO: can turn this off as a feature


    pub const REAL_CHANNEL_LENGTH_TO_FEATURE:usize = 256; //allows features to fall behind with minimal latency
    pub const REAL_CHANNEL_LENGTH_TO_COLLECTOR:usize = 256; //larger values take up memory but allow faster capture rates
    pub const LOCKED_CHANNEL_LENGTH_TO_COLLECTOR:usize = REAL_CHANNEL_LENGTH_TO_COLLECTOR>>1; //larger values take up memory but allow faster capture rates

    pub const TELEMETRY_PRODUCTION_RATE_MS:usize = 32;
                 //values smaller than 32 can not be seen my normal humans
                 //values larger than 1000 are not supported at this time

    pub const MIN_TELEMETRY_CAPTURE_RATE_MICRO_SECS:usize
               = (1000*TELEMETRY_PRODUCTION_RATE_MS)/LOCKED_CHANNEL_LENGTH_TO_COLLECTOR;


    pub const MAX_TELEMETRY_ERROR_RATE_SECONDS: usize = 60;


    pub const SHOW_TELEMETRY_ON_TELEMETRY:bool = false;


