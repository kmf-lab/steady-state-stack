// ss[related telemetry.dot-export]
use super::super::*;
// ss[impl telemetry.dot-export]
use super::test_dot_frames;
// ss[impl telemetry.dot-export]
use crate::dot_unify::ChannelEdgeRole;
// ss[related telemetry.dot-export]
use crate::monitor::{ActorIdentity, ActorMetaData, ActorStatus, ChannelMetaData};
// ss[impl telemetry.dot-export]
use crate::telemetry::metrics_server::async_write_all;
// ss[impl telemetry.dot-export]
use bytes::BytesMut;
// ss[related telemetry.dot-export]
use std::fs::remove_file;
// ss[impl telemetry.dot-export]
use std::path::PathBuf;
// ss[impl telemetry.dot-export]
use std::sync::Arc;
// ss[related telemetry.dot-export]
use std::time::Instant;

#[test]
// ss[verify telemetry.dot-export]
fn test_define_unified_edges() {
    let mut metric_state = DotState::default();
    let actor = crate::graph_liveliness::ActorIdentity::new(991, "node1", None);
    let channels = vec![Arc::new(ChannelMetaData::default())];

    define_unified_edges(
        &mut metric_state,
        actor,
        &channels,
        ChannelEdgeRole::SetsEdgeTo,
        1000,
    );
    assert_eq!(metric_state.edges.len(), 1);
    assert!(metric_state.edges[0].to.is_some());
}


#[test]
// ss[verify telemetry.dot-export]
fn test_apply_node_def() {
    // Test lines 305-342 - apply_node_def function
    let mut local_state = DotState::default();

    let actor = Arc::new(ActorMetaData {
        ident: ActorIdentity {
            id: 0,
            label: ActorName::new("test_actor", Some(1)),
        },
        remote_details: Some(RemoteDetails {
            ips: "127.0.0.1".to_string(),
            match_on: "test_match".to_string(),
            tech: "HTTP",
            direction: "out",
        }),
        avg_mcpu: false,
        avg_work: false,
        show_thread_info: false,
        percentiles_mcpu: vec![],
        percentiles_work: vec![],
        std_dev_mcpu: vec![],
        std_dev_work: vec![],
        trigger_mcpu: vec![],
        trigger_work: vec![],
        refresh_rate_in_bits: 0,
        window_bucket_in_bits: 0,
        usage_review: false,
    });

    let channel_in = Arc::new(ChannelMetaData {
        id: 0,
        labels: vec!["input_label"],
        capacity: 0,
        display_labels: false,
        line_expansion: 0.0,
        show_type: None,
        refresh_rate_in_bits: 0,
        window_bucket_in_bits: 0,
        percentiles_filled: vec![],
        percentiles_rate: vec![],
        percentiles_latency: vec![],
        std_dev_inflight: vec![],
        std_dev_consumed: vec![],
        std_dev_latency: vec![],
        trigger_rate: vec![],
        trigger_filled: vec![],
        trigger_latency: vec![],
        avg_filled: false,
        avg_rate: false,
        avg_latency: false,
        min_filled: false,
        max_filled: false,
        min_rate: false,
        max_rate: false,
        min_latency: false,
        max_latency: false,
        connects_sidecar: false,
        partner: None,
        bundle_index: None,
        type_byte_count: 0,
        show_total: false,
        girth: 1,
        show_memory: false,
        ring_slot_byte_count: 0,
        dynamic_per_slot_estimate: None,
    });

    let channel_out = Arc::new(ChannelMetaData {
        id: 1,
        labels: vec!["output_label"],
        capacity: 0,
        display_labels: false,
        line_expansion: 0.0,
        show_type: None,
        refresh_rate_in_bits: 0,
        window_bucket_in_bits: 0,
        percentiles_filled: vec![],
        percentiles_rate: vec![],
        percentiles_latency: vec![],
        std_dev_inflight: vec![],
        std_dev_consumed: vec![],
        std_dev_latency: vec![],
        trigger_rate: vec![],
        trigger_filled: vec![],
        trigger_latency: vec![],
        avg_filled: false,
        avg_rate: false,
        avg_latency: false,
        min_filled: false,
        max_filled: false,
        min_rate: false,
        max_rate: false,
        min_latency: false,
        max_latency: false,
        connects_sidecar: true,
        partner: None,
        bundle_index: None,
        type_byte_count: 0,
        show_total: false,
        girth: 1,
        show_memory: false,
        ring_slot_byte_count: 0,
        dynamic_per_slot_estimate: None,
    });

    let channels_in = vec![channel_in];
    let channels_out = vec![channel_out];

    apply_node_def(&mut local_state, actor, &channels_in, &channels_out, 1000);

    assert_eq!(local_state.nodes.len(), 1);
    assert!(local_state.nodes[0].id.is_some());
    assert_eq!(
        local_state.nodes[0].id.expect("internal error").name,
        "test_actor"
    );
    assert!(local_state.nodes[0].remote_details.is_some());
    assert_eq!(local_state.edges.len(), 2); // One for input, one for output
}

