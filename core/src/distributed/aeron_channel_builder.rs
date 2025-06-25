use std::cmp::PartialEq;
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr};
use crate::distributed::aeron_channel_structs::*;

/// Type alias for stream identifier used in Aeron communication.
///
/// This represents a unique identifier for a stream within an Aeron-based communication channel.
pub type StreamId = i32;

/// Represents a technology specification for distributed communication.
///
/// This enum defines the type of technology used for communication in a distributed system:
/// - `None`: Indicates no specific communication technology is configured.
/// - `Aeron`: Specifies Aeron-based communication with a channel and stream ID.
///
/// # Variants
/// - `None`: No communication technology is configured.
/// - `Aeron(Channel, StreamId)`: Configures Aeron communication.
///   - `Channel`: The Aeron channel configuration (e.g., point-to-point or multicast).
///   - `StreamId`: The unique identifier for the stream within the channel.
#[derive(Debug)]
pub enum AqueTech {
    /// No communication technology is configured. Primarily for testing purposes.
    None,
    /// Configures Aeron communication.
    Aeron(Channel, StreamId),
    // TODO: Add other communication technologies (e.g., MQTT, NATS, etc.).
}

impl AqueTech {
    /// Returns a string representation of the technology type.
    ///
    /// This method provides a simple identifier for the communication technology in use.
    ///
    /// # Returns
    /// - `"none"` if the variant is `None`.
    /// - `"Aeron"` if the variant is `Aeron`.
    pub(crate) fn to_tech(&self) -> &'static str {
        match self {
            AqueTech::None => "none",
            AqueTech::Aeron(_, _) => "Aeron",
        }
    }

    /// Returns the channel configuration as a string for matching purposes.
    ///
    /// This method is useful for identifying or comparing channel configurations.
    ///
    /// # Returns
    /// - `"none"` if the variant is `None`.
    /// - A string representation of the Aeron channel configuration derived from its `CString` if the variant is `Aeron`.
    pub(crate) fn to_match_me(&self) -> String {
        match self {
            AqueTech::None => "none".to_string(),
            AqueTech::Aeron(channel, _) => {
                channel.cstring().into_string().expect("Valid CString conversion")
            }
        }
    }

    /// Returns a list of remote addresses associated with the technology.
    ///
    /// This method provides a placeholder for remote endpoints; actual implementation may vary.
    ///
    /// # Returns
    /// - An empty vector if the variant is `None`.
    /// - A vector containing `"127.0.0.1"` as a placeholder for `Aeron` (intended to be replaced with actual peer IPs).
    ///
    /// # Notes
    /// - The current implementation uses a hardcoded placeholder. Future updates should replace this with dynamic peer discovery (e.g., via MQTT).
    pub(crate) fn to_remotes(&self) -> Vec<String> {
        let mut result = Vec::new();
        if let AqueTech::Aeron(_, _) = self {
            // TODO: Replace with logic to gather actual peer IPs (e.g., via MQTT).
            result.push("127.0.0.1".to_string());
        }
        result
    }
}

/// Internal enum to track the mode of the Aeron channel being configured.
///
/// This enum is used within `AeronConfig` to determine the type of channel being built.
///
/// # Variants
/// - `None`: No mode selected (default state before configuration).
/// - `PointToPoint`: Configured for unicast point-to-point communication.
/// - `Multicast`: Configured for multicast communication.
/// - `Ipc`: Configured for inter-process communication (IPC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AeronMode {
    None,
    PointToPoint,
    Multicast,
    Ipc,
}

/// Builder struct for configuring Aeron channels.
///
/// This struct provides a fluent interface to configure Aeron communication channels, supporting:
/// - `Channel::PointToPoint` for unicast or IPC communication.
/// - `Channel::Multicast` for multicast communication.
///
/// It uses a cloning pattern where each setter method takes `&self`, clones the configuration, updates it, and returns the new instance, ensuring immutability.
///
/// # Fields
/// - `media_type`: The transport type (e.g., `Udp` or `Ipc`).
/// - `term_length`: Optional buffer length for the channel in bytes.
/// - `mode`: The current configuration mode (`AeronMode`).
/// - `endpoint`: The destination endpoint (used in point-to-point and multicast).
/// - `interface`: Optional local interface for point-to-point communication.
/// - `reliability`: Optional reliability setting for point-to-point communication.
/// - `control_endpoint`: Control endpoint for multicast communication.
/// - `control_mode`: Optional control mode for multicast communication.
/// - `ttl`: Optional Time-to-Live value for multicast packets.
#[derive(Debug, Clone)]
pub struct AeronConfig {
    media_type: Option<MediaType>,
    term_length: Option<usize>,
    mode: AeronMode,
    endpoint: Option<Endpoint>,
    interface: Option<Endpoint>,
    reliability: Option<ReliableConfig>,
    control_endpoint: Option<Endpoint>,
    control_mode: Option<ControlMode>,
    ttl: Option<u8>,
}

