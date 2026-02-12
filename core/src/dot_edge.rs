use crate::ActorName;
use crate::channel_stats::ChannelStatsComputer;

/// Represents an edge in the graph, including metrics and display information.
#[derive(Debug)]
pub(crate) struct Edge {
    pub(crate) id: usize, // Position matches the channel ID
    pub(crate) from: Option<ActorName>,
    pub(crate) to: Option<ActorName>,
    pub(crate) color: &'static str, // Results from computer
    pub(crate) sidecar: bool, // Mark this edge as attaching to a sidecar
    pub(crate) pen_width: String, // Results from computer
    pub(crate) saturation_score: f64, // Results from computer
    pub(crate) ctl_labels: Vec<&'static str>, // Visibility tags for render
    pub(crate) stats_computer: ChannelStatsComputer,
    pub(crate) display_label: String, // Results from computer
    pub(crate) metric_text: String, // Results from computer
    pub(crate) partner: Option<&'static str>,
    pub(crate) bundle_index: Option<usize>,
}

/// Checks if a color string is recognized by the DOT renderer.
/// This prevents "black on black" rendering issues caused by unrecognized color names.
fn is_recognized_color(color: &str) -> bool {
    matches!(
        color,
        "red" | "green" | "blue" | "grey" | "gray" | "yellow" | "purple" | "white"
    )
}

impl Edge {
    /// Computes and refreshes the metrics for the edge based on send and take values.
    ///
    /// # Arguments
    ///
    /// * `send` - The send value.
    /// * `take` - The take value.
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
