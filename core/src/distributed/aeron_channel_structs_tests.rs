//! Tests extracted from `aeron_channel_structs.rs` so the production file stays under the 1,200-line budget.
//! Nested mods keep `super` access to private items in the parent module.

// ss[related distributed.aeron-uri]
// --- from mod aeron_channel_structs_tests ---
// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;
// ss[related distributed.aeron-uri]
use std::time::Duration;

// ss[related philosophy.structural-hierarchy]
fn uri_from_channel(channel: &Channel) -> String {
    channel.cstring().into_string().expect("cstring")
}

/// Extract `key=value` pairs from an Aeron channel URI query segment.
// ss[related distributed.aeron-uri]
fn uri_param_pairs(uri: &str) -> Vec<(&str, &str)> {
    let tail = if let Some((_, query)) = uri.split_once('?') {
        query
    } else if let Some(pos) = uri.find('|') {
        &uri[pos + 1..]
    } else {
        return Vec::new();
    };
    tail.split('|')
        .filter_map(|part| part.split_once('='))
        .collect()
}

// ss[related distributed.aeron-uri]
fn uri_param_value(uri: &str, key: &str) -> Option<String> {
    if key == "endpoint" {
        if let Some((_, query)) = uri.split_once('?') {
            if let Some(first) = query.split('|').next() {
                if let Some((k, v)) = first.split_once('=') {
                    if k == "endpoint" {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    uri_param_pairs(uri)
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

/// Parse the port from an `endpoint=host:port` token (IPv4 or bracketed IPv6).
// ss[related distributed.aeron-uri]
fn uri_endpoint_port(uri: &str) -> Option<u16> {
    let endpoint = uri_param_value(uri, "endpoint")?;
    if let Some(bracket_end) = endpoint.find(']') {
        endpoint[bracket_end + 1..]
            .trim_start_matches(':')
            .parse()
            .ok()
    } else {
        endpoint.rsplit(':').next()?.parse().ok()
    }
}

// ss[related distributed.aeron-uri]
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

#[test]
// ss[verify distributed.aeron-uri]
// ss[verify distributed.media-driver-testing]
fn test_media_driver_probe_error_display_and_hint() {
    let err = MediaDriverProbeError {
        last_failure: "CNC did not become ready".to_string(),
        elapsed: Duration::from_secs(2),
        aeron_dir: "/dev/shm/aeron-default".to_string(),
    };
    let display = format!("{err}");
    assert!(display.contains("CNC did not become ready"));
    assert!(display.contains("/dev/shm/aeron-default"));
    let hint = err.hint();
    assert!(hint.contains("aeronmd"));
    assert!(hint.contains("SS_AERON_REQUIRED"));
}

#[test]
// ss[verify distributed.media-driver-testing]
fn test_media_driver_probe_with_zero_wait_fails_fast() {
    let err = media_driver_probe_with_reason(Duration::ZERO).unwrap_err();
    assert!(!err.last_failure.is_empty());
    assert!(err.elapsed < Duration::from_millis(500));
}

#[test]
// ss[verify distributed.media-driver-testing]
fn test_media_driver_probe_bool_matches_result() {
    let with_reason = media_driver_probe_with_reason(Duration::ZERO).is_ok();
    let probe = media_driver_probe(Duration::ZERO);
    assert_eq!(probe, with_reason);
}

#[test]
// ss[verify distributed.aeron-uri]
fn test_channel_cstring_ipv4_udp_with_interface_and_term() {
    let channel = Channel::PointToPoint {
        media_type: MediaType::Udp,
        endpoint: Endpoint {
            ip: "127.0.0.1".parse().expect("ip"),
            port: 40123,
        },
        interface: Some(Endpoint {
            ip: "192.168.1.1".parse().expect("ip"),
            port: 0,
        }),
        reliability: Some(ReliableConfig::Unreliable),
        term_length: Some(65_536),
    };
    let uri = uri_from_channel(&channel);
    assert!(uri.contains("aeron:udp"));
    assert!(uri.contains("endpoint=127.0.0.1:40123"));
    assert!(uri.contains("interface=192.168.1.1:0"));
    assert!(uri.contains("reliable=false"));
    assert!(uri.contains("term-length=65536"));
}

#[test]
// ss[verify distributed.aeron-uri]
fn test_channel_cstring_ipv6_udp_endpoint() {
    let channel = Channel::PointToPoint {
        media_type: MediaType::Udp,
        endpoint: Endpoint {
            ip: "::1".parse().expect("ip"),
            port: 40456,
        },
        interface: None,
        reliability: None,
        term_length: None,
    };
    let uri = uri_from_channel(&channel);
    assert!(uri.contains("[::1]"));
    assert!(uri.contains("40456"));
}

#[test]
// ss[verify distributed.aeron-uri]
fn test_channel_cstring_spy_ipc_and_multicast_manual() {
    let spy = Channel::PointToPoint {
        media_type: MediaType::SpyIpc,
        endpoint: Endpoint {
            ip: "127.0.0.1".parse().expect("ip"),
            port: 0,
        },
        interface: None,
        reliability: None,
        term_length: None,
    };
    assert_eq!(uri_from_channel(&spy), "aeron-spy:aeron:ipc");

    let mcast = Channel::Multicast {
        media_type: MediaType::SpyUdp,
        endpoint: Endpoint {
            ip: "224.0.1.1".parse().expect("ip"),
            port: 40456,
        },
        config: MulticastConfig {
            control: Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: 40457,
            },
            ttl: Some(4),
        },
        control_mode: ControlMode::Manual,
        term_length: Some(1_048_576),
    };
    let uri = uri_from_channel(&mcast);
    assert!(uri.contains("aeron-spy:aeron:udp"));
    assert!(uri.contains("control-mode=manual"));
    assert!(uri.contains("ttl=4"));
    assert!(uri.contains("term-length=1048576"));
}

ss_proptest! {
    /// Property: direct `Channel` enum URIs include media, endpoint port, and optional tokens.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_channel_cstring_p2p_udp_tokens(
        port in crate::proptest_support::aeron_port(),
        reliable in any::<bool>(),
        with_interface in any::<bool>(),
        with_term in any::<bool>(),
        spy in any::<bool>(),
    ) {
        let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
        let channel = Channel::PointToPoint {
            media_type,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port,
            },
            interface: if with_interface {
                Some(Endpoint {
                    ip: "10.0.0.1".parse().expect("ip"),
                    port: 0,
                })
            } else {
                None
            },
            reliability: Some(if reliable {
                ReliableConfig::Reliable
            } else {
                ReliableConfig::Unreliable
            }),
            term_length: if with_term {
                Some(65_536)
            } else {
                None
            },
        };
        let uri = uri_from_channel(&channel);
        let prefix = if spy { "aeron-spy:aeron:udp" } else { "aeron:udp" };
        prop_assert!(uri.starts_with(prefix), "uri: {uri}");
        prop_assert!(uri.contains(&port.to_string()), "uri: {uri}");
        prop_assert!(
            uri.contains(if reliable { "reliable=true" } else { "reliable=false" }),
            "uri: {uri}"
        );
        if with_interface {
            prop_assert!(uri.contains("interface=10.0.0.1:0"), "uri: {uri}");
        }
        if with_term {
            prop_assert!(uri.contains("term-length=65536"), "uri: {uri}");
        }
    }

    /// Property: multicast `Channel` URIs include control endpoint and mode tokens.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_channel_cstring_multicast_tokens(
        data_port in crate::proptest_support::aeron_port(),
        control_port in crate::proptest_support::aeron_port(),
        manual in any::<bool>(),
        with_ttl in any::<bool>(),
        term in proptest::option::of(crate::proptest_support::aeron_term_length()),
    ) {
        let channel = Channel::Multicast {
            media_type: MediaType::Udp,
            endpoint: Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: data_port,
            },
            config: MulticastConfig {
                control: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: control_port,
                },
                ttl: if with_ttl { Some(8) } else { None },
            },
            control_mode: if manual {
                ControlMode::Manual
            } else {
                ControlMode::Dynamic
            },
            term_length: term,
        };
        let uri = uri_from_channel(&channel);
        prop_assert!(uri.contains("aeron:udp"), "uri: {uri}");
        prop_assert!(uri.contains(&data_port.to_string()), "uri: {uri}");
        prop_assert!(uri.contains(&control_port.to_string()), "uri: {uri}");
        prop_assert!(
            uri.contains(if manual { "control-mode=manual" } else { "control-mode=dynamic" }),
            "uri: {uri}"
        );
        if with_ttl {
            prop_assert!(uri.contains("ttl=8"), "uri: {uri}");
        }
        if let Some(t) = term {
            prop_assert!(uri.contains(&format!("term-length={t}")), "uri: {uri}");
        }
    }

    /// Property: IPC channel URIs include optional term-length.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_channel_cstring_ipc_term(
        spy in any::<bool>(),
        with_term in any::<bool>(),
        term in proptest::option::of(crate::proptest_support::aeron_term_length()),
    ) {
        let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
        let channel = Channel::PointToPoint {
            media_type,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port: 0,
            },
            interface: None,
            reliability: None,
            term_length: if with_term { term } else { None },
        };
        let uri = uri_from_channel(&channel);
        let prefix = if spy { "aeron-spy:aeron:ipc" } else { "aeron:ipc" };
        prop_assert!(uri.starts_with(prefix), "uri: {uri}");
        if let Some(t) = term {
            if with_term {
                prop_assert!(uri.contains(&format!("term-length={t}")), "uri: {uri}");
            }
        } else if !with_term {
            prop_assert!(!uri.contains("term-length="), "uri: {uri}");
        }
    }

    /// Property: `cstring()` round-trips endpoint port and optional query params through URI parse.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_uri_parse_roundtrip_p2p_udp(
        port in crate::proptest_support::aeron_port(),
        reliable in any::<bool>(),
        with_interface in any::<bool>(),
        with_term in any::<bool>(),
        spy in any::<bool>(),
    ) {
        let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
        let term_length = if with_term { Some(65_536) } else { None };
        let channel = Channel::PointToPoint {
            media_type,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port,
            },
            interface: if with_interface {
                Some(Endpoint {
                    ip: "10.0.0.1".parse().expect("ip"),
                    port: 0,
                })
            } else {
                None
            },
            reliability: Some(if reliable {
                ReliableConfig::Reliable
            } else {
                ReliableConfig::Unreliable
            }),
            term_length,
        };
        let uri = uri_from_channel(&channel);
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
        prop_assert_eq!(uri_endpoint_port(&uri), Some(port));
        let reliable_param = uri_param_value(&uri, "reliable");
        prop_assert_eq!(
            reliable_param.as_deref(),
            Some(if reliable { "true" } else { "false" })
        );
        if with_interface {
            prop_assert!(uri.contains("interface=10.0.0.1:0"), "uri: {uri}");
        }
        if with_term {
            let term_param = uri_param_value(&uri, "term-length");
            prop_assert_eq!(term_param.as_deref(), Some("65536"));
        }
    }

    /// Property: multicast URI parse round-trips data/control ports and control-mode.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_uri_parse_roundtrip_multicast(
        data_port in crate::proptest_support::aeron_port(),
        control_port in crate::proptest_support::aeron_port(),
        manual in any::<bool>(),
        with_ttl in any::<bool>(),
        spy in any::<bool>(),
        term in proptest::option::of(crate::proptest_support::aeron_term_length()),
    ) {
        let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
        let channel = Channel::Multicast {
            media_type,
            endpoint: Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: data_port,
            },
            config: MulticastConfig {
                control: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: control_port,
                },
                ttl: if with_ttl { Some(8) } else { None },
            },
            control_mode: if manual {
                ControlMode::Manual
            } else {
                ControlMode::Dynamic
            },
            term_length: term,
        };
        let uri = uri_from_channel(&channel);
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
        prop_assert_eq!(uri_endpoint_port(&uri), Some(data_port));
        prop_assert!(
            uri.contains(&control_port.to_string()),
            "control port missing from uri: {uri}"
        );
        let mode_parsed = uri_param_value(&uri, "control-mode");
        prop_assert_eq!(
            mode_parsed.as_deref(),
            Some(if manual { "manual" } else { "dynamic" })
        );
        if with_ttl {
            let ttl_parsed = uri_param_value(&uri, "ttl");
            prop_assert_eq!(ttl_parsed.as_deref(), Some("8"));
        }
        if let Some(t) = term {
            let term_expected = t.to_string();
            let term_parsed = uri_param_value(&uri, "term-length");
            prop_assert_eq!(term_parsed.as_deref(), Some(term_expected.as_str()));
        }
    }

    /// Property: IPC and spy-IPC URIs round-trip optional term-length through parse.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_uri_parse_roundtrip_ipc(
        spy in any::<bool>(),
        term in proptest::option::of(crate::proptest_support::aeron_term_length()),
    ) {
        let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
        let channel = Channel::PointToPoint {
            media_type,
            endpoint: Endpoint {
                ip: "127.0.0.1".parse().expect("ip"),
                port: 0,
            },
            interface: None,
            reliability: None,
            term_length: term,
        };
        let uri = uri_from_channel(&channel);
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
        if let Some(t) = term {
            let term_expected = t.to_string();
            let term_parsed = uri_param_value(&uri, "term-length");
            prop_assert_eq!(term_parsed.as_deref(), Some(term_expected.as_str()));
        } else {
            prop_assert!(!uri.contains("term-length="), "uri: {uri}");
        }
    }

    /// Property: IPv6 UDP endpoint port round-trips through URI parse.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_uri_parse_roundtrip_ipv6_udp(
        port in crate::proptest_support::aeron_port(),
        reliable in any::<bool>(),
        spy in any::<bool>(),
    ) {
        let media_type = if spy { MediaType::SpyUdp } else { MediaType::Udp };
        let channel = Channel::PointToPoint {
            media_type,
            endpoint: Endpoint {
                ip: "::1".parse().expect("ip"),
                port,
            },
            interface: None,
            reliability: Some(if reliable {
                ReliableConfig::Reliable
            } else {
                ReliableConfig::Unreliable
            }),
            term_length: None,
        };
        let uri = uri_from_channel(&channel);
        prop_assert!(uri.contains("[::1]"), "uri: {uri}");
        prop_assert_eq!(uri_endpoint_port(&uri), Some(port));
        for token in required_uri_tokens(&channel) {
            prop_assert!(uri.contains(&token), "missing token '{token}' in uri: {uri}");
        }
    }

    /// Property: multicast spy-IPC and IPC media URIs round-trip control tokens.
    #[test]
    // ss[verify distributed.aeron-uri]
    // ss[verify verify.process.proptest]
    fn proptest_uri_parse_roundtrip_multicast_ipc_media(
        data_port in crate::proptest_support::aeron_port(),
        control_port in crate::proptest_support::aeron_port(),
        spy in any::<bool>(),
        manual in any::<bool>(),
    ) {
        let media_type = if spy { MediaType::SpyIpc } else { MediaType::Ipc };
        let channel = Channel::Multicast {
            media_type,
            endpoint: Endpoint {
                ip: "224.0.1.1".parse().expect("ip"),
                port: data_port,
            },
            config: MulticastConfig {
                control: Endpoint {
                    ip: "224.0.1.1".parse().expect("ip"),
                    port: control_port,
                },
                ttl: None,
            },
            control_mode: if manual {
                ControlMode::Manual
            } else {
                ControlMode::Dynamic
            },
            term_length: None,
        };
        let uri = uri_from_channel(&channel);
        let expected_prefix = if spy { "aeron-spy:aeron:ipc" } else { "aeron:ipc" };
        prop_assert!(uri.contains(expected_prefix), "uri: {uri}");
        prop_assert_eq!(uri_endpoint_port(&uri), Some(data_port));
        prop_assert!(uri.contains(&control_port.to_string()), "uri: {uri}");
        let mode_parsed = uri_param_value(&uri, "control-mode");
        prop_assert_eq!(
            mode_parsed.as_deref(),
            Some(if manual { "manual" } else { "dynamic" })
        );
    }
}

