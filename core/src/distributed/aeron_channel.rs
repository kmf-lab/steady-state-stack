
use std::ffi::CString;
use std::fmt::Debug;
use std::net::{IpAddr, UdpSocket};


pub(crate) mod aeron_utils {
    use std::ffi::CString;
    use std::sync::Arc;
    use futures_util::lock::Mutex;
    use steady_state_aeron::aeron::Aeron;
    use steady_state_aeron::context::Context;
    use steady_state_aeron::utils::errors::AeronError;
    use log::{info, trace, warn};
    
    //For more details see:: https://github.com/real-logic/aeron/wiki/Channel-Configuration

    fn error_handler(error: AeronError) {
        warn!("Error: {:?}", error);
    }

    fn on_new_publication_handler(
        channel: CString,
        stream_id: i32,
        session_id: i32,
        correlation_id: i64,
    ) {
        warn!(
            "Publication: {} stream:{} session:{} correlation:{}",
            channel.to_str().expect("internal"),
            stream_id,
            session_id,
            correlation_id
        );
    }

    pub fn aeron_context() -> Option<Arc<Mutex<Aeron>>> {
        let mut aeron_context = Context::new();

        // Example set-up instructions:
        // sudo chmod -R 2775 /dev/shm/aeron-default
        // sudo usermod -aG aeronmd <your-username>
        //
        // For advanced tuning, you might set:
        //     aeron.event.buffer.length = 8MB
        // etc.

        aeron_context.set_new_publication_handler(Box::new(on_new_publication_handler));
        aeron_context.set_error_handler(Box::new(error_handler));
        aeron_context.set_pre_touch_mapped_memory(true);
        aeron_context.set_aeron_dir("/dev/shm/aeron-default".parse().expect("valid path"));

        match Aeron::new(aeron_context) {
            Ok(aeron) => {
                trace!("Aeron context created using: {:?}", aeron.context().cnc_file_name());
                Some(Arc::new(Mutex::new(aeron)))
            }
            Err(e) => {
                trace!("Failed to create Aeron context: {:?}", e);
                None
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
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
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    /// The IP address of the endpoint (e.g., `127.0.0.1` or `192.168.1.100`).
    pub ip: IpAddr,
    /// The port number for communication (e.g., `40456`).
    pub port: u16,
}

/// Represents a network interface for binding UDP traffic.
///
/// The interface is used to specify which network card or IP address to use when
/// sending or receiving UDP traffic. This is particularly useful when a machine
/// has multiple network interfaces.
#[derive(Debug, Clone, Copy)]
pub struct Interface {
    /// The IP address of the network interface.
    /// For example, `192.168.1.1` can be used to bind to a specific interface.
    pub ip: IpAddr,
    /// The port number for the interface. Set to `0` for default binding.
    pub port: u16,
}

/// Configuration for multicast communication, including control messages and Time-to-Live (TTL).
///
/// Multicast is a method of sending data to multiple receivers at once. This struct
/// provides configuration options for multicast channels.
///
/// # Fields
/// - `control`: The control endpoint that manages the multicast session.
/// - `ttl`: The Time-to-Live (TTL) value, which specifies how far multicast packets can travel.
///
/// # Notes on TTL
/// TTL is measured in "hops." Each hop represents a router or device that forwards
/// the multicast packet. A TTL of `0` means the packet will not leave the host.
/// A TTL of `1` limits the packet to the local network. Higher values allow the
/// packet to travel further across routers.
#[derive(Debug, Clone, Copy)]
pub struct MulticastConfig {
    /// The control endpoint used to manage the multicast group.
    pub control: Endpoint,
    /// Time-to-Live in hops. This determines how many routers the multicast packet can pass through.
    /// For example:
    /// - `Some(0)`: Stays on the local machine.
    /// - `Some(1)`: Stays within the local subnet.
    /// - `Some(5)`: Can pass through up to 5 routers.
    pub ttl: Option<u8>,
}

/// Configuration for a point-to-point communication channel.
///
/// Point-to-point channels can use either unicast UDP or IPC. This struct
/// provides additional configuration options for binding the channel to
/// a specific interface and setting reliability.
///
/// # Fields
/// - `interface`: An optional network interface for binding.
/// - `reliable`: An optional setting for reliable communication.
#[derive(Debug, Clone, Copy)]
pub struct PointServiceConfig {
    /// Optional network interface for binding the channel.
    pub interface: Option<Interface>,
    /// Optional setting for reliable communication.
    /// - `Some(true)`: Ensures reliable communication with retransmissions.
    /// - `Some(false)`: Uses best-effort communication without retransmissions.
    pub reliable: Option<bool>,
}

/// Specifies the reliability configuration for a channel.
///
/// Reliability determines whether lost packets are retransmitted.
///
/// # Variants
/// - `Reliable`: Ensures reliable communication with retransmissions.
/// - `Unreliable`: Best-effort communication without retransmissions.
#[derive(Debug, Clone, Copy)]
pub enum ReliableConfig {
    /// Ensures reliable communication. Lost packets are retransmitted.
    Reliable,
    /// Best-effort communication. Packets may be lost if the network drops them.
    Unreliable,
}

/// Represents all forms of Aeron channels.
///
/// Channels define the communication path for data. Aeron supports:
/// - Point-to-point communication (unicast or IPC)
/// - Multicast communication
/// - Spy channels for monitoring traffic
///
/// # Variants
/// - `PointToPoint`: Used for unicast or IPC communication.
/// - `Multicast`: Used for multicast communication.
#[derive(Debug, Clone, Copy)]
pub enum Channel {
    /// Represents a point-to-point unicast or IPC channel.
    ///
    /// # Fields
    /// - `media_type`: The type of media transport (`MediaType`).
    /// - `endpoint`: The target endpoint for communication.
    /// - `interface`: An optional source interface for UDP communication.
    /// - `reliability`: An optional setting for reliable or unreliable communication.
    PointToPoint {
        /// Specifies the transport type (e.g., UDP or IPC).
        media_type: MediaType,
        /// The target endpoint for communication (e.g., `127.0.0.1:40123`).
        endpoint: Endpoint,
        /// Optional source interface for UDP communication.
        interface: Option<Endpoint>,
        /// Optional reliability configuration (`ReliableConfig`).
        reliability: Option<ReliableConfig>,
    },
    /// Represents a multicast communication channel.
    ///
    /// # Fields
    /// - `media_type`: The type of media transport (`MediaType`).
    /// - `endpoint`: The multicast group endpoint.
    /// - `config`: Configuration for multicast, including control and TTL.
    /// - `control_mode`: Specifies how control messages are managed.
    Multicast {
        /// Specifies the transport type (should be `MediaType::Udp` for multicast).
        media_type: MediaType,
        /// The multicast group endpoint (e.g., `224.0.1.1:40456`).
        endpoint: Endpoint,
        /// Multicast configuration, including control messages and TTL.
        config: MulticastConfig,
        /// Specifies how control messages are managed (`ControlMode`).
        control_mode: ControlMode,
    },
}

pub(crate) fn is_port_open(port: u16) -> bool {
    // Very basic check: if we can bind to it, it's "open" from our perspective
    UdpSocket::bind(("127.0.0.1", port)).is_ok()
}

impl Channel {
    /// Build a valid Aeron channel string according to official docs.
    ///
    /// - If `media_type` is `Udp`, we produce `aeron:udp?endpoint=host:port`.
    /// - If `media_type` is `Ipc`, we produce `aeron:ipc`.
    /// - If `media_type` is `SpyUdp`, we produce `aeron-spy:aeron:udp?endpoint=...`.
    /// - If `media_type` is `SpyIpc`, we produce `aeron-spy:aeron:ipc`.
    ///
    /// For multicast, we add `|control=...|control-mode=...|ttl=...` as needed.
    pub fn cstring(&self) -> CString {
        let channel_str = match self {
            Channel::PointToPoint {
                media_type,
                endpoint,
                interface,
                reliability,
            } => {
                match media_type {
                    MediaType::Udp => {
                        // Start building the base channel string for PointToPoint UDP
                        let base = "aeron:udp".to_string();
                        let mut query = format!("?endpoint={}{}", ip_to_string(&endpoint.ip), endpoint.port);

                        // Add interface if specified
                        if let Some(iface) = interface {
                            query.push_str(&format!(
                                "|interface={}{}",
                                ip_to_string(&iface.ip),
                                iface.port
                            ));
                        }

                        // Add reliability configuration if specified
                        if let Some(rel_cfg) = reliability {
                            let reliable_str = match rel_cfg {
                                ReliableConfig::Reliable => "true",
                                ReliableConfig::Unreliable => "false",
                            };
                            query.push_str(&format!("|reliable={}", reliable_str));
                        }

                        format!("{}{}", base, query)
                    }

                    MediaType::Ipc => {
                        // "aeron:ipc"
                        "aeron:ipc".to_string()
                    }

                    MediaType::SpyUdp => {
                        // Spy on UDP: "aeron-spy:aeron:udp?endpoint=..."
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

                    MediaType::SpyIpc => {
                        // Spy on IPC: "aeron:spy:aeron:ipc"
                        "aeron-spy:aeron:ipc".to_string()
                    }
                }
            }

            Channel::Multicast {
                media_type,
                endpoint,
                config,
                control_mode,
            } => {
                // Typical multicast: "aeron:udp?endpoint=...|control=...|control-mode=...|ttl=..."
                let (prefix, base_media) = match media_type {
                    MediaType::Udp => ("aeron:", "udp"),
                    MediaType::Ipc => ("aeron:", "ipc"), // Unlikely but allowed
                    MediaType::SpyUdp => ("aeron-spy:aeron:", "udp"),
                    MediaType::SpyIpc => ("aeron-spy:aeron:", "ipc"),
                };

                let mut s = format!(
                    "{}{}?endpoint={}{}",
                    prefix,
                    base_media,
                    ip_to_string(&endpoint.ip),
                    endpoint.port
                );

                s.push_str(&format!(
                    "|control={}{}",
                    ip_to_string(&config.control.ip),
                    config.control.port
                ));

                let mode_str = match control_mode {
                    ControlMode::Dynamic => "dynamic",
                    ControlMode::Manual => "manual",
                };
                s.push_str(&format!("|control-mode={}", mode_str));

                if let Some(ttl_val) = config.ttl {
                    s.push_str(&format!("|ttl={}", ttl_val));
                }

                s
            }
        };

        CString::new(channel_str).expect("Failed to create CString from channel string")
    }
}

/// Convert an IP address into a partial string, adding a colon if IPv4,
/// or bracket for IPv6 plus no trailing colon if using bracket notation.
fn ip_to_string(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(ipv4) => format!("{}:", ipv4),
        // For IPv6, Aeron URIs typically enclose IP in [ ], e.g. [::1], then the port follows.
        IpAddr::V6(ipv6) => format!("[{}]", ipv6),
    }
}

#[cfg(test)]
mod aeron_channel_tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=[::1]40123"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(connection.to_str().expect("valid string"), "aeron:ipc");
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron:udp?endpoint=127.0.0.1:40123|interface=192.168.1.5:40123|reliable=true"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:udp?endpoint=127.0.0.1:40123"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:ipc"
        );
    }

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
        };

        let connection = channel.cstring();
        assert_eq!(
            connection.to_str().expect("valid string"),
            "aeron-spy:aeron:udp?endpoint=0.0.0.0:40456|control=224.0.1.1:40457|control-mode=manual|ttl=4"
        );
    }
}

