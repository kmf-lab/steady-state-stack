use std::net::IpAddr;
use crate::distributed::aeron_channel::*;

/// Represents different distributed technologies that can be configured and built.
/// Currently, it supports Aeron, and can be extended to include technologies like ZeroMQ.
#[derive(Debug, Clone)]
pub enum Distributed {
    None,
    Aeron(Channel,i32),
}

/// Represents the capability for point-to-point communication.
pub trait PointToPoint {
    fn with_interface(self, interface: Endpoint) -> Self;
    fn with_reliability(self, reliability: ReliableConfig) -> Self;
}

/// Represents the capability for multicast communication.
pub trait Multicast {
    fn with_multicast_config(self, config: MulticastConfig) -> Self;
    fn with_control_mode(self, control_mode: ControlMode) -> Self;
    fn with_ttl(self, ttl: u8) -> Self;
}

/// Represents a valid Aeron configuration for the builder pattern.
#[derive(Debug, Clone)]
pub struct AeronConfig<Mode> {
    media_type: MediaType,
    endpoint: Endpoint,
    mode: Mode,
}

/// Marker type for point-to-point configurations.
#[derive(Debug, Clone)]
pub struct PointToPointMode {
    pub interface: Option<Endpoint>,
    pub reliability: Option<ReliableConfig>,
}

/// Marker type for multicast configurations.
#[derive(Debug, Clone)]
pub struct MulticastMode {
    /// Configuration for multicast packets.
    pub config: MulticastConfig,
    /// Control mode for multicast packets.
    pub control_mode: ControlMode,
}

impl AeronConfig<PointToPointMode> {
    /// Creates a new Aeron configuration for point-to-point communication.
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
    pub fn with_control_mode(self, control_mode: ControlMode) -> Self {
        let mut mode = self.mode.clone();
        mode.control_mode = control_mode;
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Sets the time-to-live (TTL) for multicast packets.
    pub fn with_ttl(self, ttl: u8) -> Self {
        let mut mode = self.mode.clone();
        mode.config.ttl = Some(ttl);
        Self {
            media_type: self.media_type,
            endpoint: self.endpoint,
            mode,
        }
    }

    /// Builds the `Channel` instance with the specified configuration.
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
/// It can be extended to support others like ZeroMQ or Kafka by adding configuration methods.
#[derive(Debug, Clone)]
pub struct DistributionBuilder<T> {
    config: T,
}

impl DistributionBuilder<()> {
    /// Creates a new `DistributedBuilder` instance for Aeron communication.
    pub fn aeron() -> AeronBuilder {
        AeronBuilder::default()
    }
}

#[derive(Debug, Clone)]
/// Builder for configuring Aeron communication.
pub struct AeronBuilder {
    media_type: Option<MediaType>,
}

impl Default for AeronBuilder {
    fn default() -> Self {
        AeronBuilder { media_type: None }
    }
}

impl AeronBuilder {
    /// Sets the media type for Aeron communication.
    pub fn with_media_type(self, media_type: MediaType) -> Self {
        Self { media_type: Some(media_type) }
    }

    /// Sets point-to-point communication.
    pub fn point_to_point(self, ip: IpAddr, port: u16) -> DistributionBuilder<AeronConfig<PointToPointMode>> {
        let media_type = self.media_type.expect("Media type must be specified");
        DistributionBuilder {
            config: AeronConfig::new_point_to_point(media_type, Endpoint { ip, port }),
        }
    }

    /// Sets multicast communication.
    pub fn multicast(self, ip: IpAddr, port: u16, control_ip: IpAddr, control_port: u16) -> DistributionBuilder<AeronConfig<MulticastMode>> {
        let media_type = self.media_type.expect("Media type must be specified");
        DistributionBuilder {
            config: AeronConfig::new_multicast(media_type, Endpoint { ip, port }, control_ip, control_port),
        }
    }
}

impl DistributionBuilder<AeronConfig<PointToPointMode>> {
    
    /// Sets the interface for point-to-point communication.
    pub fn with_interface(self, ip: IpAddr, port: u16) -> Self {
        Self {
            config: self.config.with_interface(ip, port),
        }
    }

    /// Sets the reliability for the point-to-point communication.     
    pub fn with_reliability(self, reliability: ReliableConfig) -> Self {
        Self {
            config: self.config.with_reliability(reliability),
        }
    }

    /// Builds the `Distributed` instance with the specified stream ID.
    pub fn build(self, stream_id: i32) -> Distributed {
        Distributed::Aeron(self.config.build(), stream_id)
    }
}

impl DistributionBuilder<AeronConfig<MulticastMode>> {
    
    /// Sets the control mode for multicast packets.
    pub fn with_control_mode(self, control_mode: ControlMode) -> Self {
        Self {
            config: self.config.with_control_mode(control_mode),
        }
    }

    /// Sets the time-to-live (TTL) for multicast packets.
    pub fn with_ttl(self, ttl: u8) -> Self {
        Self {
            config: self.config.with_ttl(ttl),
        }
    }

    /// Builds the `Distributed` instance with the specified stream ID.
    pub fn build(self, stream_id: i32) -> Distributed {
        Distributed::Aeron(self.config.build(),stream_id)
    }
}

// Example usage
// ```
// let builder = DistributedBuilder::aeron()
//     .with_media_type(MediaType::Udp)
//     .point_to_point("127.0.0.1".parse().unwrap(), 40123);
// let distributed = builder
//     .with_interface("192.168.1.1".parse().unwrap(), 12345)
//     .with_reliability(ReliableConfig::Reliable)
//     .build();
// ```
// ```
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
// ```