impl Default for AeronConfig {
    /// Provides a default implementation that creates a new, empty `AeronConfig`.
    fn default() -> Self {
        Self::new()
    }
}

impl AeronConfig {
    /// Creates a new, empty `AeronConfig` instance.
    ///
    /// # Returns
    /// A new `AeronConfig` with all fields initialized to their default values (mostly `None` or `AeronMode::None`).
    ///
    /// # Notes
    /// - Before calling `build`, you must set `media_type` and select a mode using `use_point_to_point`, `use_ipc`, or `use_multicast`.
    pub fn new() -> Self {
        Self {
            media_type: None,
            term_length: None,
            mode: AeronMode::None,
            endpoint: None,
            interface: None,
            reliability: None,
            control_endpoint: None,
            control_mode: None,
            ttl: None,
        }
    }

    // ### Common Configuration Methods ###

    /// Sets the media type for the channel.
    ///
    /// # Parameters
    /// - `media_type`: The transport type (e.g., `MediaType::Udp` or `MediaType::Ipc`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified media type.
    pub fn with_media_type(&self, media_type: MediaType) -> Self {
        let mut clone = self.clone();
        clone.media_type = Some(media_type);
        if media_type == MediaType::Ipc {
            //auto set the mode since we have no othe choice when using Ipc
            clone.mode = AeronMode::Ipc;
        }
        clone
    }

    /// Sets the term buffer length for the channel.
    ///
    /// # Parameters
    /// - `term_length`: The buffer size in bytes (e.g., `1_048_576` for 1 MB).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified term length.
    pub fn with_term_length(&self, term_length: usize) -> Self {
        let mut clone = self.clone();
        clone.term_length = Some(term_length);
        clone
    }

    // ### Point-to-Point Configuration Methods ###

