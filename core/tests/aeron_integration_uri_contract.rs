//! Aeron URI / channel builder contracts (no media driver required).

mod common;

use steady_state::distributed::aeron_channel_builder::AeronConfig;
use steady_state::distributed::aeron_channel_structs::{Channel, ControlMode, Endpoint, MediaType, ReliableConfig};
use steady_state::SS_PROPCASES;
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
use proptest::prop_assert_eq;

fn required_uri_tokens(channel: &Channel) -> Vec<String> {
    match channel {
        Channel::PointToPoint {
            media_type,
            endpoint,
            interface,
            reliability,
            term_length,
        } => {
            let mut tokens = match media_type {
                MediaType::Udp => vec!["aeron:udp".to_string(), "endpoint=".to_string()],
                MediaType::Ipc => vec!["aeron:ipc".to_string()],
                MediaType::SpyUdp => {
                    vec!["aeron-spy:aeron:udp".to_string(), "endpoint=".to_string()]
                }
                MediaType::SpyIpc => vec!["aeron-spy:aeron:ipc".to_string()],
            };
            if matches!(media_type, MediaType::Udp | MediaType::SpyUdp) {
                tokens.push(endpoint.port.to_string());
                if interface.is_some() {
                    tokens.push("interface=".to_string());
                }
                if let Some(rel) = reliability {
                    tokens.push(match rel {
                        ReliableConfig::Reliable => "reliable=true".to_string(),
                        ReliableConfig::Unreliable => "reliable=false".to_string(),
                    });
                }
            }
            if term_length.is_some() {
                tokens.push("term-length=".to_string());
            }
            tokens
        }
        Channel::Multicast {
            media_type,
            endpoint,
            config,
            control_mode,
            term_length,
        } => {
            let mut tokens = match media_type {
                MediaType::Udp => vec![
                    "aeron:udp".to_string(),
                    "endpoint=".to_string(),
                    "control=".to_string(),
                ],
                MediaType::Ipc => vec![
                    "aeron:ipc".to_string(),
                    "endpoint=".to_string(),
                    "control=".to_string(),
                ],
                MediaType::SpyUdp => vec![
                    "aeron-spy:aeron:udp".to_string(),
                    "endpoint=".to_string(),
                    "control=".to_string(),
                ],
                MediaType::SpyIpc => vec![
                    "aeron-spy:aeron:ipc".to_string(),
                    "endpoint=".to_string(),
                    "control=".to_string(),
                ],
            };
            tokens.push(endpoint.port.to_string());
            tokens.push(config.control.port.to_string());
            tokens.push(match control_mode {
                ControlMode::Dynamic => "control-mode=dynamic".to_string(),
                ControlMode::Manual => "control-mode=manual".to_string(),
            });
            if config.ttl.is_some() {
                tokens.push("ttl=".to_string());
            }
            if term_length.is_some() {
                tokens.push("term-length=".to_string());
            }
            tokens
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(SS_PROPCASES))]

    /// Property: AeronConfig-built UDP channel URIs include media and endpoint port.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
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
    // ss[verify verify.process.proptest]
    fn aeron_integration_uri_proptest_ipc_token(_seed in 0u8..255u8) {
        let _ = _seed;
        let uri = channel_ipc().cstring().into_string().expect("cstring");
        prop_assert!(uri.contains("ipc"), "uri: {uri}");
    }

    /// Property: multicast URIs include group and both port endpoints.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn aeron_integration_uri_proptest_multicast_ports(
        control_port in 40100u16..41200u16,
        data_port in 40100u16..41200u16,
    ) {
        let ch = AeronConfig::new()
            .with_media_type(MediaType::Udp)
            .use_multicast(
                Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: control_port,
                },
                Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: data_port,
                },
            )
            .with_control_mode(ControlMode::Dynamic)
            .build();
        let uri = ch.cstring().into_string().expect("cstring");
        prop_assert!(uri.contains("udp"), "uri: {uri}");
        prop_assert!(uri.contains("224.0.1.1"), "uri: {uri}");
        prop_assert!(uri.contains(&control_port.to_string()), "uri: {uri}");
        prop_assert!(uri.contains(&data_port.to_string()), "uri: {uri}");
    }

    /// Property: port_from_salt is deterministic and reflected in the URI.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn aeron_integration_uri_proptest_salt_port_range(salt in 0u16..4096u16) {
        let port = port_from_salt(salt);
        prop_assert_eq!(port, 40_456 + salt);
        let uri = channel_udp_p2p(port).cstring().into_string().expect("cstring");
        prop_assert!(uri.contains(&port.to_string()), "uri: {uri}");
    }

    /// Property: `cstring()` contains every required token for built channel shapes.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn aeron_integration_uri_proptest_cstring_required_tokens(
        port in 40100u16..41200u16,
        control_port in 40100u16..41200u16,
        reliable in proptest::bool::ANY,
        manual in proptest::bool::ANY,
        with_ttl in proptest::bool::ANY,
        with_term in proptest::bool::ANY,
        use_multicast in proptest::bool::ANY,
    ) {
        let channel = if use_multicast {
            let mut config = AeronConfig::new()
                .with_media_type(MediaType::Udp)
                .use_multicast(
                    Endpoint {
                        ip: "224.0.1.1".parse().expect("ip"),
                        port,
                    },
                    Endpoint {
                        ip: "224.0.1.1".parse().expect("ip"),
                        port: control_port,
                    },
                )
                .with_control_mode(if manual {
                    ControlMode::Manual
                } else {
                    ControlMode::Dynamic
                });
            if with_ttl {
                config = config.with_ttl(4);
            }
            if with_term {
                config = config.with_term_length(65_536);
            }
            config.build()
        } else {
            let mut config = AeronConfig::new()
                .with_media_type(MediaType::Udp)
                .use_point_to_point(Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port,
                })
                .with_reliability(if reliable {
                    ReliableConfig::Reliable
                } else {
                    ReliableConfig::Unreliable
                });
            if with_term {
                config = config.with_term_length(65_536);
            }
            config.build()
        };
        let uri = channel.cstring().into_string().expect("cstring");
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
    }

    /// Property: IPC and spy channel URIs include their media tokens.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn aeron_integration_uri_proptest_special_media_tokens(
        ipc in proptest::bool::ANY,
        spy in proptest::bool::ANY,
        with_term in proptest::bool::ANY,
    ) {
        let channel = if ipc {
            let mut config = AeronConfig::new().with_media_type(if spy {
                MediaType::SpyIpc
            } else {
                MediaType::Ipc
            }).use_ipc();
            if with_term {
                config = config.with_term_length(65_536);
            }
            config.build()
        } else {
            let port = 40_123u16;
            let mut config = AeronConfig::new()
                .with_media_type(if spy {
                    MediaType::SpyUdp
                } else {
                    MediaType::Udp
                })
                .use_point_to_point(Endpoint {
                    ip: "127.0.0.1".parse().expect("ip"),
                    port,
                });
            if with_term {
                config = config.with_term_length(65_536);
            }
            config.build()
        };
        let uri = channel.cstring().into_string().expect("cstring");
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
    }
}
