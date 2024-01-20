

    #[cfg(feature = "telemetry_server")]
    pub const TELEMETRY_SERVER: bool = true;
    #[cfg(not(feature = "telemetry_server"))]
    pub const TELEMETRY_SERVER: bool = false;

    #[cfg(feature = "telemetry_history")]
    pub const TELEMETRY_HISTORY: bool = true;
    #[cfg(not(feature = "telemetry_history"))]
    pub const TELEMETRY_HISTORY: bool = false;


    pub const CHANNEL_LENGTH_TO_FEATURE:usize = 4; //allows features to fall behind with some latency
    pub const CHANNEL_LENGTH_TO_COLLECTOR:usize = 16; //larger values take up memory but allow faster capture rates
    pub const TELEMETRY_PRODUCTION_RATE_MS:usize = 32; //values faster than 32 can not be seen my normal humans
    pub const MIN_TELEMETRY_CAPTURE_RATE_MS:usize = TELEMETRY_PRODUCTION_RATE_MS/CHANNEL_LENGTH_TO_COLLECTOR;

