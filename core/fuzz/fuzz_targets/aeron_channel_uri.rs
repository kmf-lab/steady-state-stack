#![no_main]

//! Coverage-guided fuzz of [`Channel::cstring`] URI construction.

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use steady_state::distributed::aeron_channel_structs::{
    ControlMode, MulticastConfig, ReliableConfig,
};
use steady_state::{Channel, Endpoint, MediaType};

#[derive(Arbitrary, Debug)]
struct FuzzChannel {
    multicast: bool,
    media: u8,
    v6: bool,
    ip: [u8; 16],
    port: u16,
    with_iface: bool,
    iface_ip: [u8; 16],
    iface_port: u16,
    reliable: Option<bool>,
    term_length: Option<u16>,
    ttl: Option<u8>,
    control_port: u16,
    control_manual: bool,
}

fn media_type(tag: u8) -> MediaType {
    match tag % 4 {
        0 => MediaType::Udp,
        1 => MediaType::Ipc,
        2 => MediaType::SpyUdp,
        _ => MediaType::SpyIpc,
    }
}

fn ip_addr(v6: bool, bytes: [u8; 16]) -> IpAddr {
    if v6 {
        IpAddr::V6(Ipv6Addr::from(bytes))
    } else {
        IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]))
    }
}

fn expected_prefix(media: MediaType) -> &'static str {
    match media {
        MediaType::Udp => "aeron:udp",
        MediaType::Ipc => "aeron:ipc",
        MediaType::SpyUdp => "aeron-spy:aeron:udp",
        MediaType::SpyIpc => "aeron-spy:aeron:ipc",
    }
}

fuzz_target!(|input: FuzzChannel| {
    let media = media_type(input.media);
    let endpoint = Endpoint {
        ip: ip_addr(input.v6, input.ip),
        port: input.port,
    };
    let reliability = input.reliable.map(|r| {
        if r {
            ReliableConfig::Reliable
        } else {
            ReliableConfig::Unreliable
        }
    });
    let term_length = input.term_length.map(|t| t as usize);
    let interface = if input.with_iface {
        Some(Endpoint {
            ip: ip_addr(input.v6, input.iface_ip),
            port: input.iface_port,
        })
    } else {
        None
    };

    let channel = if input.multicast {
        Channel::Multicast {
            media_type: media,
            endpoint,
            config: MulticastConfig {
                control: Endpoint {
                    ip: ip_addr(input.v6, input.iface_ip),
                    port: input.control_port,
                },
                ttl: input.ttl,
            },
            control_mode: if input.control_manual {
                ControlMode::Manual
            } else {
                ControlMode::Dynamic
            },
            term_length,
        }
    } else {
        Channel::PointToPoint {
            media_type: media,
            endpoint,
            interface,
            reliability,
            term_length,
        }
    };

    // Panic here is a real defect (CString::new expect on interior NUL).
    let uri = channel
        .cstring()
        .into_string()
        .expect("Aeron URI must be valid UTF-8");
    let prefix = expected_prefix(media);
    assert!(
        uri.starts_with(prefix),
        "uri {uri:?} must start with {prefix}"
    );
});
