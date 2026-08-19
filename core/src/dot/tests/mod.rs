// DOT export tests split by concern.

// ss[related philosophy.structural-hierarchy]
use super::*;
// ss[impl telemetry.dot-export]
use bytes::BytesMut;
// ss[impl telemetry.dot-export]
use std::time::Instant;

// ss[related philosophy.structural-hierarchy]
mod format_tests;
// ss[impl telemetry.dot-export]
mod build_tests;
// ss[impl telemetry.dot-export]
mod history_tests;
// ss[related philosophy.structural-hierarchy]
mod register_tests;
// ss[impl telemetry.dot-export]
mod format_proptest;
// ss[impl telemetry.dot-export]
mod build_proptest;
// ss[related philosophy.structural-hierarchy]
mod history_proptest;

// ss[related telemetry.dot-export]
pub(super) fn test_dot_frames() -> DotGraphFrames {
    DotGraphFrames {
        active_metric: BytesMut::new(),
        active_graph: BytesMut::new(),
        config_line: String::new(),
        dot_scratch: String::new(),
        hex_line: String::new(),
        lane_color_counts: std::collections::BTreeMap::new(),
        last_generated_graph: Instant::now(),
    }
}
