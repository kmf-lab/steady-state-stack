
use std::ffi::CString;
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};


pub(crate) mod aeron_utils {
    use std::ffi::CString;
    use std::sync::Arc;
    use futures_util::lock::Mutex;
    use steady_state_aeron::aeron::Aeron;
    use steady_state_aeron::context::Context;
    use steady_state_aeron::utils::errors::AeronError;
    use log::warn;
    use crate::ActorIdentity;
    
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
                warn!("Aeron context created using: {:?}", aeron.context().cnc_file_name());
                Some(Arc::new(Mutex::new(aeron)))
            }
            Err(e) => {
                warn!("Failed to create Aeron context: {:?}", e);
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MediaType {
    /// Standard UDP channel: `aeron:udp?…`
    Udp,
    /// IPC channel: `aeron:ipc`
    Ipc,
    /// Spy on a UDP channel: `aeron-spy:aeron:udp?…`
    SpyUdp,
    /// Spy on an IPC channel: `aeron-spy:aeron:ipc?…`
    SpyIpc,
}

#[derive(Debug, Clone, Copy)]
pub enum ControlMode {
    Dynamic,
    Manual,
}

#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct Interface {
    pub ip: IpAddr,
    pub port: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct MulticastConfig {
    pub control: Endpoint,
    pub ttl: Option<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct PointServiceConfig {
    pub interface: Option<Interface>,
    pub reliable: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
pub enum ReliableConfig {
    Reliable,
    Unreliable,
}


/// Represents all channel forms in Aeron:
/// - Point-to-point unicast or IPC
/// - Multicast
/// - Spy (implicitly handled by using MediaType::{SpyUdp, SpyIpc})
#[derive(Debug, Clone, Copy)]
pub enum Channel {
    /// Unicast or IPC channel
    PointToPoint {
        media_type: MediaType,
        endpoint: Endpoint,         // e.g., 127.0.0.1:40123
        interface: Option<Endpoint>,// for specifying interface=IP:port (mainly for UDP)
        reliability: Option<ReliableConfig>,
    },
    /// Multicast channel
    Multicast {
        media_type: MediaType,
        endpoint: Endpoint,         // e.g., 0.0.0.0:40456
        config: MulticastConfig,    // control address + TTL
        control_mode: ControlMode,
    },
}

pub(crate) fn is_port_open(port: u16) -> bool {
    // Very basic check: if we can bind to it, it's "open" from our perspective
    match UdpSocket::bind(("127.0.0.1", port)) {
        Ok(_) => true,
        Err(_) => false,
    }
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