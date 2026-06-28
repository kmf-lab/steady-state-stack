// ss[related telemetry.dot-export]
use bytes::{BufMut, BytesMut};

use super::DotState;

/// Builds the Prometheus metrics from the current state.
///
/// # Arguments
///
/// * `state` - THE current metric state.
/// * `txt_metric` - THE buffer to store the metrics text.
pub(crate) fn build_metric(state: &DotState, txt_metric: &mut BytesMut) {
    txt_metric.clear(); // Clear the buffer for reuse

    state
        .nodes
        .iter()
        .filter(|n| n.id.is_some())
        .for_each(|node| {
            txt_metric.put_slice(node.metric_text.as_bytes());
        });

    state
        .edges
        .iter()
        .filter(|e| e.id != usize::MAX)
        .for_each(|edge| {
            txt_metric.put_slice(edge.metric_text.as_bytes());
        });
}
