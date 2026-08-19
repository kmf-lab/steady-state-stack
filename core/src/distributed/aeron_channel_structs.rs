// ss[related distributed.aeron-uri]
use std::ffi::CString;
// ss[related philosophy.structural-hierarchy]
use std::fmt::Debug;
// ss[related philosophy.structural-hierarchy]
use std::net::{IpAddr};

// ss[related distributed.aeron-uri]
pub use aeron_utils::{
    media_driver_probe, media_driver_probe_default, media_driver_probe_with_reason,
    MediaDriverProbeError,
};

/// Aeron media driver connectivity helpers (CNC probe, context retry).
pub mod aeron_utils {
    // ss[related distributed.aeron-uri]
    use std::sync::Arc;
    // ss[related philosophy.structural-hierarchy]
    use futures_util::lock::Mutex;
    // ss[related philosophy.structural-hierarchy]
    use aeron::aeron::Aeron;
    // ss[related distributed.aeron-uri]
    use aeron::context::Context;
    // ss[related philosophy.structural-hierarchy]
    use aeron::utils::errors::AeronError;
    // ss[related philosophy.structural-hierarchy]
    use log::*;
    // ss[related distributed.aeron-uri]
    use std::time::Instant;
    // ss[related philosophy.structural-hierarchy]
    use std::fs::File;
    // ss[related philosophy.structural-hierarchy]
    use std::io::{Read, Seek, SeekFrom};
    // ss[related distributed.aeron-uri]
    use std::time::{Duration};
    // ss[related philosophy.structural-hierarchy]
    use std::path::Path;

    /// Handles Aeron errors by logging them at the warn level.
    ///
    /// # Arguments
    /// - `error`: The `AeronError` encountered during operation.
    // ss[related distributed.aeron-uri]
    fn error_handler(error: AeronError) {
        error!("Aeron Error: {:?}", error);
    }



    /// if this is still zero then the driver is not ready for use
    // ss[related distributed.aeron-uri]
    fn is_cnc_version_marker_set<P: AsRef<Path>>(cnc_path: P) -> bool {
        // The version marker is a 32-bit int at offset 0
        // ss[related philosophy.structural-hierarchy]
        const VERSION_OFFSET: u64 = 0;
        let mut file = match File::open(&cnc_path) {
            Ok(f) => f,
            Err(_) => return false,
        };

        if file.seek(SeekFrom::Start(VERSION_OFFSET)).is_err() {
            return false;
        }

        let mut buf = [0u8; 4];
        if file.read_exact(&mut buf).is_err() {
            return false;
        }

        let version = u32::from_le_bytes(buf);
        version != 0
    }

