// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use crate::telemetry::metrics_server::async_write_all;
// ss[impl telemetry.dot-export]
use bytes::BytesMut;

#[test]
// ss[verify telemetry.dot-export]
fn test_mean_avg_fill_percent_all_none_returns_none_without_panic() {
    let all_none = [None::<u8>, None];
    assert_eq!(mean_avg_fill_percent(all_none.iter()), None);
    let empty: [Option<u8>; 0] = [];
    assert_eq!(mean_avg_fill_percent(empty.iter()), None);
}

#[test]
// ss[verify telemetry.dot-export]
fn test_build_metric() {
    let state = DotState {
        nodes: vec![Node {
            id: Some(ActorName::new("1", None)),
            color: "grey",
            pen_width: NODE_PEN_WIDTH,
            stats_computer: ActorStatsComputer::default(),
            display_label: String::new(),
            dot_subtitle: None,
            tooltip: String::new(),
            metric_text: "node_metric".to_string(),
            remote_details: None,
            thread_info_cache: None,
            total_count_restarts: 0,
            bool_stalled: false,
            work_info: None,
        }],
        edges: vec![Edge {
            id: 1,
            from: None,
            to: None,
            color: "grey",
            sidecar: false,
            pen_width: EDGE_PEN_WIDTH.to_string(),
            saturation_score: 0.0,
            ctl_labels: Vec::new(),
            stats_computer: ChannelStatsComputer::default(),
            display_label: "edge_metric".to_string(),
            metric_text: "edge_metric".to_string(),
            partner: None,
            bundle_index: None,
        ..Default::default()
        }],
        seq: 0,
        telemetry_colors: None,
        refresh_rate_ms: 0,
        bundle_floor_size: 4,
    };
    let mut txt_metric = BytesMut::new();
    build_metric(&state, &mut txt_metric);
}
