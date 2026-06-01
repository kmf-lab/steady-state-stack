//! Aeron URI / channel builder contracts (no media driver required).

mod common;

use steady_state::distributed::aeron_channel_builder::AeronConfig;
use steady_state::distributed::aeron_channel_structs::{Channel, ControlMode, Endpoint, MediaType, ReliableConfig};
use common::support::pub_sub_harness::{channel_ipc, channel_multicast, channel_udp_p2p, port_from_salt};

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_integration_uri_ipc_contains_media() {
    let ch = channel_ipc();
    let uri = ch.cstring().into_string().expect("cstring");
    assert!(uri.contains("ipc"), "IPC URI should contain ipc: {uri}");
}

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_integration_uri_udp_p2p_endpoint() {
    let port = port_from_salt(80);
    let ch = channel_udp_p2p(port);
    let uri = ch.cstring().into_string().expect("cstring");
    assert!(uri.contains("udp"), "UDP URI: {uri}");
    assert!(uri.contains(&port.to_string()), "UDP URI should include port: {uri}");
}

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_integration_uri_multicast_endpoints() {
    let ch = channel_multicast(port_from_salt(90), port_from_salt(190));
    let uri = ch.cstring().into_string().expect("cstring");
    assert!(uri.contains("udp"), "multicast URI: {uri}");
    assert!(uri.contains("224.0.1.1"), "multicast group: {uri}");
}

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_integration_uri_builder_matches_channel_enum() {
    let p2p = AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .use_point_to_point(Endpoint {
            ip: "127.0.0.1".parse().expect("ip"),
            port: 40123,
        })
        .with_reliability(ReliableConfig::Reliable)
        .build();
    match p2p {
        Channel::PointToPoint { media_type, endpoint, .. } => {
            assert_eq!(media_type, MediaType::Udp);
            assert_eq!(endpoint.port, 40123);
        }
        _ => panic!("expected point-to-point"),
    }

    let mcast = AeronConfig::new()
        .with_media_type(MediaType::Udp)
        .use_multicast(
            Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: 40456,
            },
            Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: 40457,
            },
        )
        .with_control_mode(ControlMode::Manual)
        .build();
    match mcast {
        Channel::Multicast { control_mode, .. } => {
            assert_eq!(control_mode, ControlMode::Manual);
        }
        _ => panic!("expected multicast"),
    }
}

#[test]
// ss[verify distributed.aeron-uri]
fn aeron_integration_uri_salt_roundtrip_contains_tokens() {
    for salt in 0..200u16 {
        let port = port_from_salt(salt);
        let udp = channel_udp_p2p(port);
        let uri = udp.cstring().into_string().expect("cstring");
        assert!(uri.contains("udp"), "salt {salt}: {uri}");
        assert!(uri.contains(&port.to_string()), "salt {salt}: {uri}");
        let ipc = channel_ipc();
        let ipc_uri = ipc.cstring().into_string().expect("cstring");
        assert!(ipc_uri.contains("ipc"), "salt {salt}: {ipc_uri}");
    }
}

use proptest::prop_assert;

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]

    /// Property: AeronConfig-built UDP channel URIs include media and endpoint port.
    #[test]
    // ss[verify distributed.aeron-uri]
    fn aeron_integration_uri_proptest_udp_port_in_uri(port in 40100u16..41200u16) {
        let ch = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_point_to_point(Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port,
            })
            .with_reliability(ReliableConfig::Reliable)
            .build();
        let uri = ch.cstring().into_string().expect("cstring");
        prop_assert!(uri.contains("udp"), "uri: {uri}");
        prop_assert!(uri.contains(&port.to_string()), "uri: {uri}");
    }

    /// Property: IPC channel URIs always contain the ipc media token.
    #[test]
    // ss[verify distributed.aeron-uri]
    fn aeron_integration_uri_proptest_ipc_token(_seed in 0u8..255u8) {
        let _ = _seed;
        let uri = channel_ipc().cstring().into_string().expect("cstring");
        prop_assert!(uri.contains("ipc"), "uri: {uri}");
    }
}
