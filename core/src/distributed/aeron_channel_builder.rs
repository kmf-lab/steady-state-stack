use std::cmp::PartialEq;
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use aeron::aeron::Aeron;
use futures_util::lock::Mutex;
use crate::distributed::aeron_channel_structs::*;

/// Type alias for stream identifier used in Aeron communication.
type StreamId = i32;

/// Represents a technology specification for distributed communication.
///
/// This enum defines the type of technology used for communication:
/// - `None`: No technology specified.
/// - `Aeron`: Uses Aeron for communication, with an optional Aeron instance, channel, and stream ID.
///
/// # Variants
/// - `None`: No communication technology configured.
/// - `Aeron(Option<Arc<Mutex<Aeron>>>, Channel, StreamId)`: Aeron-based communication.
///   - First field: Optional Aeron instance wrapped in `Arc<Mutex>` for thread-safe access.
///   - Second field: The configured Aeron channel (e.g., point-to-point or multicast).
///   - Third field: The stream ID for identifying the communication stream.
///
pub enum AqueTech {
    None,
    Aeron(Channel, StreamId),
}

impl AqueTech {
    /// Returns a string representation of the technology type.
    ///
    /// # Returns
    /// - `"none"` if the technology is `None`.
    /// - `"Aeron"` if the technology is `Aeron` with a valid Aeron instance.
    /// - `"Unknown"` otherwise (e.g., `Aeron` without an instance).
    pub(crate) fn to_tech(&self) -> &'static str {
        match self {
            AqueTech::None => "none",
            AqueTech::Aeron(_, _) => "Aeron",
            _ => "Unknown",
        }
    }

    /// Returns the channel configuration as a string for matching purposes.
    ///
    /// # Returns
    /// - `"none"` if the technology is `None`.
    /// - The `CString` representation of the Aeron channel if `Aeron` with an instance.
    /// - `"Unknown"` otherwise.
    pub(crate) fn to_match_me(&self) -> String {
        match self {
            AqueTech::None => "none".to_string(),
            AqueTech::Aeron(channel, _) => {
                channel.cstring().into_string().expect("Valid CString conversion")
            }
            _ => "Unknown".to_string(),
        }
    }

    /// Returns a list of remote addresses associated with the technology.
    ///
    /// # Returns
    /// - An empty vector if not `Aeron` with an instance.
    /// - A vector with `"127.0.0.1"` as a placeholder for `Aeron` (to be replaced with actual peer IPs).
    pub(crate) fn to_remotes(&self) -> Vec<String> {
        let mut result = Vec::new();
        match self {
            AqueTech::Aeron(_, _) => {
                // TODO: Replace with logic to gather actual peer IPs (e.g., via MQTT).
                result.push("127.0.0.1".to_string());
            }
            _ => {}
        }
        result
    }
}

/// Internal enum to track the mode of the Aeron channel being configured.
///
/// Used within `AeronConfig` to determine the type of `Channel` to build.
///
/// # Variants
/// - `None`: No mode selected (default state).
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
/// Supports creating either:
/// - `Channel::PointToPoint` for unicast or IPC communication.
/// - `Channel::Multicast` for multicast communication.
///
/// Uses a cloning pattern: each setter method takes `&self`, clones the config,
/// updates the clone, and returns it to ensure immutability.
///
/// # Fields
/// - `media_type`: Transport type (e.g., `Udp`, `Ipc`).
/// - `term_length`: Optional buffer length for the channel.
/// - `mode`: Current configuration mode (`AeronMode`).
/// - `endpoint`: Destination endpoint (used in both point-to-point and multicast).
/// - `interface`: Optional local interface for point-to-point.
/// - `reliability`: Optional reliability setting for point-to-point.
/// - `control_endpoint`: Control endpoint for multicast.
/// - `control_mode`: Optional control mode for multicast.
/// - `ttl`: Optional Time-to-Live for multicast packets.
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

impl AeronConfig {
    /// Creates a new, empty `AeronConfig`.
    ///
    /// # Notes
    /// - You must set a mode with `use_point_to_point`, `use_ipc`, or `use_multicast`.
    /// - You must set `media_type` before calling `build`.
    ///
    /// # Returns
    /// A new `AeronConfig` with all fields initialized to defaults.
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

    // ### Common Methods ###

