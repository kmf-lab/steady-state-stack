// DOT export tests split by concern.

use super::*;
use bytes::BytesMut;
use std::time::Instant;

mod format_tests;
mod build_tests;
mod history_tests;
mod register_tests;
mod format_proptest;
mod build_proptest;
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
