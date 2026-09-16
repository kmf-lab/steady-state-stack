//! Property tests for telemetry parse / diagram apply / report generation.

use super::*;
use crate::monitor::{ActorMetaData, ActorStatus, ChannelMetaData};
use crate::ss_proptest;
use crate::ActorIdentity;
use bytes::Bytes;
use parking_lot::RwLock;
use proptest::prelude::*;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;

fn empty_frames() -> DotGraphFrames {
    DotGraphFrames {
        active_metric: BytesMut::new(),
        active_graph: BytesMut::new(),
        config_line: String::with_capacity(160),
        dot_scratch: String::with_capacity(512),
        hex_line: String::with_capacity(16),
        lane_color_counts: std::collections::BTreeMap::new(),
        last_generated_graph: Instant::now(),
    }
}

fn empty_metric_state() -> Arc<RwLock<MetricState>> {
    Arc::new(RwLock::new(MetricState {
        doc: Bytes::new(),
        metric: Bytes::new(),
        config: Bytes::from_static(b"{}"),
        last_disk_write: Arc::new(AtomicU64::new(0)),
        start_time: Instant::now(),
    }))
}

fn node_def(id: usize, name: &'static str, ch_out: Option<usize>) -> DiagramData {
    let outs: Box<[Arc<ChannelMetaData>]> = match ch_out {
        Some(cid) => Box::new([Arc::new(ChannelMetaData {
            id: cid,
            ..Default::default()
        })]),
        None => Box::new([]),
    };
    DiagramData::NodeDef(
        1,
        Box::new((
            Arc::new(ActorMetaData {
                ident: ActorIdentity::new(id, name, None),
                ..Default::default()
            }),
            Box::new([]),
            outs,
        )),
    )
}

ss_proptest! {
    /// Property: `{host}:{port}` with a u16 port parses; missing/invalid port is None.
    #[test]
    // ss[verify telemetry.builtin-server]
    // ss[verify verify.process.proptest]
    fn proptest_parse_host_port(
        host in "[A-Za-z0-9._-]{1,24}",
        port in any::<u16>(),
        garbage in "[A-Za-z]{1,8}",
    ) {
        let ok = format!("{}:{}", host, port);
        prop_assert_eq!(parse_host_port(&ok), Some((host.clone(), port)));
        prop_assert!(parse_host_port(&host).is_none(), "no colon");
        let bad_port = format!("{}:{}", host, garbage);
        prop_assert!(parse_host_port(&bad_port).is_none());
        let overflow = format!("{}:70000", host);
        prop_assert!(parse_host_port(&overflow).is_none());
    }

    /// Property: NodeDef / process / subtitle / volume apply without panic.
    #[test]
    // ss[verify telemetry.builtin-server]
    // ss[verify verify.process.proptest]
    fn proptest_process_msg_diagram_arms(
        actor_id in 0usize..6,
        seq in 0u64..32,
        await_ns in 0u64..1_000,
        extra in 1u64..2_000,
        take in 0i64..64,
        send in 0i64..64,
        subtitle in prop::option::of("[A-Za-z0-9 ]{0,12}"),
        unknown_edge in 20usize..40,
    ) {
        crate::core_exec::block_on(async {
            let mut state = DotState::default();
            state.refresh_rate_ms = 200;
            let mut history = FrameHistory::new(200);
            process_msg(node_def(actor_id, "unit", Some(0)), &mut state, &mut history, 200).await;
            prop_assert!(state.nodes.len() > actor_id);
            prop_assert_eq!(state.seq, 1);

            let status = ActorStatus {
                ident: ActorIdentity::new(actor_id, "unit", None),
                await_total_ns: await_ns,
                unit_total_ns: await_ns.saturating_add(extra),
                ..Default::default()
            };
            process_msg(
                DiagramData::NodeProcessData(seq, Box::new([status])),
                &mut state,
                &mut history,
                200,
            )
            .await;
            process_msg(
                DiagramData::NodeProcessData(
                    seq,
                    Box::new([ActorStatus {
                        ident: ActorIdentity::new(99, "missing", None),
                        await_total_ns: 0,
                        unit_total_ns: 1,
                        ..Default::default()
                    }]),
                ),
                &mut state,
                &mut history,
                200,
            )
            .await;

            process_msg(
                DiagramData::NodeDotSubtitle(
                    seq,
                    Box::new([(actor_id, subtitle.clone()), (99, None)]),
                ),
                &mut state,
                &mut history,
                200,
            )
            .await;
            if let Some(node) = state.nodes.get(actor_id) {
                prop_assert_eq!(&node.dot_subtitle, &subtitle);
            }

            process_msg(
                DiagramData::ChannelVolumeData(
                    seq,
                    Box::new([(0, take, send), (unknown_edge, take, send)]),
                ),
                &mut state,
                &mut history,
                200,
            )
            .await;
            prop_assert_eq!(state.seq, seq);
            Ok(())
        })?;
    }

    /// Property: generate_reports flush/no-flush and color presence swap MetricState bytes.
    #[test]
    // ss[verify telemetry.builtin-server]
    // ss[verify verify.process.proptest]
    fn proptest_generate_reports_flush_and_colors(
        flush_frame in any::<bool>(),
        flush_all in any::<bool>(),
        use_custom_colors in any::<bool>(),
    ) {
        crate::core_exec::block_on(async {
            let mut metrics_state = DotState::default();
            metrics_state.refresh_rate_ms = 100;
            if use_custom_colors {
                metrics_state.telemetry_colors =
                    Some(("#111111".to_string(), "#222222".to_string()));
            }
            let mut history = FrameHistory::new(100);
            let mut frames = empty_frames();
            let shared = empty_metric_state();
            generate_reports(
                &mut metrics_state,
                &mut history,
                &mut frames,
                flush_all,
                shared.clone(),
                flush_frame,
            )
            .await;
            if flush_frame {
                let g = shared.read();
                let cfg = String::from_utf8_lossy(&g.config);
                if use_custom_colors {
                    prop_assert!(cfg.contains("#111111"));
                } else {
                    prop_assert!(cfg.contains("#00a900"));
                }
                prop_assert!(cfg.contains("\"refresh_rate_ms\": 100"));
            }
            Ok(())
        })?;
    }
}
