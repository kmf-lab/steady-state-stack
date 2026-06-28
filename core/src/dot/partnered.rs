// ss[related telemetry.dot-export]
use crate::ActorName;

pub(crate) struct PartneredEdge {
    pub(crate) from: Option<ActorName>,
    pub(crate) to: Option<ActorName>,
    pub(crate) lane_rgbs: Vec<(u32, u32, u32)>,
    /// Resolved edge colors (after triggers), one per lane — for tooltip histograms.
    pub(crate) lane_colors: Vec<&'static str>,
    /// Whole-percent avg fill per lane (`None` if disabled or no window sample).
    pub(crate) avg_fill_per_lane: Vec<Option<u8>>,
    pub(crate) show_avg_filled_any: bool,
    pub(crate) summary_label: String,
    pub(crate) combined_type: String,
    pub(crate) partner_name: Option<&'static str>,
    pub(crate) sub_capacities: Vec<usize>,
    pub(crate) sidecar: bool,
    pub(crate) saturation_score: f64,
    pub(crate) tooltip: String,
    pub(crate) sub_totals: Vec<u128>,
    pub(crate) ids: Vec<usize>,
    pub(crate) ctl_labels: Vec<&'static str>,
    pub(crate) pen_width: String,
    pub(crate) memory_footprint: usize,
    pub(crate) show_memory: bool,
    pub(crate) show_total: bool,
}