    /// Configures the builder for point-to-point unicast communication.
    ///
    /// # Parameters
    /// - `endpoint`: The destination endpoint (e.g., `Endpoint { ip: "127.0.0.1".parse().unwrap(), port: 40123 }`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance in `PointToPoint` mode with the specified endpoint.
    pub fn use_point_to_point(&self, endpoint: Endpoint) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::PointToPoint;
        clone.endpoint = Some(endpoint);
        clone
    }

    /// Configures the builder for inter-process communication (IPC).
    ///
    /// # Returns
    /// A new `AeronConfig` instance in `Ipc` mode.
    ///
    /// # Notes
    /// - IPC uses a default endpoint of `127.0.0.1:0`.
    pub fn use_ipc(&self) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::Ipc;
        clone
    }

    /// Sets the local interface for point-to-point communication.
    ///
    /// # Parameters
    /// - `interface`: The local endpoint to bind (e.g., `Endpoint { ip: "192.168.1.10".parse().unwrap(), port: 0 }`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified interface.
    ///
    /// # Panics
    /// Panics if the current mode is not `AeronMode::PointToPoint`.
    pub fn with_interface(&self, interface: Endpoint) -> Self {
        if self.mode != AeronMode::PointToPoint {
            panic!("`with_interface` called but not in point-to-point mode.");
        }
        let mut clone = self.clone();
        clone.interface = Some(interface);
        clone
    }

    /// Sets the reliability configuration for point-to-point communication.
    ///
    /// # Parameters
    /// - `reliability`: The reliability setting (e.g., `ReliableConfig::Reliable` or `ReliableConfig::Unreliable`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified reliability.
    ///
    /// # Panics
    /// Panics if the current mode is not `AeronMode::PointToPoint`.
    pub fn with_reliability(&self, reliability: ReliableConfig) -> Self {
        if self.mode != AeronMode::PointToPoint {
            panic!("`with_reliability` called but not in point-to-point mode.");
        }
        let mut clone = self.clone();
        clone.reliability = Some(reliability);
        clone
    }

    // ### Multicast Configuration Methods ###

    /// Configures the builder for multicast communication.
    ///
    /// # Parameters
    /// - `group_endpoint`: The multicast group endpoint (e.g., `Endpoint { ip: "224.0.1.1".parse().unwrap(), port: 40456 }`).
    /// - `control_endpoint`: The control endpoint for multicast (e.g., `Endpoint { ip: "224.0.1.1".parse().unwrap(), port: 40457 }`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance in `Multicast` mode with the specified endpoints.
    pub fn use_multicast(&self, group_endpoint: Endpoint, control_endpoint: Endpoint) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::Multicast;
        clone.endpoint = Some(group_endpoint);
        clone.control_endpoint = Some(control_endpoint);
        clone
    }

    /// Sets the control mode for multicast communication.
    ///
    /// # Parameters
    /// - `control_mode`: The control mode (e.g., `ControlMode::Dynamic` or `ControlMode::Manual`).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified control mode.
    ///
    /// # Panics
    /// Panics if the current mode is not `AeronMode::Multicast`.
    pub fn with_control_mode(&self, control_mode: ControlMode) -> Self {
        if self.mode != AeronMode::Multicast {
            panic!("`with_control_mode` called but not in multicast mode.");
        }
        let mut clone = self.clone();
        clone.control_mode = Some(control_mode);
        clone
    }

    /// Sets the Time-to-Live (TTL) for multicast packets.
    ///
    /// # Parameters
    /// - `ttl`: The TTL value (e.g., `1` for local subnet, `5` for broader reach).
    ///
    /// # Returns
    /// A new `AeronConfig` instance with the specified TTL.
    ///
    /// # Panics
    /// Panics if the current mode is not `AeronMode::Multicast`.
    pub fn with_ttl(&self, ttl: u8) -> Self {
        if self.mode != AeronMode::Multicast {
            panic!("`with_ttl` called but not in multicast mode.");
        }
        let mut clone = self.clone();
        clone.ttl = Some(ttl);
        clone
    }

    // ### Build Method ###

    /// Builds the final `Channel` based on the configured settings.
    ///
    /// Constructs a `Channel` enum variant based on the mode and settings provided.
    ///
    /// # Returns
    /// A `Channel` enum instance, either `PointToPoint` or `Multicast`, configured according to the builder's state.
    ///
    /// # Panics
    /// - If `media_type` is not set.
    /// - If no mode is selected (`AeronMode::None`).
    /// - If required fields are missing (e.g., `endpoint` for `PointToPoint`, or `control_endpoint` for `Multicast`).
    pub fn build(&self) -> Channel {
        let media_type = self.media_type.expect("media_type must be set before build()");

        match self.mode {
            AeronMode::None => {
                panic!("No channel mode selected (point-to-point or multicast).");
            }
            AeronMode::Ipc => {
                Channel::PointToPoint {
                    media_type,
                    endpoint: Endpoint {
                        ip: IpAddr::from(Ipv4Addr::new(127, 0, 0, 1)),
                        port: 0,
                    },
                    interface: self.interface,
                    reliability: self.reliability,
                    term_length: self.term_length,
                }
            }
            AeronMode::PointToPoint => {
                let endpoint = self
                    .endpoint
                    .expect("endpoint must be set for point-to-point mode");
                Channel::PointToPoint {
                    media_type,
                    endpoint,
                    interface: self.interface,
                    reliability: self.reliability,
                    term_length: self.term_length,
                }
            }
            AeronMode::Multicast => {
                let endpoint = self
                    .endpoint
                    .expect("multicast group endpoint must be set");
                let control_ep = self
                    .control_endpoint
                    .expect("control endpoint must be set for multicast");
                Channel::Multicast {
                    media_type,
                    endpoint,
                    config: MulticastConfig {
                        control: control_ep,
                        ttl: self.ttl,
                    },
                    control_mode: self.control_mode.unwrap_or(ControlMode::Dynamic),
                    term_length: self.term_length,
                }
            }
        }
    }
}

