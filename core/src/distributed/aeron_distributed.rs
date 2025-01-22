use std::cmp::PartialEq;
use crate::distributed::aeron_channel::*;

#[derive(Debug,Clone)]
pub enum DistributedTech {
    None,
    Aeron(Channel),
    // Add more types here as needed
}


/////////////////////////////////////

/// Internal marker for whether the config is point-to-point or multicast.
///
/// We store it inside `AeronConfig` so at build-time we know which
/// `Channel` variant to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AeronMode {
    None,
    PointToPoint,
    Multicast,
}

/// A single builder struct holding **all** Aeron-related configuration.
///
/// It creates either:
/// - A point-to-point channel (`Channel::PointToPoint`)
/// - A multicast channel (`Channel::Multicast`)
///
/// # Cloning Pattern
/// Each `with_` or `use_` method takes `&self`, clones this config,
/// then updates the clone and returns it. This ensures the original
/// builder is never mutated.
#[derive(Debug, Clone)]
pub struct AeronConfig {
    // -------------------------------
    // Common fields for any channel
    // -------------------------------
    media_type: Option<MediaType>,
    term_length: Option<usize>,

    // --------------------------------------
    // Specific mode: none, point-to-point, or multicast
    // --------------------------------------
    mode: AeronMode,

    // Endpoint is used in both P2P and multicast
    endpoint: Option<Endpoint>,

    // For point-to-point:
    interface: Option<Endpoint>,
    reliability: Option<ReliableConfig>,

    // For multicast:
    control_endpoint: Option<Endpoint>,
    control_mode: Option<ControlMode>,
    ttl: Option<u8>,
}


impl AeronConfig {
    /// Creates a new (empty) Aeron configuration.
    ///
    /// You must call at least one of:
    /// - `use_point_to_point(...)`
    /// - `use_multicast(...)`
    ///
    /// And you must also set a `media_type` before building.
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

    // -------------------------------------------------
    //       COMMON / SHARED METHODS
    // -------------------------------------------------

    /// Sets the Aeron `MediaType` (e.g., `Udp`, `Ipc`, `SpyUdp`, `SpyIpc`).
    ///
    pub fn with_media_type(&self, media_type: MediaType) -> Self {
        let mut clone = self.clone();
        clone.media_type = Some(media_type);
        clone
    }

    /// Sets an optional Aeron term buffer length (e.g. 1 MB).
    ///
    pub fn with_term_length(&self, term_length: usize) -> Self {
        let mut clone = self.clone();
        clone.term_length = Some(term_length);
        clone
    }

    // -------------------------------------------------
    //          POINT-TO-POINT METHODS
    // -------------------------------------------------