    // ss[related distributed.aeron-uri]
    pub(crate) fn aeron_context_with_retry(
        mut aeron_context: Context,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> Option<Arc<Mutex<Aeron>>> {
        // Existing setup (error handler, directories, etc.)
        aeron_context.set_error_handler(Box::new(error_handler));
        aeron_context.set_pre_touch_mapped_memory(true);

        #[cfg(not(windows))]
        aeron_context.set_aeron_dir("/dev/shm/aeron-default".parse().unwrap());
        #[cfg(windows)]
        aeron_context.set_aeron_dir("C:\\Temp\\aeron".parse().unwrap());

        let start = Instant::now();
        let mut last_failure: String = String::from("CNC did not become ready");
        loop {
            if start.elapsed() >= max_wait {
                warn!(
                    "Aeron context unavailable after {:?}: {}. Is the Aeron media driver running? (check CNC / aeron dir, e.g. /dev/shm/aeron-default on Linux)",
                    max_wait, last_failure
                );
                return None;
            }
            match Aeron::new(aeron_context.clone()) {
                Ok(aeron) => {
                    let cnc_path = PathBuf::from(aeron.context().cnc_file_name());
                    if is_cnc_stable(&aeron, Duration::from_millis(300)) {
                        if is_cnc_version_marker_set(&cnc_path) {
                            trace!(
                                    "Aeron context created successfully. CNC file stable and version marker set. Client ID: {:?}",
                                    aeron.client_id()
                                );
                            return Some(Arc::new(Mutex::new(aeron)));
                        } else {
                            last_failure =
                                "CNC file version marker not set (driver still starting?)".to_string();
                            debug!("{}. Retrying in {:?}...", last_failure, retry_interval);
                        }
                    } else {
                        last_failure = "CNC file unstable (still changing)".to_string();
                        debug!("{}. Retrying in {:?}...", last_failure, retry_interval);
                    }
                    std::thread::sleep(retry_interval);
                }
                Err(e) => {
                    last_failure = format!("{:?}", e);
                    debug!(
                        "Failed to create Aeron context: {:?}. Retrying in {:?}...",
                        e, retry_interval
                    );
                    std::thread::sleep(retry_interval);
                }
            }
        }
    }

    // ss[related distributed.aeron-uri]
    use std::path::PathBuf;

    /// Checks if the CNC file's modification time has stabilized for `stabilization_period`.
    // ss[related distributed.aeron-uri]
    fn is_cnc_stable(aeron: &Aeron, stabilization_period: Duration) -> bool {
        // ss[related philosophy.structural-hierarchy]
        use std::fs;
        // Convert String to PathBuf correctly
        let cnc_path = PathBuf::from(aeron.context().cnc_file_name());

        // Get initial metadata and modification time
        let initial_metadata = match fs::metadata(&cnc_path) {
            Ok(meta) => meta,
            Err(_) => return false,
        };
        let initial_mtime = match initial_metadata.modified() {
            Ok(mtime) => mtime,
            Err(_) => return false,
        };

        // Wait for the stabilization period
        std::thread::sleep(stabilization_period);

        // Get new metadata and modification time
        let new_metadata = match fs::metadata(&cnc_path) {
            Ok(meta) => meta,
            Err(_) => return false,
        };
        let new_mtime = match new_metadata.modified() {
            Ok(mtime) => mtime,
            Err(_) => return false,
        };

        // Check if the modification time has remained the same
        new_mtime == initial_mtime
    }

    // ss[related distributed.aeron-uri]
    fn default_aeron_dir() -> String {
        #[cfg(not(windows))]
        {
            "/dev/shm/aeron-default".to_string()
        }
        #[cfg(windows)]
        {
            "C:\\Temp\\aeron".to_string()
        }
    }

    /// Detailed reason when the media driver is not reachable.
    #[derive(Debug, Clone)]
    // ss[related distributed.aeron-uri]
    pub struct MediaDriverProbeError {
        /// Last CNC or context creation failure observed during probing.
        pub last_failure: String,
        /// Wall time spent probing before giving up.
        pub elapsed: Duration,
        /// Aeron directory path used for the probe (e.g. `/dev/shm/aeron-default`).
        pub aeron_dir: String,
    }

    // ss[related distributed.aeron-uri]
    impl MediaDriverProbeError {
        /// Operator-facing hint for starting or fixing the media driver.
        // ss[related philosophy.structural-hierarchy]
        pub fn hint(&self) -> String {
            format!(
                "Is aeronmd running? CNC/aeron dir: {}. Last error: {}. \
                 Install: core/routing_service/aeron/README_linux.md. \
                 Set SS_AERON_REQUIRED=1 to fail tests instead of skipping when the driver is down.",
                self.aeron_dir, self.last_failure
            )
        }
    }

    // ss[related distributed.aeron-uri]
    impl std::fmt::Display for MediaDriverProbeError {
        // ss[related philosophy.structural-hierarchy]
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Aeron media driver not available after {:?}: {} (dir: {})",
                self.elapsed, self.last_failure, self.aeron_dir
            )
        }
    }

    // ss[related distributed.aeron-uri]
    impl std::error::Error for MediaDriverProbeError {}

    /// Probes for a running media driver; returns a detailed error when unavailable.
    // ss[impl distributed.media-driver-testing]
    pub fn media_driver_probe_with_reason(max_wait: Duration) -> Result<(), MediaDriverProbeError> {
        let aeron_dir = default_aeron_dir();
        let mut aeron_context = Context::new();
        aeron_context.set_error_handler(Box::new(error_handler));
        aeron_context.set_pre_touch_mapped_memory(true);
        #[cfg(not(windows))]
        let _ = aeron_context.set_aeron_dir(aeron_dir.parse().unwrap());
        #[cfg(windows)]
        let _ = aeron_context.set_aeron_dir(aeron_dir.parse().unwrap());

        let start = Instant::now();
        let retry_interval = Duration::from_millis(100);
        let mut last_failure = String::from("CNC did not become ready");

        loop {
            if start.elapsed() >= max_wait {
                return Err(MediaDriverProbeError {
                    last_failure,
                    elapsed: start.elapsed(),
                    aeron_dir,
                });
            }
            match Aeron::new(aeron_context.clone()) {
                Ok(aeron) => {
                    let cnc_path = PathBuf::from(aeron.context().cnc_file_name());
                    if is_cnc_stable(&aeron, Duration::from_millis(300)) {
                        if is_cnc_version_marker_set(&cnc_path) {
                            return Ok(());
                        }
                        last_failure =
                            "CNC file version marker not set (driver still starting?)".to_string();
                    } else {
                        last_failure = "CNC file unstable (still changing)".to_string();
                    }
                    std::thread::sleep(retry_interval);
                }
                Err(e) => {
                    last_failure = format!("{e:?}");
                    std::thread::sleep(retry_interval);
                }
            }
        }
    }

    /// Returns true when an Aeron media driver is reachable within `max_wait`.
    // ss[related distributed.aeron-uri]
    pub fn media_driver_probe(max_wait: Duration) -> bool {
        media_driver_probe_with_reason(max_wait).is_ok()
    }

    /// Default probe budget (5 seconds) for local integration tests.
    // ss[related distributed.aeron-uri]
    pub fn media_driver_probe_default() -> bool {
        media_driver_probe(Duration::from_secs(5))
    }
}

