use clap::{Parser, ValueEnum};

/// Enum to represent communication type (IPC or UDP)
#[derive(ValueEnum, Debug, PartialEq, Clone)]
pub enum CommType {
    /// Inter-Process Communication
    Ipc,
    /// UDP communication
    Udp,
}

/// Command-line arguments for the application
#[derive(Parser, Debug, PartialEq, Clone)]
pub(crate) struct MainArg {
    /// Rate in microseconds (1000 per ms)
    #[arg(short = 'r', long = "rate", default_value = "20")]
    pub(crate) rate_ms: u64,

    /// Number of beats
    #[arg(short = 'b', long = "beats", default_value = "10000")]
    pub(crate) beats: u64,

    /// Communication type (ipc or udp)
    #[arg(long = "comm-type", default_value = "ipc")]
    pub(crate) comm_type: CommType,

    /// IP address (required if comm-type is udp)
    #[arg(long = "ip", required_if_eq("comm_type", "udp"))]
    pub(crate) ip: Option<String>,

    /// Port number (required if comm-type is udp)
    #[arg(long = "port", required_if_eq("comm_type", "udp"))]
    pub(crate) port: Option<u16>,
}

impl Default for MainArg {
    fn default() -> Self {
        MainArg {
            rate_ms: 20,
            beats: 10000,
            comm_type: CommType::Ipc,
            ip: None,
            port: None,
        }
    }
}