    /// Switches this builder to **point-to-point** mode, with the given endpoint.
    ///
    /// - `endpoint`: Where to send/receive data (e.g. `127.0.0.1:40123`).
    ///
    /// # Panics
    /// Panics if the chosen `media_type` is invalid for P2P. Typically,
    /// valid media types for P2P are `Udp`, `SpyUdp`, `Ipc`, and `SpyIpc`.
    ///
    pub fn use_point_to_point(&self, endpoint: Endpoint) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::PointToPoint;
        clone.endpoint = Some(endpoint);
        clone
    }

    /// Sets the local interface for **point-to-point** communication.
    ///
    /// - `interface`: The IP/port to bind locally (e.g. `192.168.1.10:0`).
    ///
    /// # Panics
    /// Panics if not currently in P2P mode.
    ///
    pub fn with_interface(&self, interface: Endpoint) -> Self {
        if self.mode != AeronMode::PointToPoint {
            panic!("`with_interface` called but not in point-to-point mode.");
        }
        let mut clone = self.clone();
        clone.interface = Some(interface);
        clone
    }

    /// Sets the reliability for **point-to-point** communication.
    ///
    /// - `reliability`: `ReliableConfig::Reliable` or `ReliableConfig::Unreliable`.
    ///
    /// # Panics
    /// Panics if not currently in P2P mode.
    ///
    pub fn with_reliability(&self, reliability: ReliableConfig) -> Self {
        if self.mode != AeronMode::PointToPoint {
            panic!("`with_reliability` called but not in point-to-point mode.");
        }
        let mut clone = self.clone();
        clone.reliability = Some(reliability);
        clone
    }

    // -------------------------------------------------
    //            MULTICAST METHODS
    // -------------------------------------------------

    /// Switches this builder to **multicast** mode, specifying both the group endpoint
    /// and the control endpoint.
    ///
    /// - `group_endpoint`: The multicast group (e.g. `224.0.1.1:40456`).
    /// - `control_endpoint`: The control endpoint for joining/leaving the group.
    ///
    /// # Panics
    /// Panics if `media_type` is invalid for multicast (usually must be `Udp` or `SpyUdp`).
    ///
    pub fn use_multicast(&self, group_endpoint: Endpoint, control_endpoint: Endpoint) -> Self {
        let mut clone = self.clone();
        clone.mode = AeronMode::Multicast;
        clone.endpoint = Some(group_endpoint);
        clone.control_endpoint = Some(control_endpoint);
        clone
    }

    /// Sets the control mode for **multicast** (either `Dynamic` or `Manual`).
    ///
    /// # Panics
    /// Panics if not in multicast mode.
    ///
     pub fn with_control_mode(&self, control_mode: ControlMode) -> Self {
        if self.mode != AeronMode::Multicast {
            panic!("`with_control_mode` called but not in multicast mode.");
        }
        let mut clone = self.clone();
        clone.control_mode = Some(control_mode);
        clone
    }

    /// Sets the Time-to-Live (TTL) for **multicast** packets.
    ///
    /// # Panics
    /// Panics if not in multicast mode.
    ///
    pub fn with_ttl(&self, ttl: u8) -> Self {
        if self.mode != AeronMode::Multicast {
            panic!("`with_ttl` called but not in multicast mode.");
        }
        let mut clone = self.clone();
        clone.ttl = Some(ttl);
        clone
    }

    // -------------------------------------------------
    //               FINAL BUILD
    // -------------------------------------------------

    /// Consumes the builder and produces a final [`Channel`].
    ///
    /// # Panics
    /// Panics if required fields are missing or if `mode` is `None`.
    ///
    pub fn build(&self) -> Channel {
        let media_type = self.media_type
            .expect("media_type must be set before build()");

        match self.mode {
            AeronMode::None => {                
                panic!("No channel mode selected (point-to-point or multicast).");
                               
            }

            AeronMode::PointToPoint => {
                // P2P must have an endpoint
                let endpoint = self.endpoint
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
                // Mcast must have an endpoint and a control endpoint
                let endpoint = self.endpoint
                    .expect("multicast group endpoint must be set");
                let control_ep = self.control_endpoint
                    .expect("control endpoint must be set for multicast");

                Channel::Multicast {
                    media_type,
                    endpoint,
                    config: MulticastConfig {
                        control: control_ep,
                        ttl: self.ttl,
                    },
                    control_mode: self.control_mode
                        .unwrap_or(ControlMode::Dynamic),
                    term_length: self.term_length,
                }
            }
        }
    }
}

// -------------------------------
// EXAMPLE USAGE (IN TESTS)
// -------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_p2p() {
        let p2p_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port: 40123,
            })
            .with_interface(Endpoint {
                ip: "192.168.1.10".parse().expect("ip"),
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
                assert!(interface.is_some());
                assert_eq!(reliability, Some(ReliableConfig::Reliable));
                assert_eq!(term_length, Some(1_048_576));
            }
            _ => panic!("Expected a PointToPoint channel"),
        }
    }

    #[test]
    fn build_multicast() {
        let mcast_channel = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_multicast(
                Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: 40456,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: 40457,
                }
            )
            .with_control_mode(ControlMode::Manual)
            .with_ttl(5)
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
                assert!(term_length.is_none());
            }
            _ => panic!("Expected a Multicast channel"),
        }
    }
}
