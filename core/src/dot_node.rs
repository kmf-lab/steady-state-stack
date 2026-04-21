use crate::ActorName;
use crate::actor_stats::ActorStatsComputer;
use crate::dot::RemoteDetails;
use crate::monitor::{ActorStatus, ThreadInfo};

/// Represents a node in the graph, including metrics and display information.
pub(crate) struct Node {
    pub(crate) id: Option<ActorName>,
    pub(crate) remote_details: Option<RemoteDetails>,
    pub(crate) color: &'static str,
    pub(crate) pen_width: &'static str,
    pub(crate) stats_computer: ActorStatsComputer,
    pub(crate) display_label: String,
    /// Raw (unescaped) optional subtitle line under the actor name in DOT labels only.
    pub(crate) dot_subtitle: Option<String>,
    pub(crate) tooltip: String,
    pub(crate) metric_text: String,
    pub(crate) thread_info_cache: Option<ThreadInfo>,
    pub(crate) total_count_restarts: u32,
    pub(crate) bool_stalled: bool,
    pub(crate) work_info: Option<(u16, u16)>,
}

impl Node {
    /// Computes and refreshes the metrics for the node based on the actor status.
    ///
    /// **Avg load %** uses the same *local* busy ratio as **mCPU**: fraction of this actor's
    /// `unit_total_ns` not attributed to instrumented profile time (`await_total_ns` from
    /// telemetry; see `FinallyRollupProfileGuard`), i.e. `(unit - await) / unit`, scaled to 0..100.
    /// It does **not** divide by summed busy time across other actors.
    ///
    /// # Arguments
    ///
    /// * `actor_status` - The status of the actor.
    pub(crate) fn compute_and_refresh(&mut self, actor_status: ActorStatus) {
        let num = actor_status.await_total_ns;
        let den = actor_status.unit_total_ns;

        let mcpu_load = if den == 0 {
            None
        } else {
            assert!(den.ge(&num), "num: {} den: {}", num, den);
            let busy = den - num;
            // mCPU/load from busy fraction. `num == 0` means no instrumented time in the window
            // → fully busy (1024 mCPU), not zero. Integer division: (busy×1024)/den may differ by 1
            // from `1024 - (num×1024)/den` in edge cases.
            let mcpu: u16 = ((1024u128 * busy as u128) / den as u128).min(1024) as u16;
            let load: u16 = ((100u64 * busy as u64) / den as u64).min(100) as u16;
            Some((mcpu, load))
        };

        //if we have no new work data then continue what we found last time
        if mcpu_load.is_some() {
            self.work_info = mcpu_load;
        }
        let mcpu_load = self.work_info;

        //only set when we get a new one otherwise we just hold the old one.
        if actor_status.thread_info.is_some() {
            self.thread_info_cache = actor_status.thread_info;
        }
        let thread_id = if self.stats_computer.show_thread_id {
            self.thread_info_cache
        } else {
            None
        };
        self.total_count_restarts = self
            .total_count_restarts
            .max(actor_status.total_count_restarts);
        self.bool_stalled = actor_status.is_quiet;

        // Old strings for this actor are passed back in so they get cleared and re-used rather than reallocate
        let (color, pen_width) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.tooltip,
            &mut self.metric_text,
            mcpu_load,
            self.total_count_restarts,
            actor_status.bool_stop,
            actor_status.is_quiet,
            thread_id,
            self.dot_subtitle.as_deref(),
        );

        self.color = color;
        self.pen_width = pen_width;
    }
}
