//! ChannelBuilder telemetry metric setters live here so channel_builder.rs stays under the 1,200-line budget.
//! Same inherent impl; callers keep using `ChannelBuilder::with_*` unchanged.

use super::*;

impl ChannelBuilder {
    /**
     * Enables monitoring of the average filled state.
     *
     * Tracks the average number of messages in the channel over time for telemetry reporting.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average filled state monitoring enabled.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_avg_filled(&self) -> Self {
        let mut result = self.clone();
        result.avg_filled = true;
        result
    }

    /**
     * Enables monitoring of the maximum filled state.
     *
     * Tracks the highest number of messages held, useful for peak usage analysis.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with maximum filled state monitoring enabled.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_max(&self) -> Self {
        let mut result = self.clone();
        result.max_filled = true;
        result
    }

    /// show the max latency
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_max(&self) -> Self {
        let mut result = self.clone();
        result.max_latency = true;
        result
    }

    /// show the max rate
    // ss[related channel.lazy.defer-allocation]
    pub fn with_rate_max(&self) -> Self {
        let mut result = self.clone();
        result.max_rate = true;
        result
    }

    /**
     * Enables monitoring of the minimum filled state.
     *
     * Tracks the lowest number of messages held, indicating low activity or bottlenecks.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with minimum filled state monitoring enabled.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_min(&self) -> Self {
        let mut result = self.clone();
        result.min_filled = true;
        result
    }

    /// show the min latency
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_min(&self) -> Self {
        let mut result = self.clone();
        result.min_latency = true;
        result
    }

    /// show the min rate
    // ss[related channel.lazy.defer-allocation]
    pub fn with_rate_min(&self) -> Self {
        let mut result = self.clone();
        result.min_rate = true;
        result
    }


    /**
     * Enables monitoring of the average message rate.
     *
     * Provides insight into typical throughput over time via telemetry.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average rate monitoring enabled.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_avg_rate(&self) -> Self {
        let mut result = self.clone();
        result.avg_rate = true;
        result
    }

    /**
     * Enables monitoring of the average message latency.
     *
     * Tracks the average time messages spend in the channel for performance tuning.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with average latency monitoring enabled.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_avg_latency(&self) -> Self {
        let mut result = self.clone();
        result.avg_latency = true;
        result
    }

    /**
     * Marks the channel as connecting to a sidecar process.
     *
     * Indicates special handling or routing via a sidecar for display purposes.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the sidecar flag set.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn connects_sidecar(&self) -> Self {
        let mut result = self.clone();
        result.connects_sidecar = true;
        result
    }

    /**
     * Sets labels for the channel and controls their display in telemetry.
     *
     * Labels aid in identification and categorization; display can be toggled.
     *
     * # Arguments
     *
     * - `labels`: Static slice of string slices representing labels.
     * - `display`: Boolean indicating whether to show labels in telemetry.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with configured labels.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_labels(&self, labels: &'static [&'static str], display: bool) -> Self {
        let mut result = self.clone();
        result.labels = if display { labels } else { &[] };
        result
    }

    /**
     * Adds a standard deviation metric for the filled state.
     *
     * Tracks variability in the channel’s filled state over time.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the filled state metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_filled.push(config);
        result
    }

    /**
     * Adds a standard deviation metric for the message rate.
     *
     * Tracks variability in message throughput over time.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the rate metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_rate_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_rate.push(config);
        result
    }

    /**
     * Adds a standard deviation metric for message latency.
     *
     * Tracks variability in message latency for performance consistency.
     *
     * # Arguments
     *
     * - `config`: `StdDev` configuration for the latency metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_standard_deviation(&self, config: StdDev) -> Self {
        let mut result = self.clone();
        result.std_dev_latency.push(config);
        result
    }

    /**
     * Adds a percentile metric for the filled state.
     *
     * Provides distribution insights into the channel’s filled state.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the filled state metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_filled.push(config);
        result
    }

    /**
     * Adds a percentile metric for the message rate.
     *
     * Provides distribution insights into message throughput.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the rate metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_rate_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_rate.push(config);
        result
    }

    /**
     * Adds a percentile metric for message latency.
     *
     * Ensures most messages meet latency requirements via distribution tracking.
     *
     * # Arguments
     *
     * - `config`: `Percentile` configuration for the latency metric.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the metric added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_percentile(&self, config: Percentile) -> Self {
        let mut result = self.clone();
        result.percentiles_latency.push(config);
        result
    }

    /**
     * Adds a trigger for the message rate with an alert color.
     *
     * Alerts when the rate crosses a threshold, aiding proactive monitoring.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Rate>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_rate_trigger(&self, bound: Trigger<Rate>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_rate.push((bound, color));
        result
    }

    /**
     * Adds a trigger for the filled state with an alert color.
     *
     * Alerts based on channel fullness, indicating backpressure or issues.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Filled>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_filled_trigger(&self, bound: Trigger<Filled>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_filled.push((bound, color));
        result
    }

    /**
     * Adds a trigger for message latency with an alert color.
     *
     * Alerts when latency exceeds a threshold, supporting performance SLAs.
     *
     * # Arguments
     *
     * - `bound`: `Trigger<Duration>` defining the alert condition.
     * - `color`: `AlertColor` to display when triggered.
     *
     * # Returns
     *
     * a new `ChannelBuilder` instance with the trigger added.
     */
    // ss[related channel.lazy.defer-allocation]
    pub fn with_latency_trigger(&self, bound: Trigger<Duration>, color: AlertColor) -> Self {
        let mut result = self.clone();
        result.trigger_latency.push((bound, color));
        result
    }
}