/// Unit tests for the `AeronConfig` builder and related functionality.
#[cfg(test)]
mod tests {
    use super::*;

    /// Tests building a point-to-point channel with all optional fields configured.
    #[test]
    fn build_p2p_full_config() {
        let p2p_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Valid IP"),
                port: 40123,
            })
            .with_interface(Endpoint {
                ip: "192.168.1.10".parse().expect("Valid IP"),
                port: 0,
            })
            .with_reliability(ReliableConfig::Reliable)
            .with_term_length(1_048_576)
            .build();

        match p2p_channel {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
                term_length,
            } => {
                assert_eq!(media_type, MediaType::Udp);
                assert_eq!(endpoint.port, 40123);
                assert_eq!(
                    interface,
                    Some(Endpoint {
                        ip: "192.168.1.10".parse().expect("Valid IP"),
                        port: 0
                    })
                );
                assert_eq!(reliability, Some(ReliableConfig::Reliable));
                assert_eq!(term_length, Some(1_048_576));
            }
            _ => panic!("Expected a PointToPoint channel"),
        }
    }

    /// Tests building a minimal point-to-point channel with required fields only.
    #[test]
    fn build_p2p_minimal() {
        let p2p_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("Valid IP"),
                port: 40123,
            })
            .build();

        match p2p_channel {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
                term_length,
            } => {
                assert_eq!(media_type, MediaType::Udp);
                assert_eq!(endpoint.port, 40123);
                assert!(interface.is_none());
                assert!(reliability.is_none());
                assert!(term_length.is_none());
            }
            _ => panic!("Expected a PointToPoint channel"),
        }
    }

    /// Tests building an IPC channel with minimal configuration.
    #[test]
    fn build_ipc() {
        let ipc_channel = AeronConfig::new()
            .with_media_type(MediaType::Ipc)
            .use_ipc()
            .build();

        match ipc_channel {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
                term_length,
            } => {
                assert_eq!(media_type, MediaType::Ipc);
                assert_eq!(endpoint.ip, IpAddr::from(Ipv4Addr::new(127, 0, 0, 1)));
                assert_eq!(endpoint.port, 0);
                assert!(interface.is_none());
                assert!(reliability.is_none());
                assert!(term_length.is_none());
            }
            _ => panic!("Expected a PointToPoint channel for IPC"),
        }
    }

    /// Tests building a multicast channel with all optional fields configured.
    #[test]
    fn build_multicast_full_config() {
        let mcast_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_multicast(
                Endpoint {
                    ip: "224.0.1.1".parse().expect("Valid IP"),
                    port: 40456,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect("Valid IP"),
                    port: 40457,
                },
            )
            .with_control_mode(ControlMode::Manual)
            .with_ttl(5)
            .with_term_length(1_048_576)
            .build();

        match mcast_channel {
            Channel::Multicast {
                media_type,
                endpoint,
                config,
                control_mode,
                term_length,
            } => {
                assert_eq!(media_type, MediaType::Udp);
                assert_eq!(endpoint.port, 40456);
                assert_eq!(config.control.port, 40457);
                assert_eq!(control_mode, ControlMode::Manual);
                assert_eq!(config.ttl, Some(5));
                assert_eq!(term_length, Some(1_048_576));
            }
            _ => panic!("Expected a Multicast channel"),
        }
    }

    /// Tests building a minimal multicast channel with default control mode.
    #[test]
    fn build_multicast_minimal() {
        let mcast_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_multicast(
                Endpoint {
                    ip: "224.0.1.1".parse().expect("Valid IP"),
                    port: 40456,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect("Valid IP"),
                    port: 40457,
                },
            )
            .build();

        match mcast_channel {
            Channel::Multicast {
                media_type,
                endpoint,
                config,
                control_mode,
                term_length,
            } => {
                assert_eq!(media_type, MediaType::Udp);
                assert_eq!(endpoint.port, 40456);
                assert_eq!(config.control.port, 40457);
                assert_eq!(control_mode, ControlMode::Dynamic); // Default value
                assert!(config.ttl.is_none());
                assert!(term_length.is_none());
            }
            _ => panic!("Expected a Multicast channel"),
        }
    }
}