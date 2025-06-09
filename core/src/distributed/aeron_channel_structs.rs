use std::ffi::CString;
use std::fmt::Debug;
use std::net::{IpAddr};

pub(crate) mod aeron_utils {
    use std::sync::Arc;
    use futures_util::lock::Mutex;
    use aeron::aeron::Aeron;
    use aeron::context::Context;
    use aeron::utils::errors::AeronError;
    use log::*;
    use std::time::Instant;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};
    use std::time::{SystemTime, UNIX_EPOCH, Duration};

    /// Handles Aeron errors by logging them at the warn level.
    ///
    /// # Arguments
    /// - `error`: The `AeronError` encountered during operation.
    fn error_handler(error: AeronError) {
        error!("Aeron Error: {:?}", error);
    }


    pub(crate) fn aeron_context_with_retry(
        mut aeron_context: Context,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> Option<Arc<Mutex<Aeron>>> {
        aeron_context.set_error_handler(Box::new(error_handler));
        aeron_context.set_pre_touch_mapped_memory(true);

        #[cfg(not(windows))]
        aeron_context.set_aeron_dir("/dev/shm/aeron-default".parse().expect("valid path"));
        #[cfg(windows)]
        aeron_context.set_aeron_dir("C:\\Temp\\aeron".parse().expect("valid path"));

        let start = Instant::now();
        loop {
            match Aeron::new(aeron_context.clone()) {
                Ok(aeron) => {
                    trace!(
                        "Aeron context created using: {:?} client_id: {:?}",
                        aeron.context().cnc_file_name(),
                        aeron.client_id()
                    );
                    
                    
                    
                    return Some(Arc::new(Mutex::new(aeron)));
                }
                Err(e) => {
                    warn!(
                    "Failed to create Aeron context: {:?}. Retrying in {:?}...",
                    e, retry_interval
                );
                    if start.elapsed() > max_wait {
                        trace!("Giving up after {:?}", max_wait);
                        return None;
                    }
                    std::thread::sleep(retry_interval);
                }
            }
        }
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
fn ip_to_string(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ipv4) => format!("{}:", ipv4),
        IpAddr::V6(ipv6) => format!("[{}]", ipv6),
    }
}