/// Specifies the type of media transport for an Aeron channel.
///
/// Aeron supports different kinds of communication, depending on the use case.
/// Each type is represented by this enum.
///
/// # Variants
/// - `Udp`: Standard UDP channel for unicast or multicast communication.
/// - `Ipc`: Inter-Process Communication channel for processes on the same machine.
/// - `SpyUdp`: Observes traffic on a UDP channel without sending or receiving.
/// - `SpyIpc`: Observes traffic on an IPC channel without sending or receiving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related distributed.aeron-uri]
pub enum MediaType {
    /// Standard UDP channel: used for point-to-point or multicast communication.
    /// Example: `aeron:udp?endpoint=127.0.0.1:40456`
    Udp,
    /// IPC channel: used for high-speed communication between processes on the same host.
    /// Example: `aeron:ipc`
    Ipc,
    /// Spy on an existing UDP channel: monitors traffic without participating.
    /// Example: `aeron-spy:aeron:udp?endpoint=127.0.0.1:40456`
    SpyUdp,
    /// Spy on an existing IPC channel: monitors IPC traffic without participating.
    /// Example: `aeron-spy:aeron:ipc`
    SpyIpc,
}

/// Specifies how control messages are handled in multicast communication.
///
/// Control messages in multicast are used to coordinate the distribution of data.
/// The mode determines whether control is handled automatically or manually.
///
/// # Variants
/// - `Dynamic`: Control messages are managed automatically by Aeron.
/// - `Manual`: The user must manage control messages manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related distributed.aeron-uri]
pub enum ControlMode {
    /// Control messages are managed automatically by Aeron.
    /// This is the most common mode and is easier to use for most applications.
    Dynamic,
    /// The user must manage control messages manually.
    /// This mode provides more fine-grained control over the multicast setup.
    Manual,
}

/// Represents an endpoint in Aeron communication, consisting of an IP address and port.
///
/// An endpoint is the destination or source of data in a channel. For example, when
/// sending or receiving data, the endpoint specifies the IP address and port where
/// the communication will take place.
///
/// # Fields
/// - `ip`: The IP address of the endpoint.
/// - `port`: The port number associated with the endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related distributed.aeron-uri]
pub struct Endpoint {
    /// The IP address of the endpoint (e.g., `127.0.0.1` or `::1`).
    pub ip: IpAddr,
    /// The port number for communication (e.g., `40456`).
    pub port: u16,
}

/// Represents a network interface for binding UDP traffic.
///
/// The interface is used to specify which network card or IP address to use when
/// sending or receiving UDP traffic. This is particularly useful when a machine
/// has multiple network interfaces.
///
/// # Fields
/// - `ip`: The IP address of the interface.
/// - `port`: The port number for the interface.
#[derive(Debug, Clone, Copy)]
// ss[related distributed.aeron-uri]
pub struct Interface {
    /// The IP address of the network interface (e.g., `192.168.1.1`).
    pub ip: IpAddr,
    /// The port number for the interface. Typically `0` for default binding.
    pub port: u16,
}

/// Configuration for multicast communication, including control messages and Time-to-Live (TTL).
///
/// Multicast is a method of sending data to multiple receivers at once. This struct
/// provides configuration options for multicast channels.
///
/// # Fields
/// - `control`: The control endpoint that manages the multicast session.
/// - `ttl`: The Time-to-Live (TTL) value for multicast packets.
///
/// # Notes on TTL
/// TTL is measured in "hops." Each hop represents a router or device that forwards
/// the multicast packet. A TTL of `0` means the packet stays on the host, while
/// higher values allow it to travel further.
#[derive(Debug, Clone, Copy)]
// ss[related distributed.aeron-uri]
pub struct MulticastConfig {
    /// The control endpoint used to manage the multicast group.
    pub control: Endpoint,
    /// Time-to-Live in hops (e.g., `Some(1)` limits to the local subnet).
    pub ttl: Option<u8>,
}

/// Configuration for a point-to-point communication channel.
///
/// Point-to-point channels can use either unicast UDP or IPC. This struct
/// provides additional configuration options for binding and reliability.
///
/// # Fields
/// - `interface`: An optional network interface for binding.
/// - `reliable`: An optional setting for reliable communication.
#[derive(Debug, Clone, Copy)]
// ss[related distributed.aeron-uri]
pub struct PointServiceConfig {
    /// Optional network interface for binding the channel.
    pub interface: Option<Interface>,
    /// Optional reliability setting (`true` for reliable, `false` for unreliable).
    pub reliable: Option<bool>,
}

/// Specifies the reliability configuration for a channel.
///
/// Reliability determines whether lost packets are retransmitted.
///
/// # Variants
/// - `Reliable`: Ensures reliable communication with retransmissions.
/// - `Unreliable`: Best-effort communication without retransmissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related distributed.aeron-uri]
pub enum ReliableConfig {
    /// Ensures reliable communication. Lost packets are retransmitted.
    Reliable,
    /// Best-effort communication. Packets may be lost if the network drops them.
    Unreliable,
}

