//! This module provides builder-pattern functionality for configuring various
//! forms of Aeron-based distributed communication. It includes support for
//! point-to-point (unicast or IPC) and multicast, with optional reliability
//! settings, TTL (time-to-live) for multicast, and more.

use std::net::IpAddr;
use crate::distributed::aeron_channel::*;

/// Represents different distributed technologies that can be configured and built.
///
/// Currently, it supports Aeron, and can be extended to include technologies like ZeroMQ.
/// This enum, once fully built, carries all the configuration necessary for distributed
/// communication.
///
/// # Variants
/// - `None`: Indicates that no distributed communication is configured.
/// - `Aeron(Channel, i32)`: An Aeron-based channel plus a stream ID, which identifies the specific data stream.
#[derive(Debug, Clone)]
pub enum Distributed {
    /// No distributed communication is configured.
    None,
    /// An Aeron-based communication channel and its associated `stream_id`.
    ///
    /// * `Channel`: Describes how Aeron should send/receive data (unicast, IPC, or multicast).
    Aeron(Channel),
}

/// Represents the capability for point-to-point communication.
///
/// This trait is typically implemented by builder components that can
/// configure source network interface and reliability for unicast or IPC channels.
pub trait PointToPoint {
    /// Configures a specific interface (`Endpoint`) on which the communication will bind.
    ///
    /// In some scenarios, a machine has multiple network interfaces and
    /// you want to pick one for sending/receiving data. This method helps
    /// you specify exactly which IP/port to use.
    ///
    /// # Arguments
    /// - `interface`: An `Endpoint` indicating `ip` and `port` to bind to.
    fn with_interface(self, interface: Endpoint) -> Self;

    /// Configures the reliability level of the communication.
    ///
    /// By default, Aeron can work with "best-effort" or "reliable" modes.
    /// This method lets you pick which one you want:
    /// - `ReliableConfig::Reliable` for retransmission in case of packet loss.
    /// - `ReliableConfig::Unreliable` for best effort (lower latency, possible packet loss).
    ///
    /// # Arguments
    /// - `reliability`: A `ReliableConfig` (either `Reliable` or `Unreliable`).
    fn with_reliability(self, reliability: ReliableConfig) -> Self;
}

/// Represents the capability for multicast communication.
///
/// This trait is typically implemented by builder components that can
/// configure multicast-specific parameters like TTL (time-to-live) and
/// control mode (dynamic vs manual).
pub trait Multicast {
    /// Provides a complete `MulticastConfig` object for configuring
    /// the control endpoint and TTL directly.
    ///
    /// Useful if you already have a fully-formed `MulticastConfig` object
    /// and want to apply it in one go.
    fn with_multicast_config(self, config: MulticastConfig) -> Self;

    /// Specifies how control messages for multicast are handled:
    /// - `ControlMode::Dynamic`: Aeron automatically manages control messages.
    /// - `ControlMode::Manual`: You take over handling control messages yourself.
    fn with_control_mode(self, control_mode: ControlMode) -> Self;

    /// Sets the time-to-live (TTL) for multicast packets, in router hops.
    ///
    /// Each router the packet crosses decrements the TTL by 1. When it reaches 0, it is dropped.
    /// Lower TTL keeps packets local; higher TTL allows wider network reach.
    fn with_ttl(self, ttl: u8) -> Self;
}

/// Represents a valid Aeron configuration for the builder pattern.
///
/// This generic struct can represent either:
/// - A point-to-point mode (`AeronConfig<PointToPointMode>`)  
/// - A multicast mode (`AeronConfig<MulticastMode>`)
///
/// It carries essential details such as the `media_type`, the target `endpoint`,
/// and mode-specific fields.
#[derive(Debug, Clone)]
pub struct AeronConfig<Mode> {
    /// The Aeron media type (e.g., `Udp`, `Ipc`, `SpyUdp`, or `SpyIpc`).
    media_type: MediaType,
    /// The primary `Endpoint` for communication.
    ///
    /// In point-to-point, this is usually the remote or local socket address
    /// (e.g., `127.0.0.1:40123`).
    /// In multicast, this may be the multicast group address/port (e.g., `224.0.1.1:40456`).
    endpoint: Endpoint,
    /// The specific mode of operation—point-to-point or multicast—with its own parameters.
    mode: Mode,
}