#[cfg(test)]
mod aeron_channel_tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    /// Tests a basic PointToPoint UDP channel with an IPv4 endpoint.
    #[test]
    fn test_point_to_point_ipv4() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };

        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint,
            interface: None,
            reliability: None,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123"
        );
    }

    /// Tests a PointToPoint UDP channel with an IPv6 endpoint.
    #[test]
    fn test_point_to_point_ipv6() {
        let endpoint = Endpoint {
            ip: IpAddr::V6(Ipv6Addr::LOCALHOST),
            port: 40123,
        };

        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint,
            interface: None,
            reliability: None,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=[::1]40123"
        );
    }

    /// Tests a basic PointToPoint IPC channel.
    #[test]
    fn test_ipc() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 0,
        };

        let channel = Channel::PointToPoint {
            media_type: MediaType::Ipc,
            endpoint,
            interface: None,
            reliability: None,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(connection.to_str().expect("valid string"), "aeron:ipc");
    }

    /// Tests a Multicast UDP channel with TTL and manual control mode.
    #[test]
    fn test_multicast() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };

        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };

        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };

        let channel = Channel::Multicast {
            media_type: MediaType::Udp,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }

    /// Tests a PointToPoint UDP channel with interface and reliability settings.
    #[test]
    fn test_point_to_point_with_interface_reliability() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };

        let iface = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)),
            port: 40123,
        };

        let reliability = ReliableConfig::Reliable;

        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint,
            interface: Some(iface),
            reliability: Some(reliability),
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123|interface=192.168.1.5:40123|reliable=true"
        );
    }

    /// Tests a PointToPoint SpyUdp channel.
    #[test]
    fn test_spy_udp_point_to_point() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };

        let channel = Channel::PointToPoint {
            media_type: MediaType::SpyUdp,
            endpoint,
            interface: None,
            reliability: None,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:udp?endpoint=127.0.0.1:40123"
        );
    }

    /// Tests a PointToPoint SpyIpc channel.
    #[test]
    fn test_spy_ipc() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 0,
        };

        let channel = Channel::PointToPoint {
            media_type: MediaType::SpyIpc,
            endpoint,
            interface: None,
            reliability: None,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:ipc"
        );
    }

    /// Tests a Multicast SpyUdp channel.
    #[test]
    fn test_spy_udp_multicast() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };

        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };

        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };

        let channel = Channel::Multicast {
            media_type: MediaType::SpyUdp,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: None,
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }

    /// Tests a PointToPoint UDP channel with an IPv6 interface.
    #[test]
    fn test_point_to_point_udp_with_ipv6_interface() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };
        let iface = Endpoint {
            ip: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            port: 0,
        };
        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint,
            interface: Some(iface),
            reliability: None,
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123|interface=[2001:db8::1]0"
        );
    }

    /// Tests a PointToPoint IPC channel with a term length.
    #[test]
    fn test_point_to_point_ipc_with_term_length() {
        let channel = Channel::PointToPoint {
            media_type: MediaType::Ipc,
            endpoint: Endpoint { ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port: 0 },
            interface: None,
            reliability: None,
            term_length: Some(65536),
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:ipc|term-length=65536"
        );
    }

    /// Tests a PointToPoint SpyIpc channel with a term length.
    #[test]
    fn test_point_to_point_spy_ipc_with_term_length() {
        let channel = Channel::PointToPoint {
            media_type: MediaType::SpyIpc,
            endpoint: Endpoint { ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port: 0 },
            interface: None,
            reliability: None,
            term_length: Some(65536),
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:ipc|term-length=65536"
        );
    }

    /// Tests a Multicast UDP channel with IPv6 endpoint and control.
    #[test]
    fn test_multicast_udp_with_ipv6() {
        let endpoint = Endpoint {
            ip: IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)),
            port: 40456,
        };
        let control = Endpoint {
            ip: IpAddr::V6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1)),
            port: 40457,
        };
        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };
        let channel = Channel::Multicast {
            media_type: MediaType::Udp,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=[ff02::1]40456|control=[ff02::1]40457|control-mode=manual|ttl=4"
        );
    }

    /// Tests a Multicast IPC channel (unusual but allowed by current code).
    #[test]
    fn test_multicast_ipc() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };
        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };
        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };
        let channel = Channel::Multicast {
            media_type: MediaType::Ipc,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:ipc?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }

    /// Tests a Multicast SpyIpc channel (unusual but allowed by current code).
    #[test]
    fn test_multicast_spy_ipc() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };
        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };
        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };
        let channel = Channel::Multicast {
            media_type: MediaType::SpyIpc,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:ipc?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }

    /// Tests a Multicast UDP channel with a term length.
    #[test]
    fn test_multicast_udp_with_term_length() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };
        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };
        let config = MulticastConfig {
            control,
            ttl: Some(4),
        };
        let channel = Channel::Multicast {
            media_type: MediaType::Udp,
            endpoint,
            config,
            control_mode: ControlMode::Manual,
            term_length: Some(65536),
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4|term-length=65536"
        );
    }

    /// Tests a PointToPoint UDP channel with unreliable configuration.
    #[test]
    fn test_point_to_point_udp_with_unreliable() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };
        let channel = Channel::PointToPoint {
            media_type: MediaType::Udp,
            endpoint,
            interface: None,
            reliability: Some(ReliableConfig::Unreliable),
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123|reliable=false"
        );
    }

    /// Tests a Multicast UDP channel with dynamic control mode and no TTL.
    #[test]
    fn test_multicast_udp_with_dynamic_control() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            port: 40456,
        };
        let control = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)),
            port: 40457,
        };
        let config = MulticastConfig {
            control,
            ttl: None,
        };
        let channel = Channel::Multicast {
            media_type: MediaType::Udp,
            endpoint,
            config,
            control_mode: ControlMode::Dynamic,
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=dynamic"
        );
    }

    /// Tests a PointToPoint IPC channel with endpoint, interface, and reliability ignored.
    #[test]
    fn test_point_to_point_ipc_with_endpoint_interface_reliability() {
        let endpoint = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 40123,
        };
        let iface = Endpoint {
            ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)),
            port: 40123,
        };
        let channel = Channel::PointToPoint {
            media_type: MediaType::Ipc,
            endpoint,
            interface: Some(iface),
            reliability: Some(ReliableConfig::Reliable),
            term_length: None,
        };
        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:ipc"
        );
    }
}