/*
The `AERON_EVENT_LOG` environment variable in Aeron is a bitmap that enables specific logging events. The value `0xffff` represents a 16-bit bitmap where every bit is set to `1`, enabling all possible logging events.

### Explanation of Each Bit in `AERON_EVENT_LOG`

Below is the breakdown of each bit, what it controls, and its hexadecimal representation:

| **Bit** | **Hex Value** | **Description**                                         |
|---------|---------------|---------------------------------------------------------|
| **0**   | `0x0001`      | **Frame Logging**: Logs Aeron frame-level events, including control frames, data frames, and acknowledgment frames. |
| **1**   | `0x0002`      | **Raw Frame Logging**: Logs raw Aeron frames at the lowest level, including protocol headers and payload. |
| **2**   | `0x0004`      | **Network Events**: Logs network-level events, such as sending and receiving packets. |
| **3**   | `0x0008`      | **Driver Commands**: Logs commands sent to the media driver (e.g., adding publications, subscriptions). |
| **4**   | `0x0010`      | **Driver Responses**: Logs responses from the media driver (e.g., acknowledgments of commands). |
| **5**   | `0x0020`      | **Media Driver Events**: Logs internal media driver events, including buffer allocations and releases. |
| **6**   | `0x0040`      | **Flow Control**: Logs flow control events, such as NAKs (negative acknowledgments) or loss recovery. |
| **7**   | `0x0080`      | **Congestion Control**: Logs congestion control events, such as changes in transmission rate. |
| **8**   | `0x0100`      | **Publication Events**: Logs publication-related events, such as buffers being sent. |
| **9**   | `0x0200`      | **Subscription Events**: Logs subscription-related events, such as buffers being received. |
| **10**  | `0x0400`      | **Error Logging**: Logs driver errors or protocol-level errors. |
| **11**  | `0x0800`      | **Driver Time Events**: Logs driver time-related events, such as polling intervals or latency measurements. |
| **12**  | `0x1000`      | **Image Events**: Logs Aeron image-related events (e.g., lifecycle events like image creation or loss). |
| **13**  | `0x2000`      | **Replay Events**: Logs events related to Aeron’s replay mechanism for stored messages. |
| **14**  | `0x4000`      | **Loss Events**: Logs message loss and recovery events. |
| **15**  | `0x8000`      | **All Other Events**: Catches any Aeron-related events not explicitly covered by the other bits. |

---

### Interpreting the Value `0xffff`

`0xffff` in hexadecimal means all 16 bits are set to `1`. Each bit represents a specific logging category, so setting all bits to `1` enables logging for **all possible events**.

In binary, `0xffff` looks like this:

```
1111 1111 1111 1111
```

This enables all the above event categories. It's the most verbose logging level, useful for debugging.

---

### Enabling Specific Logging Categories

To enable only specific categories, you can set the value of `AERON_EVENT_LOG` to the **bitwise OR** of the desired hex values. For example:

- To log **frame-level events** and **network events** only:
```
0x0001 (Frame Logging) | 0x0004 (Network Events) = 0x0005
```
Set:
```bash
export AERON_EVENT_LOG="0x0005"
```

- To log **errors** and **image events** only:
```
0x0400 (Error Logging) | 0x1000 (Image Events) = 0x1400
```
Set:
```bash
export AERON_EVENT_LOG="0x1400"
```

---

### Use Cases

1. **Debugging Connectivity**:
- Set `AERON_EVENT_LOG="0x0007"` to enable logging for **frames** (`0x0001`), **raw frames** (`0x0002`), and **network events** (`0x0004`).

2. **Investigating Loss or Congestion**:
- Set `AERON_EVENT_LOG="0x0040 | 0x4000"` to enable logging for **flow control** (`0x0040`) and **loss events** (`0x4000`).

3. **Error Analysis**:
- Set `AERON_EVENT_LOG="0x0400"` to enable **error logging** only.

---

### Default Value

If `AERON_EVENT_LOG` is not set, logging is typically disabled or limited to critical errors. Using `0xffff` is suitable for development and debugging but may generate a large volume of logs in production.

For production environments, consider enabling only specific logging categories relevant to diagnosing issues.

---

By understanding the value of each bit, you can fine-tune Aeron's logging to suit your debugging needs and reduce unnecessary verbosity when required.

*/