/// Represents all forms of Aeron channels.
///
/// Channels define the communication path for data. Aeron supports point-to-point
/// (unicast or IPC) and multicast communication, as well as spy channels for monitoring.
///
/// # Variants
/// - `PointToPoint`: Used for unicast or IPC communication.
/// - `Multicast`: Used for multicast communication.
#[derive(Debug, Clone)]
// ss[related distributed.aeron-uri]
pub enum Channel {
    /// Represents a point-to-point unicast or IPC channel.
    ///
    /// # Fields
    /// - `media_type`: The type of media transport.
    /// - `endpoint`: The target endpoint for communication.
    /// - `interface`: An optional source interface for UDP communication.
    /// - `reliability`: An optional reliability setting.
    /// - `term_length`: An optional term length for the channel.
    PointToPoint {
        /// Specifies the transport type (e.g., `Udp` or `Ipc`).
        media_type: MediaType,
        /// The target endpoint (e.g., `127.0.0.1:40123`).
        endpoint: Endpoint,
        /// Optional source interface for UDP communication.
        interface: Option<Endpoint>,
        /// Optional reliability configuration.
        reliability: Option<ReliableConfig>,
        /// Optional term length for the channel buffer (e.g., `65536`).
        term_length: Option<usize>,
    },
    /// Represents a multicast communication channel.
    ///
    /// # Fields
    /// - `media_type`: The type of media transport.
    /// - `endpoint`: The multicast group endpoint.
    /// - `config`: Configuration for multicast, including control and TTL.
    /// - `control_mode`: How control messages are managed.
    /// - `term_length`: An optional term length for the channel.
    Multicast {
        /// Specifies the transport type (typically `Udp` for multicast).
        media_type: MediaType,
        /// The multicast group endpoint (e.g., `224.0.1.1:40456`).
        endpoint: Endpoint,
        /// Configuration for multicast, including control and TTL.
        config: MulticastConfig,
        /// Specifies how control messages are managed.
        control_mode: ControlMode,
        /// Optional term length for the channel buffer (e.g., `65536`).
        term_length: Option<usize>,
    },
}

// ss[related distributed.aeron-uri]
impl Channel {
    /// Builds a valid Aeron channel string according to official docs.
    ///
    /// Generates a channel URI based on the configuration:
    /// - `Udp`: Produces `aeron:udp?endpoint=host:port`.
    /// - `Ipc`: Produces `aeron:ipc`.
    /// - `SpyUdp`: Produces `aeron-spy:aeron:udp?endpoint=...`.
    /// - `SpyIpc`: Produces `aeron-spy:aeron:ipc`.
    ///
    /// For multicast, additional parameters like `control`, `control-mode`, and `ttl` are appended.
    ///
    /// # Returns
    /// A `CString` representing the Aeron channel URI.
    ///
    /// # Note
    /// For IPv6, the current implementation produces `endpoint=[::1]40123`, which omits the colon
    /// before the port. The correct format should be `endpoint=[::1]:40123`.
    // ss[related distributed.aeron-uri]
    pub fn cstring(&self) -> CString {
        let channel_str = match self {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
                term_length
            } => {
                let mut s = match media_type {
                    MediaType::Udp => {
                        // Construct base UDP URI with endpoint
                        let base = "aeron:udp".to_string();
                        let mut query = format!("?endpoint={}{}", ip_to_string(&endpoint.ip), endpoint.port);

                        // Append interface if provided
                        if let Some(iface) = interface {
                            query.push_str(&format!(
                                "|interface={}{}",
                                ip_to_string(&iface.ip),
                                iface.port
                            ));
                        }

                        // Append reliability setting if provided
                        if let Some(rel_cfg) = reliability {
                            let reliable_str = match rel_cfg {
                                ReliableConfig::Reliable => "true",
                                ReliableConfig::Unreliable => "false",
                            };
                            query.push_str(&format!("|reliable={}", reliable_str));
                        }

                        format!("{}{}", base, query)
                    }
                    MediaType::Ipc => "aeron:ipc".to_string(),
                    MediaType::SpyUdp => {
                        // Construct spy UDP URI with endpoint
                        let base = "aeron-spy:aeron:udp".to_string();
                        let mut query = format!("?endpoint={}{}", ip_to_string(&endpoint.ip), endpoint.port);

                        if let Some(iface) = interface {
                            query.push_str(&format!(
                                "|interface={}{}",
                                ip_to_string(&iface.ip),
                                iface.port
                            ));
                        }

                        if let Some(rel_cfg) = reliability {
                            let reliable_str = match rel_cfg {
                                ReliableConfig::Reliable => "true",
                                ReliableConfig::Unreliable => "false",
                            };
                            query.push_str(&format!("|reliable={}", reliable_str));
                        }

                        format!("{}{}", base, query)
                    }
                    MediaType::SpyIpc => "aeron-spy:aeron:ipc".to_string(),
                };

                // Append term length if specified
                if let Some(term_length) = term_length {
                    s.push_str(&format!("|term-length={}", term_length));
                }

                s
            }
            Channel::Multicast {
                media_type,
                endpoint,
                config,
                control_mode,
                term_length
            } => {
                // Determine prefix and base media type for multicast
                let (prefix, base_media) = match media_type {
                    MediaType::Udp => ("aeron:", "udp"),
                    MediaType::Ipc => ("aeron:", "ipc"), // Allowed but unusual for multicast
                    MediaType::SpyUdp => ("aeron-spy:aeron:", "udp"),
                    MediaType::SpyIpc => ("aeron-spy:aeron:", "ipc"),
                };

                // Construct base multicast URI with endpoint
                let mut s = format!(
                    "{}{}?endpoint={}{}",
                    prefix,
                    base_media,
                    ip_to_string(&endpoint.ip),
                    endpoint.port
                );

                // Append control endpoint
                s.push_str(&format!(
                    "|control={}{}",
                    ip_to_string(&config.control.ip),
                    config.control.port
                ));

                // Append control mode
                let mode_str = match control_mode {
                    ControlMode::Dynamic => "dynamic",
                    ControlMode::Manual => "manual",
                };
                s.push_str(&format!("|control-mode={}", mode_str));

                // Append TTL if specified
                if let Some(ttl_val) = config.ttl {
                    s.push_str(&format!("|ttl={}", ttl_val));
                }

                // Append term length if specified
                if let Some(term_length) = term_length {
                    s.push_str(&format!("|term-length={}", term_length));
                }

                s
            }
        };
        CString::new(channel_str).expect("Failed to create CString from channel string")
    }
}

