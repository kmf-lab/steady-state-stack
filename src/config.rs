    #[cfg(feature = "telemetry_server")]
    pub const TELEMETRY_SERVER: bool = true;
    #[cfg(not(feature = "telemetry_server"))]
    pub const TELEMETRY_SERVER: bool = false;

    #[cfg(feature = "telemetry_history")]
    pub const TELEMETRY_HISTORY: bool = true;
    #[cfg(not(feature = "telemetry_history"))]
    pub const TELEMETRY_HISTORY: bool = false;


    pub const TELEMETRY_SERVER_PORT:u16 = 8080; // need to move to env?
    pub const TELEMETRY_SERVER_IP:&str = "127.0.01"; // need to move to env?

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