/// Marker type for point-to-point configurations.
///
/// This contains optional parameters like:
/// - `interface`: The local network interface to bind, if specified.
/// - `reliability`: Whether the channel is reliable or best-effort.
#[derive(Debug, Clone)]
pub struct PointToPointMode {
    /// The local interface endpoint, if any (e.g., `192.168.1.2:0`).
    pub interface: Option<Endpoint>,
    /// The reliability setting for the channel.
    /// - `Some(ReliableConfig::Reliable)` for reliable retransmissions.
    /// - `Some(ReliableConfig::Unreliable)` for best-effort.
    pub reliability: Option<ReliableConfig>,
}

/// Marker type for multicast configurations.
///
/// This includes:
/// - The `MulticastConfig` struct (where you can set `control` endpoint and `ttl`).
/// - The `ControlMode` indicating how control messages are handled.
#[derive(Debug, Clone)]
pub struct MulticastMode {
    /// Configuration for multicast packets (control endpoint, TTL, etc.).
    pub config: MulticastConfig,
    /// Specifies whether the control messages are handled automatically (`Dynamic`)
    /// or manually (`Manual`).
    pub control_mode: ControlMode,
}

impl AeronConfig<PointToPointMode> {
    /// Creates a new Aeron configuration for point-to-point communication.
    ///
    /// # Panics
    /// Panics if `media_type` is not one of `Udp`, `SpyUdp`, `Ipc`, or `SpyIpc`.
    pub fn new_point_to_point(media_type: MediaType, endpoint: Endpoint) -> Self {
        if matches!(media_type, MediaType::Udp | MediaType::SpyUdp | MediaType::Ipc | MediaType::SpyIpc) {
            Self {
                media_type,
                endpoint,
                mode: PointToPointMode {
                    interface: None,
                    reliability: None,
                },
            }
        } else {
            panic!("Invalid media type for point-to-point configuration");
        }
    }

