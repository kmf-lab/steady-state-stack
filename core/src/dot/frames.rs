// ss[related telemetry.dot-export]
use std::collections::BTreeMap;
use std::time::Instant;

use bytes::BytesMut;

/// Working buffers for DOT + Prometheus + config JSON (reused each telemetry frame).
pub struct DotGraphFrames {
    pub(crate) active_metric: BytesMut,
    pub(crate) active_graph: BytesMut,
    /// Small JSON payload built without per-frame `String` allocation on the hot path.
    pub(crate) config_line: String,
    /// DOT escapes, rollups, histogram text (used sequentially; not nested).
    pub(crate) dot_scratch: String,
    /// `#RRGGBB` for the current edge render (separate from [`DotGraphFrames::dot_scratch`]).
    pub(crate) hex_line: String,
    pub(crate) lane_color_counts: BTreeMap<&'static str, usize>,
    pub(crate) last_generated_graph: Instant,
}
