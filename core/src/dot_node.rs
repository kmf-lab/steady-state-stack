// ss[related telemetry.dot-export]
use crate::ActorName;
// ss[related philosophy.structural-hierarchy]
use crate::actor_stats::ActorStatsComputer;
// ss[related philosophy.structural-hierarchy]
use crate::dot::RemoteDetails;
// ss[related telemetry.dot-export]
use crate::monitor::{ActorStatus, ThreadInfo};

/// Represents a node in the graph, including metrics and display information.
// ss[related telemetry.dot-export]
pub(crate) struct Node {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) id: Option<ActorName>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) remote_details: Option<RemoteDetails>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) color: &'static str,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) pen_width: &'static str,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) stats_computer: ActorStatsComputer,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) display_label: String,
    /// Raw (unescaped) optional subtitle line under the actor name in DOT labels only.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) dot_subtitle: Option<String>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) tooltip: String,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) metric_text: String,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) thread_info_cache: Option<ThreadInfo>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) total_count_restarts: u32,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) bool_stalled: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) last_bool_stop: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) work_info: Option<(u16, u16)>,
}

/// Graph-share load percent from this actor's mCPU and the summed graph mCPU.
// ss[related telemetry.dot-export]
pub(crate) fn graph_share_load(mcpu: u16, total_mcpu: u128) -> u16 {
    if total_mcpu == 0 {
        0
    } else {
        ((100u128 * mcpu as u128) / total_mcpu).min(100) as u16
    }
}

// ss[related telemetry.dot-export]
impl Node {
    /// Applies local mCPU from telemetry status; load share is filled by [`Self::apply_graph_load_and_emit`].
    ///
    /// **Avg mCPU** is local busy fraction: `(unit - await) / unit` scaled to 0..1024.
    // ss[related telemetry.dot-export]
    pub(crate) fn apply_local_mcpu(&mut self, actor_status: ActorStatus) -> bool {
        let num = actor_status.await_total_ns;
        let den = actor_status.unit_total_ns;

        let updated = if den == 0 {
            false
        } else {
            assert!(den.ge(&num), "num: {} den: {}", num, den);
            let busy = den - num;
            let mcpu: u16 = ((1024u128 * busy as u128) / den as u128).min(1024) as u16;
            let prior_load = self.work_info.map(|(_, load)| load).unwrap_or(0);
            self.work_info = Some((mcpu, prior_load));
            true
        };

        if actor_status.thread_info.is_some() {
            self.thread_info_cache = actor_status.thread_info;
        }
        self.total_count_restarts = self
            .total_count_restarts
            .max(actor_status.total_count_restarts);
        self.bool_stalled = actor_status.is_quiet;
        self.last_bool_stop = actor_status.bool_stop;
        updated
    }

    /// Sets graph-share **Avg load %** and refreshes DOT / Prometheus labels.
    ///
    /// **Avg load %** is `100 × this_mcpu / Σ last-known graph mCPU` (hotspot share), not local CPU utilization.
    // ss[related telemetry.dot-export]
    pub(crate) fn apply_graph_load_and_emit(&mut self, total_mcpu: u128, accumulate: bool) {
        if let Some((mcpu, _)) = self.work_info {
            let load = graph_share_load(mcpu, total_mcpu);
            self.work_info = Some((mcpu, load));
        }
        let mcpu_load = self.work_info;
        let thread_id = if self.stats_computer.show_thread_id {
            self.thread_info_cache
        } else {
            None
        };

        let (color, pen_width) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.tooltip,
            &mut self.metric_text,
            mcpu_load,
            self.total_count_restarts,
            self.last_bool_stop,
            self.bool_stalled,
            thread_id,
            self.dot_subtitle.as_deref(),
            accumulate,
        );

        self.color = color;
        self.pen_width = pen_width;
    }

    /// Single-node refresh (tests): local mCPU then load share against this node only (100% when alone).
    // ss[related telemetry.dot-export]
    pub(crate) fn compute_and_refresh(&mut self, actor_status: ActorStatus) {
        self.apply_local_mcpu(actor_status);
        let total = self
            .work_info
            .map(|(mcpu, _)| mcpu as u128)
            .unwrap_or(0);
        self.apply_graph_load_and_emit(total, true);
    }
}