    /// Sets the interface for point-to-point communication.
    ///
    /// This determines the local IP/port to bind to. If you're listening or
    /// sending packets from a specific network card, set its IP and port here.
    ///
    /// # Arguments
    /// - `ip`: The IP address of the local interface.
    /// - `port`: The port to use on that interface (often 0 for auto-assignment).
    pub fn with_interface(self, ip: IpAddr, port: u16) -> Self {
        let mut mode = self.mode.clone();
        mode.interface = Some(Endpoint { ip, port });
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Sets the reliability for the point-to-point communication.
    ///
    /// # Arguments
    /// - `reliability`: A `ReliableConfig`, either `Reliable` or `Unreliable`.
    pub fn with_reliability(self, reliability: ReliableConfig) -> Self {
        let mut mode = self.mode.clone();
        mode.reliability = Some(reliability);
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Builds the `Channel` instance with the specified configuration.
    ///
    /// # Returns
    /// A `Channel::PointToPoint` variant that Aeron can use for sending/receiving data.
    pub fn build(self) -> Channel {
        Channel::PointToPoint {
            media_type: self.media_type,
            endpoint: self.endpoint,
            interface: self.mode.interface,
            reliability: self.mode.reliability,
        }
    }
}

impl AeronConfig<MulticastMode> {
    /// Creates a new Aeron configuration for multicast communication.
    ///
    /// # Panics
    /// Panics if `media_type` is not `Udp` or `SpyUdp`.  
    /// (Multicast requires a UDP-based transport.)
    pub fn new_multicast(media_type: MediaType, endpoint: Endpoint, control_ip: IpAddr, control_port: u16) -> Self {
        if matches!(media_type, MediaType::Udp | MediaType::SpyUdp) {
            Self {
                media_type,
                endpoint,
                mode: MulticastMode {
                    config: MulticastConfig {
                        control: Endpoint {
                            ip: control_ip,
                            port: control_port,
                        },
                        ttl: None,
                    },
                    control_mode: ControlMode::Dynamic,
                },
            }
        } else {
            panic!("Invalid media type for multicast configuration");
        }
    }

    /// Sets the control mode for multicast packets.
    ///
    /// # Arguments
    /// - `control_mode`: A `ControlMode` indicating whether control messages
    ///   are dynamically handled by Aeron or configured manually.
    pub fn with_control_mode(self, control_mode: ControlMode) -> Self {
        let mut mode = self.mode.clone();
        mode.control_mode = control_mode;
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Sets the time-to-live (TTL) for multicast packets, in hops.
    ///
    /// This controls how many network devices (routers) can forward the multicast packet
    /// before it is dropped.
    ///
    /// # Arguments
    /// - `ttl`: An integer representing the number of hops allowed (e.g., 1, 5, 8).
    pub fn with_ttl(self, ttl: u8) -> Self {
        let mut mode = self.mode.clone();
        mode.config.ttl = Some(ttl);
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Builds the `Channel` instance with the specified multicast configuration.
    ///
    /// # Returns
    /// A `Channel::Multicast` variant that Aeron can use for sending/receiving data
    /// to/from a multicast group.
    pub fn build(self) -> Channel {
        Channel::Multicast {
            media_type: self.media_type,
            endpoint: self.endpoint,
            config: self.mode.config,
            control_mode: self.mode.control_mode,
        }
    }
}

/// Builder for configuring and creating a `Distributed` instance.
///
/// This builder supports multiple distributed technologies, starting with Aeron.
/// It can be extended to support others like ZeroMQ or Kafka by adding
/// additional configuration pathways.
///
/// The builder is generic over a configuration type `T`. Initially, it's `()`,
/// which means "no configuration yet." Once you pick a technology (e.g., Aeron),
/// it transitions to a more specific builder.
#[derive(Debug, Clone)]
pub struct DistributionBuilder<T> {
    /// The current configuration state stored in this builder.
    config: T,
}

impl DistributionBuilder<()> {
    /// Creates a new `DistributedBuilder` instance for Aeron communication.
    ///
    /// This resets the builder to a state where you're ready to configure Aeron
    /// channels (either point-to-point or multicast).
    ///
    /// # Returns
    /// An [`AeronBuilder`] that lets you set the media type (`Udp`, `Ipc`, etc.)
    /// and pick the communication mode (point-to-point or multicast).
    pub fn aeron() -> AeronBuilder {
        AeronBuilder::default()
    }
}

/// Builder for configuring Aeron communication.
///
/// The `AeronBuilder` starts with an optional media type. You must call
/// `with_media_type()` to specify `Udp`, `Ipc`, `SpyUdp`, or `SpyIpc`.
/// After setting the media type, you can build either point-to-point
/// or multicast channels using the relevant methods.
#[derive(Debug, Clone, Default)]
pub struct AeronBuilder {
    /// The chosen media type, if set. If not set, building will panic.
    media_type: Option<MediaType>,
}


impl AeronBuilder {
    /// Sets the media type for Aeron communication.
    ///
    /// # Arguments
    /// - `media_type`: A `MediaType` variant (e.g., `Udp` for network, `Ipc` for local IPC).
    ///
    /// # Returns
    /// An updated `AeronBuilder` ready to pick the communication style (point-to-point or multicast).
    pub fn with_media_type(self, media_type: MediaType) -> Self {
        Self { media_type: Some(media_type) }
    }

    /// Sets up a point-to-point communication channel.
    ///
    /// # Arguments
    /// - `ip`: The IP address of the endpoint (where to send/receive).
    /// - `port`: The port number of the endpoint.
    ///
    /// # Returns
    /// A `DistributionBuilder<AeronConfig<PointToPointMode>>` for further point-to-point configuration.
    ///
    /// # Panics
    /// Panics if no media type has been specified.
    pub fn point_to_point(self, ip: IpAddr, port: u16) -> DistributionBuilder<AeronConfig<PointToPointMode>> {
        let media_type = self.media_type.expect("Media type must be specified");
        DistributionBuilder {
            config: AeronConfig::new_point_to_point(media_type, Endpoint { ip, port }),
        }
    }

    /// Sets up a multicast communication channel.
    ///
    /// # Arguments
    /// - `ip`: The IP address of the multicast endpoint (often `0.0.0.0` for local bind).
    /// - `port`: The port number of the multicast endpoint.
    /// - `control_ip`: The IP address for the multicast control endpoint (e.g., `224.0.1.1`).
    /// - `control_port`: The port for the multicast control endpoint.
    ///
    /// # Returns
    /// A `DistributionBuilder<AeronConfig<MulticastMode>>` for further multicast configuration.
    ///
    /// # Panics
    /// Panics if no media type has been specified.
    pub fn multicast(
        self,
        ip: IpAddr,
        port: u16,
        control_ip: IpAddr,
        control_port: u16
    ) -> DistributionBuilder<AeronConfig<MulticastMode>> {
        let media_type = self.media_type.expect("Media type must be specified");
        DistributionBuilder {
            config: AeronConfig::new_multicast(media_type, Endpoint { ip, port }, control_ip, control_port),
        }
    }
}

impl DistributionBuilder<AeronConfig<PointToPointMode>> {
    /// Sets the interface for point-to-point communication (local bind IP and port).
    ///
    /// # Arguments
    /// - `ip`: The local interface IP.
    /// - `port`: The local interface port.
    ///
    /// # Returns
    /// A new `DistributionBuilder` wrapping the updated configuration.
    pub fn with_interface(self, ip: IpAddr, port: u16) -> Self {
        Self {
            config: self.config.with_interface(ip, port),
        }
    }

    /// Sets the reliability for the point-to-point communication.
    ///
    /// # Arguments
    /// - `reliability`: The `ReliableConfig` to use (`Reliable` or `Unreliable`).
    ///
    /// # Returns
    /// A new `DistributionBuilder` wrapping the updated configuration.
    pub fn with_reliability(self, reliability: ReliableConfig) -> Self {
        Self {
            config: self.config.with_reliability(reliability),
        }
    }

    /// Builds the `Distributed` instance with the specified stream ID.
    ///
    /// # Arguments
    /// - `stream_id`: The Aeron stream ID for identifying the data stream.
    ///
    /// # Returns
    /// A `Distributed` enum variant containing the configured Aeron channel and stream ID.
    pub fn build(self) -> Distributed {
        Distributed::Aeron(self.config.build())
    }
}

impl DistributionBuilder<AeronConfig<MulticastMode>> {
    /// Sets the control mode for multicast packets.
    ///
    /// # Arguments
    /// - `control_mode`: `ControlMode::Dynamic` or `ControlMode::Manual`.
    ///
    /// # Returns
    /// A new `DistributionBuilder` wrapping the updated configuration.
    pub fn with_control_mode(self, control_mode: ControlMode) -> Self {
        Self {
            config: self.config.with_control_mode(control_mode),
        }
    }

    /// Sets the time-to-live (TTL) for multicast packets.
    ///
    /// # Arguments
    /// - `ttl`: The TTL in router hops (e.g., 1, 5, 8).
    ///
    /// # Returns
    /// A new `DistributionBuilder` wrapping the updated configuration.
    pub fn with_ttl(self, ttl: u8) -> Self {
        Self {
            config: self.config.with_ttl(ttl),
        }
    }

    /// Builds the `Distributed` instance with the specified stream ID.
    ///
    /// # Arguments
    /// - `stream_id`: The Aeron stream ID for identifying the data stream.
    ///
    /// # Returns
    /// A `Distributed` enum variant containing the configured Aeron channel and stream ID.
    pub fn build(self) -> Distributed {
        Distributed::Aeron(self.config.build())
    }
}

// Example usage:
//
// let builder = DistributedBuilder::aeron()
//     .with_media_type(MediaType::Udp)
//     .point_to_point("127.0.0.1".parse().unwrap(), 40123);
// let distributed = builder
//     .with_interface("192.168.1.1".parse().unwrap(), 12345)
//     .with_reliability(ReliableConfig::Reliable)
//     .build();
//
// let multicast_builder = DistributedBuilder::aeron()
//     .with_media_type(MediaType::Udp)
//     .multicast(
//         "0.0.0.0".parse().unwrap(),
//         40456,
//         "224.0.1.1".parse().unwrap(),
//         40457,
//     );
// let multicast_distributed = multicast_builder
//     .with_control_mode(ControlMode::Manual)
//     .with_ttl(8)
//     .build();