/// Converts an IP address into a string format suitable for Aeron channel URIs.
///
/// - For IPv4, appends a colon (e.g., `127.0.0.1:`).
/// - For IPv6, encloses the address in brackets (e.g., `[::1]`).
///
/// # Arguments
/// - `ip`: The IP address to convert.
///
/// # Returns
/// A `String` representing the formatted IP address.
// ss[related distributed.aeron-uri]
fn ip_to_string(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ipv4) => format!("{}:", ipv4),
        IpAddr::V6(ipv6) => format!("[{}]", ipv6),
    }
}

#[cfg(test)]
// ss[related distributed.aeron-uri]
mod aeron_channel_structs_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use proptest::prelude::*;
    // ss[related distributed.aeron-uri]
    use std::time::Duration;

    // ss[related philosophy.structural-hierarchy]
    fn uri_from_channel(channel: &Channel) -> String {
        channel.cstring().into_string().expect("cstring")
    }

    /// Extract `key=value` pairs from an Aeron channel URI query segment.
    // ss[related distributed.aeron-uri]
    fn uri_param_pairs(uri: &str) -> Vec<(&str, &str)> {
        let tail = if let Some((_, query)) = uri.split_once('?') {
            query
        } else if let Some(pos) = uri.find('|') {
            &uri[pos + 1..]
        } else {
            return Vec::new();
        };
        tail.split('|')
            .filter_map(|part| part.split_once('='))
            .collect()
    }

    // ss[related distributed.aeron-uri]
    fn uri_param_value(uri: &str, key: &str) -> Option<String> {
        if key == "endpoint" {
            if let Some((_, query)) = uri.split_once('?') {
                if let Some(first) = query.split('|').next() {
                    if let Some((k, v)) = first.split_once('=') {
                        if k == "endpoint" {
                            return Some(v.to_string());
                        }
                    }
                }
            }
        }
        uri_param_pairs(uri)
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    }

    /// Parse the port from an `endpoint=host:port` token (IPv4 or bracketed IPv6).
    // ss[related distributed.aeron-uri]
    fn uri_endpoint_port(uri: &str) -> Option<u16> {
        let endpoint = uri_param_value(uri, "endpoint")?;
        if let Some(bracket_end) = endpoint.find(']') {
            endpoint[bracket_end + 1..]
                .trim_start_matches(':')
                .parse()
                .ok()
        } else {
            endpoint.rsplit(':').next()?.parse().ok()
        }
    }

    // ss[related distributed.aeron-uri]
    fn required_uri_tokens(channel: &Channel) -> Vec<String> {
        match channel {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
                term_length,
            } => {
                let mut tokens = match media_type {
                    MediaType::Udp => vec!["aeron:udp".to_string(), "endpoint=".to_string()],
                    MediaType::Ipc => vec!["aeron:ipc".to_string()],
                    MediaType::SpyUdp => {
                        vec!["aeron-spy:aeron:udp".to_string(), "endpoint=".to_string()]
                    }
                    MediaType::SpyIpc => vec!["aeron-spy:aeron:ipc".to_string()],
                };
                if matches!(media_type, MediaType::Udp | MediaType::SpyUdp) {
                    tokens.push(endpoint.port.to_string());
                    if interface.is_some() {
                        tokens.push("interface=".to_string());
                    }
                    if let Some(rel) = reliability {
                        tokens.push(match rel {
                            ReliableConfig::Reliable => "reliable=true".to_string(),
                            ReliableConfig::Unreliable => "reliable=false".to_string(),
                        });
                    }
                }
                if term_length.is_some() {
                    tokens.push("term-length=".to_string());
                }
                tokens
            }
            Channel::Multicast {
                media_type,
                endpoint,
                config,
                control_mode,
                term_length,
            } => {
                let mut tokens = match media_type {
                    MediaType::Udp => vec![
                        "aeron:udp".to_string(),
                        "endpoint=".to_string(),
                        "control=".to_string(),
                    ],
                    MediaType::Ipc => vec![
                        "aeron:ipc".to_string(),
                        "endpoint=".to_string(),
                        "control=".to_string(),
                    ],
                    MediaType::SpyUdp => vec![
                        "aeron-spy:aeron:udp".to_string(),
                        "endpoint=".to_string(),
                        "control=".to_string(),
                    ],
                    MediaType::SpyIpc => vec![
                        "aeron-spy:aeron:ipc".to_string(),
                        "endpoint=".to_string(),
                        "control=".to_string(),
                    ],
                };
                tokens.push(endpoint.port.to_string());
                tokens.push(config.control.port.to_string());
                tokens.push(match control_mode {
                    ControlMode::Dynamic => "control-mode=dynamic".to_string(),
                    ControlMode::Manual => "control-mode=manual".to_string(),
                });
                if config.ttl.is_some() {
                    tokens.push("ttl=".to_string());
                }
                if term_length.is_some() {
                    tokens.push("term-length=".to_string());
                }
                tokens
            }
        }
    }

    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify distributed.media-driver-testing]
    fn test_media_driver_probe_error_display_and_hint() {
        let err = MediaDriverProbeError {
            last_failure: "CNC did not become ready".to_string(),
            elapsed: Duration::from_secs(2),
            aeron_dir: "/dev/shm/aeron-default".to_string(),
        };
        let display = format!("{err}");
        assert!(display.contains("CNC did not become ready"));
        assert!(display.contains("/dev/shm/aeron-default"));
        let hint = err.hint();
        assert!(hint.contains("aeronmd"));
        assert!(hint.contains("SS_AERON_REQUIRED"));
    }

    #[test]
    // ss[verify distributed.media-driver-testing]
    fn test_media_driver_probe_with_zero_wait_fails_fast() {
        let err = media_driver_probe_with_reason(Duration::ZERO).unwrap_err();
        assert!(!err.last_failure.is_empty());
        assert!(err.elapsed < Duration::from_millis(500));
    }

    #[test]
    // ss[verify distributed.media-driver-testing]
    fn test_media_driver_probe_bool_matches_result() {
        let with_reason = media_driver_probe_with_reason(Duration::ZERO).is_ok();
        let probe = media_driver_probe(Duration::ZERO);
        assert_eq!(probe, with_reason);
    }

    #[test]
    // ss[verify distributed.aeron-uri]
    fn test_channel_cstring_ipv4_udp_with_interface_and_term() {
        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port: 40123,
            },
            interface: Some(Endpoint {
                ip: "192.168.1.1".parse().expect("ip"),
                port: 0,
            }),
            reliability: Some(ReliableConfig::Unreliable),
            term_length: Some(65_536),
        };
        let uri = uri_from_channel(&channel);
        assert!(uri.contains("aeron:udp"));
        assert!(uri.contains("endpoint=127.0.0.1:40123"));
        assert!(uri.contains("interface=192.168.1.1:0"));
        assert!(uri.contains("reliable=false"));
        assert!(uri.contains("term-length=65536"));
    }

    #[test]
    // ss[verify distributed.aeron-uri]
    fn test_channel_cstring_ipv6_udp_endpoint() {
        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint: Endpoint {
                ip: "::1".parse().expect("ip"),
                port: 40456,
            },
            interface: None,
            reliability: None,
            term_length: None,
        };
        let uri = uri_from_channel(&channel);
        assert!(uri.contains("[::1]"));
        assert!(uri.contains("40456"));
    }

    #[test]
    // ss[verify distributed.aeron-uri]
    fn test_channel_cstring_spy_ipc_and_multicast_manual() {
        let spy = Channel::PointToPoint {
            media_type: MediaType::SpyIpc,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port: 0,
            },
            interface: None,
            reliability: None,
            term_length: None,
        };
        assert_eq!(uri_from_channel(&spy), "aeron-spy:aeron:ipc");

        let mcast = Channel::Multicast {
            media_type: MediaType::SpyUdp,
            endpoint: Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: 40456,
            },
            config: MulticastConfig {
                control: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: 40457,
                },
                ttl: Some(4),
            },
            control_mode: ControlMode::Manual,
            term_length: Some(1_048_576),
        };
        let uri = uri_from_channel(&mcast);
        assert!(uri.contains("aeron-spy:aeron:udp"));
        assert!(uri.contains("control-mode=manual"));
        assert!(uri.contains("ttl=4"));
        assert!(uri.contains("term-length=1048576"));
    }

    ss_proptest! {
        /// Property: direct `Channel` enum URIs include media, endpoint port, and optional tokens.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_channel_cstring_p2p_udp_tokens(
            port in crate::proptest_support::aeron_port(),
            reliable in any::<bool>(),
            with_interface in any::<bool>(),
            with_term in any::<bool>(),
            spy in any::<bool>(),
        ) {
            let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
            let channel = Channel::PointToPoint {
                media_type,
                endpoint: Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port,
                },
                interface: if with_interface {
                    Some(Endpoint {
                        ip: "10.0.0.1".parse().expect("ip"),
                        port: 0,
                    })
                } else {
                    None
                },
                reliability: Some(if reliable {
                    ReliableConfig::Reliable
                } else {
                    ReliableConfig::Unreliable
                }),
                term_length: if with_term {
                    Some(65_536)
                } else {
                    None
                },
            };
            let uri = uri_from_channel(&channel);
            let prefix = if spy { "aeron-spy:aeron:udp" } else { "aeron:udp" };
            prop_assert!(uri.starts_with(prefix), "uri: {uri}");
            prop_assert!(uri.contains(&port.to_string()), "uri: {uri}");
            prop_assert!(
                uri.contains(if reliable { "reliable=true" } else { "reliable=false" }),
                "uri: {uri}"
            );
            if with_interface {
                prop_assert!(uri.contains("interface=10.0.0.1:0"), "uri: {uri}");
            }
            if with_term {
                prop_assert!(uri.contains("term-length=65536"), "uri: {uri}");
            }
        }

        /// Property: multicast `Channel` URIs include control endpoint and mode tokens.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_channel_cstring_multicast_tokens(
            data_port in crate::proptest_support::aeron_port(),
            control_port in crate::proptest_support::aeron_port(),
            manual in any::<bool>(),
            with_ttl in any::<bool>(),
            term in proptest::option::of(crate::proptest_support::aeron_term_length()),
        ) {
            let channel = Channel::Multicast {
                media_type: MediaType::Udp,
                endpoint: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: data_port,
                },
                config: MulticastConfig {
                    control: Endpoint {
                        ip: "224.0.1.1".parse().expect("ip"),
                        port: control_port,
                    },
                    ttl: if with_ttl { Some(8) } else { None },
                },
                control_mode: if manual {
                    ControlMode::Manual
                } else {
                    ControlMode::Dynamic
                },
                term_length: term,
            };
            let uri = uri_from_channel(&channel);
            prop_assert!(uri.contains("aeron:udp"), "uri: {uri}");
            prop_assert!(uri.contains(&data_port.to_string()), "uri: {uri}");
            prop_assert!(uri.contains(&control_port.to_string()), "uri: {uri}");
            prop_assert!(
                uri.contains(if manual { "control-mode=manual" } else { "control-mode=dynamic" }),
                "uri: {uri}"
            );
            if with_ttl {
                prop_assert!(uri.contains("ttl=8"), "uri: {uri}");
            }
            if let Some(t) = term {
                prop_assert!(uri.contains(&format!("term-length={t}")), "uri: {uri}");
            }
        }

        /// Property: IPC channel URIs include optional term-length.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_channel_cstring_ipc_term(
            spy in any::<bool>(),
            with_term in any::<bool>(),
            term in proptest::option::of(crate::proptest_support::aeron_term_length()),
        ) {
            let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
            let channel = Channel::PointToPoint {
                media_type,
                endpoint: Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port: 0,
                },
                interface: None,
                reliability: None,
                term_length: if with_term { term } else { None },
            };
            let uri = uri_from_channel(&channel);
            let prefix = if spy { "aeron-spy:aeron:ipc" } else { "aeron:ipc" };
            prop_assert!(uri.starts_with(prefix), "uri: {uri}");
            if let Some(t) = term {
                if with_term {
                    prop_assert!(uri.contains(&format!("term-length={t}")), "uri: {uri}");
                }
            } else if !with_term {
                prop_assert!(!uri.contains("term-length="), "uri: {uri}");
            }
        }

        /// Property: `cstring()` round-trips endpoint port and optional query params through URI parse.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_uri_parse_roundtrip_p2p_udp(
            port in crate::proptest_support::aeron_port(),
            reliable in any::<bool>(),
            with_interface in any::<bool>(),
            with_term in any::<bool>(),
            spy in any::<bool>(),
        ) {
            let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
            let term_length = if with_term { Some(65_536) } else { None };
            let channel = Channel::PointToPoint {
                media_type,
                endpoint: Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port,
                },
                interface: if with_interface {
                    Some(Endpoint {
                        ip: "10.0.0.1".parse().expect("ip"),
                        port: 0,
                    })
                } else {
                    None
                },
                reliability: Some(if reliable {
                    ReliableConfig::Reliable
                } else {
                    ReliableConfig::Unreliable
                }),
                term_length,
            };
            let uri = uri_from_channel(&channel);
            for token in required_uri_tokens(&channel) {
                prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
            }
            prop_assert_eq!(uri_endpoint_port(&uri), Some(port));
            let reliable_param = uri_param_value(&uri, "reliable");
            prop_assert_eq!(
                reliable_param.as_deref(),
                Some(if reliable { "true" } else { "false" })
            );
            if with_interface {
                prop_assert!(uri.contains("interface=10.0.0.1:0"), "uri: {uri}");
            }
            if with_term {
                let term_param = uri_param_value(&uri, "term-length");
                prop_assert_eq!(term_param.as_deref(), Some("65536"));
            }
        }

        /// Property: multicast URI parse round-trips data/control ports and control-mode.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_uri_parse_roundtrip_multicast(
            data_port in crate::proptest_support::aeron_port(),
            control_port in crate::proptest_support::aeron_port(),
            manual in any::<bool>(),
            with_ttl in any::<bool>(),
            spy in any::<bool>(),
            term in proptest::option::of(crate::proptest_support::aeron_term_length()),
        ) {
            let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
            let channel = Channel::Multicast {
                media_type,
                endpoint: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: data_port,
                },
                config: MulticastConfig {
                    control: Endpoint {
                        ip: "224.0.1.1".parse().expect("ip"),
                        port: control_port,
                    },
                    ttl: if with_ttl { Some(8) } else { None },
                },
                control_mode: if manual {
                    ControlMode::Manual
                } else {
                    ControlMode::Dynamic
                },
                term_length: term,
            };
            let uri = uri_from_channel(&channel);
            for token in required_uri_tokens(&channel) {
                prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
            }
            prop_assert_eq!(uri_endpoint_port(&uri), Some(data_port));
            prop_assert!(
                uri.contains(&control_port.to_string()),
                "control port missing from uri: {uri}"
            );
            let mode_parsed = uri_param_value(&uri, "control-mode");
            prop_assert_eq!(
                mode_parsed.as_deref(),
                Some(if manual { "manual" } else { "dynamic" })
            );
            if with_ttl {
                let ttl_parsed = uri_param_value(&uri, "ttl");
                prop_assert_eq!(ttl_parsed.as_deref(), Some("8"));
            }
            if let Some(t) = term {
                let term_expected = t.to_string();
                let term_parsed = uri_param_value(&uri, "term-length");
                prop_assert_eq!(term_parsed.as_deref(), Some(term_expected.as_str()));
            }
        }

        /// Property: IPC and spy-IPC URIs round-trip optional term-length through parse.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_uri_parse_roundtrip_ipc(
            spy in any::<bool>(),
            term in proptest::option::of(crate::proptest_support::aeron_term_length()),
        ) {
            let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
            let channel = Channel::PointToPoint {
                media_type,
                endpoint: Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port: 0,
                },
                interface: None,
                reliability: None,
                term_length: term,
            };
            let uri = uri_from_channel(&channel);
            for token in required_uri_tokens(&channel) {
                prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
            }
            if let Some(t) = term {
                let term_expected = t.to_string();
                let term_parsed = uri_param_value(&uri, "term-length");
                prop_assert_eq!(term_parsed.as_deref(), Some(term_expected.as_str()));
            } else {
                prop_assert!(!uri.contains("term-length="), "uri: {uri}");
            }
        }

        /// Property: IPv6 UDP endpoint port round-trips through URI parse.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_uri_parse_roundtrip_ipv6_udp(
            port in crate::proptest_support::aeron_port(),
            reliable in any::<bool>(),
            spy in any::<bool>(),
        ) {
            let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
            let channel = Channel::PointToPoint {
                media_type,
                endpoint: Endpoint {
                    ip: "::1".parse().expect("ip"),
                    port,
                },
                interface: None,
                reliability: Some(if reliable {
                    ReliableConfig::Reliable
                } else {
                    ReliableConfig::Unreliable
                }),
                term_length: None,
            };
            let uri = uri_from_channel(&channel);
            prop_assert!(uri.contains("[::1]"), "uri: {uri}");
            prop_assert_eq!(uri_endpoint_port(&uri), Some(port));
            for token in required_uri_tokens(&channel) {
                prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
            }
        }

        /// Property: multicast spy-IPC and IPC media URIs round-trip control tokens.
        #[test]
        // ss[verify distributed.aeron-uri]
        // ss[verify verify.process.proptest]
        fn proptest_uri_parse_roundtrip_multicast_ipc_media(
            data_port in crate::proptest_support::aeron_port(),
            control_port in crate::proptest_support::aeron_port(),
            spy in any::<bool>(),
            manual in any::<bool>(),
        ) {
            let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
            let channel = Channel::Multicast {
                media_type,
                endpoint: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: data_port,
                },
                config: MulticastConfig {
                    control: Endpoint {
                        ip: "224.0.1.1".parse().expect("ip"),
                        port: control_port,
                    },
                    ttl: None,
                },
                control_mode: if manual {
                    ControlMode::Manual
                } else {
                    ControlMode::Dynamic
                },
                term_length: None,
            };
            let uri = uri_from_channel(&channel);
            let expected_prefix = if spy { "aeron-spy:aeron:ipc" } else { "aeron:ipc" };
            prop_assert!(uri.contains(expected_prefix), "uri: {uri}");
            prop_assert_eq!(uri_endpoint_port(&uri), Some(data_port));
            prop_assert!(uri.contains(&control_port.to_string()), "uri: {uri}");
            let mode_parsed = uri_param_value(&uri, "control-mode");
            prop_assert_eq!(
                mode_parsed.as_deref(),
                Some(if manual { "manual" } else { "dynamic" })
            );
        }
    }
}
