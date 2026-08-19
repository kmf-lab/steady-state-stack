// ss[related telemetry.dot-export]
use crate::ActorName;
// ss[related philosophy.structural-hierarchy]
use crate::channel_stats::ChannelStatsComputer;

/// Represents an edge in the graph, including metrics and display information.
///
/// Diagnostic fields (`diag_*`) capture the **first claimant** numeric actor id + `Arc<ChannelMetaData>` pointer
/// for each endpoint so WARN lines can correlate duplicate `channels_out` / `channels_in`.
#[derive(Default, Debug)]
// ss[related telemetry.dot-export]
pub(crate) struct Edge {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) id: usize, // Position matches the channel ID
    // ss[related philosophy.structural-hierarchy]
    pub(crate) from: Option<ActorName>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) to: Option<ActorName>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) color: &'static str, // Results from computer
    // ss[related philosophy.structural-hierarchy]
    pub(crate) sidecar: bool, // Mark this edge as attaching to a sidecar
    // ss[related philosophy.structural-hierarchy]
    pub(crate) pen_width: String, // Results from computer
    // ss[related philosophy.structural-hierarchy]
    pub(crate) saturation_score: f64, // Results from computer
    // ss[related philosophy.structural-hierarchy]
    pub(crate) ctl_labels: Vec<&'static str>, // Visibility tags for render
    // ss[related philosophy.structural-hierarchy]
    pub(crate) stats_computer: ChannelStatsComputer,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) display_label: String, // Results from computer
    // ss[related philosophy.structural-hierarchy]
    pub(crate) metric_text: String, // Results from computer
    // ss[related philosophy.structural-hierarchy]
    pub(crate) partner: Option<&'static str>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) bundle_index: Option<usize>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) diag_from_claim_actor_id: Option<usize>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) diag_from_claim_meta_arc: Option<usize>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) diag_to_claim_actor_id: Option<usize>,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) diag_to_claim_meta_arc: Option<usize>,
}

/// Checks if a color string is recognized by the DOT renderer.
/// This prevents "black on black" rendering issues caused by unrecognized color names.
// ss[related telemetry.dot-export]
fn is_recognized_color(color: &str) -> bool {
    matches!(
        color,
        "red" | "green" | "blue" | "grey" | "gray" | "yellow" | "purple" | "white"
    )
}

// ss[related telemetry.dot-export]
impl Edge {
    /// Computes and refreshes the metrics for the edge based on send and take values.
    ///
    /// # Arguments
    ///
    /// * `send` - The send value.
    /// * `take` - The take value.
    // ss[related telemetry.dot-export]
    pub(crate) fn compute_and_refresh(&mut self, send: i64, take: i64) {
        let (color, _pen) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.metric_text,
            self.from,
            send,
            take,
        );

        //this is different from the actors in that sending and take are totaled up
        // ie they get accumulated and eld by self.status_computer for rollovers.

        // CRITICAL: Handle color updates safely.
        // 1. If the stats computer returns "black" (often used for idle/off),
        //    treat it as "grey" to ensure visibility on black backgrounds.
        // 2. Only update if the resulting color is recognized.
        //    This prevents "black on black" rendering issues caused by unrecognized color strings.
        let effective_color = if color == "black" { "grey" } else { color };

        if is_recognized_color(effective_color) {
            self.color = effective_color;
        }
        
        self.saturation_score = self.stats_computer.saturation_score;
    }
}
