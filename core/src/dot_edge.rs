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
}

impl Edge {
    /// Computes and refreshes the metrics for the edge based on send and take values.
    ///
    /// # Arguments
    ///
    /// * `send` - The send value.
    /// * `take` - The take value.
    pub(crate) fn compute_and_refresh(&mut self, send: i64, take: i64) {
        let (color, pen) = self.stats_computer.compute(
            &mut self.display_label,
            &mut self.metric_text,
            self.from,
            send,
            take,
        );

        //this is different from the actors in that sending and take are totaled up
        // ie they get accumulated and eld by self.status_computer for rollovers.

        self.color = color;
        self.pen_width = pen;
        self.saturation_score = self.stats_computer.saturation_score;
    }
}
