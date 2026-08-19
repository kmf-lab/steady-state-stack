// ss[related telemetry.dot-export]
use crate::ActorName;

// ss[impl telemetry.dot-export]
pub(crate) struct PartneredEdge {
    // ss[impl telemetry.dot-export]
    pub(crate) from: Option<ActorName>,
    // ss[impl telemetry.dot-export]
    pub(crate) to: Option<ActorName>,
    // ss[impl telemetry.dot-export]
    pub(crate) lane_rgbs: Vec<(u32, u32, u32)>,
    /// Resolved edge colors (after triggers), one per lane — for tooltip histograms.
    // ss[impl telemetry.dot-export]
    pub(crate) lane_colors: Vec<&'static str>,
    /// Whole-percent avg fill per lane (`None` if disabled or no window sample).
    // ss[impl telemetry.dot-export]
    pub(crate) avg_fill_per_lane: Vec<Option<u8>>,
    // ss[impl telemetry.dot-export]
    pub(crate) show_avg_filled_any: bool,
    // ss[impl telemetry.dot-export]
    pub(crate) summary_label: String,
    // ss[impl telemetry.dot-export]
    pub(crate) combined_type: String,
    // ss[impl telemetry.dot-export]
    pub(crate) partner_name: Option<&'static str>,
    // ss[impl telemetry.dot-export]
    pub(crate) sub_capacities: Vec<usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) sidecar: bool,
    // ss[impl telemetry.dot-export]
    pub(crate) saturation_score: f64,
    // ss[impl telemetry.dot-export]
    pub(crate) tooltip: String,
    // ss[impl telemetry.dot-export]
    pub(crate) sub_totals: Vec<u128>,
    // ss[impl telemetry.dot-export]
    pub(crate) ids: Vec<usize>,
    // ss[impl telemetry.dot-export]
    pub(crate) ctl_labels: Vec<&'static str>,
    // ss[impl telemetry.dot-export]
    pub(crate) pen_width: String,
    // ss[impl telemetry.dot-export]
    pub(crate) ring_memory_footprint: usize,
    // ss[impl telemetry.dot-export]
    pub(crate) dynamic_memory_footprint: usize,
    // ss[impl telemetry.dot-export]
    pub(crate) show_memory: bool,
    // ss[impl telemetry.dot-export]
    pub(crate) show_total: bool,
}