    /// Sets the media type for the channel.
    ///
    /// # Parameters
    /// - `media_type`: The transport type (e.g., `Udp`, `Ipc`).
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated media type.
    pub fn with_media_type(&self, media_type: MediaType) -> Self {
        let mut clone = self.clone();
        clone.media_type = Some(media_type);
        clone
    }

    /// Sets the term buffer length for the channel.
    ///
    /// # Parameters
    /// - `term_length`: Buffer size in bytes (e.g., 1_048_576 for 1 MB).
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated term length.
    pub fn with_term_length(&self, term_length: usize) -> Self {
        let mut clone = self.clone();
        clone.term_length = Some(term_length);
        clone
    }

    // ### Point-to-Point Methods ###

    /// Configures the builder for point-to-point communication.
    ///
    /// # Parameters
    /// - `endpoint`: Destination endpoint (e.g., `127.0.0.1:40123`).
    ///
    /// # Returns
    /// A new `AeronConfig` in point-to-point mode with the specified endpoint.
    pub fn use_point_to_point(&self, endpoint: Endpoint) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::PointToPoint;
        clone.endpoint = Some(endpoint);
        clone
    }

    /// Configures the builder for IPC communication.
    ///
    /// # Returns
    /// A new `AeronConfig` in IPC mode.
    pub fn use_ipc(&self) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::Ipc;
        clone
    }

    /// Sets the local interface for point-to-point communication.
    ///
    /// # Parameters
    /// - `interface`: Local endpoint to bind (e.g., `192.168.1.10:0`).
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated interface.
    ///
    /// # Panics
    /// Panics if not in `PointToPoint` mode.
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
    /// - `reliability`: `ReliableConfig::Reliable` or `ReliableConfig::Unreliable`.
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated reliability.
    ///
    /// # Panics
    /// Panics if not in `PointToPoint` mode.
    pub fn with_reliability(&self, reliability: ReliableConfig) -> Self {
        if self.mode != AeronMode::PointToPoint {
            panic!("`with_reliability` called but not in point-to-point mode.");
        }
        let mut clone = self.clone();
        clone.reliability = Some(reliability);
        clone
    }

    // ### Multicast Methods ###

    /// Configures the builder for multicast communication.
    ///
    /// # Parameters
    /// - `group_endpoint`: Multicast group endpoint (e.g., `224.0.1.1:40456`).
    /// - `control_endpoint`: Endpoint for multicast control (e.g., `224.0.1.1:40457`).
    ///
    /// # Returns
    /// A new `AeronConfig` in multicast mode with the specified endpoints.
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
    /// - `control_mode`: `ControlMode::Dynamic` or `ControlMode::Manual`.
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated control mode.
    ///
    /// # Panics
    /// Panics if not in `Multicast` mode.
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
    /// - `ttl`: TTL value (e.g., 1 for local subnet).
    ///
    /// # Returns
    /// A new `AeronConfig` with the updated TTL.
    ///
    /// # Panics
    /// Panics if not in `Multicast` mode.
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
    /// # Returns
    /// A `Channel` enum (`PointToPoint` or `Multicast`) representing the Aeron channel.
    ///
    /// # Panics
    /// - If `media_type` is not set.
    /// - If no mode is selected (`AeronMode::None`).
    /// - If required fields are missing (e.g., `endpoint` for point-to-point).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests building a point-to-point channel with all optional fields set.
    #[test]
    fn build_p2p_full_config() {
        let p2p_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect(""),
                port: 40123,
            })
            .with_interface(Endpoint {
                ip: "192.168.1.10".parse().expect(""),
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
                        ip: "192.168.1.10".parse().expect(""),
                        port: 0
                    })
                );
                assert_eq!(reliability, Some(ReliableConfig::Reliable));
                assert_eq!(term_length, Some(1_048_576));
            }
            _ => panic!("Expected a PointToPoint channel"),
        }
    }

    /// Tests building a minimal point-to-point channel.
    #[test]
    fn build_p2p_minimal() {
        let p2p_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect(""),
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

    /// Tests building an IPC channel.
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

    /// Tests building a multicast channel with all optional fields set.
    #[test]
    fn build_multicast_full_config() {
        let mcast_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_multicast(
                Endpoint {
                    ip: "224.0.1.1".parse().expect(""),
                    port: 40456,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect(""),
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
                    ip: "224.0.1.1".parse().expect(""),
                    port: 40456,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect(""),
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