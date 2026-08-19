// ss[related telemetry.dot-export]
use std::collections::BTreeMap;
// ss[impl telemetry.dot-export]
use std::time::Instant;

// ss[related telemetry.dot-export]
use bytes::BytesMut;

/// Working buffers for DOT + Prometheus + config JSON (reused each telemetry frame).
// ss[related telemetry.dot-export]
pub struct DotGraphFrames {
    // ss[impl telemetry.dot-export]
    pub(crate) active_metric: BytesMut,
    // ss[impl telemetry.dot-export]
    pub(crate) active_graph: BytesMut,
    /// Small JSON payload built without per-frame `String` allocation on the hot path.
    // ss[impl telemetry.dot-export]
    pub(crate) config_line: String,
    /// DOT escapes, rollups, histogram text (used sequentially; not nested).
    // ss[impl telemetry.dot-export]
    pub(crate) dot_scratch: String,
    /// `#RRGGBB` for the current edge render (separate from [`DotGraphFrames::dot_scratch`]).
    // ss[impl telemetry.dot-export]
    pub(crate) hex_line: String,
    // ss[impl telemetry.dot-export]
    pub(crate) lane_color_counts: BTreeMap<&'static str, usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) last_generated_graph: Instant,
}
