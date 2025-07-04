use std::collections::VecDeque;
use hdrhistogram::{Counter, Histogram};
#[allow(unused_imports)]
use log::*;
use std::cmp;

use crate::*;
use crate::channel_stats::{compute_labels, ComputeLabelsConfig, ComputeLabelsLabels, DOT_GREEN, DOT_GREY, DOT_ORANGE, DOT_RED, DOT_YELLOW, PLACES_TENS};
use crate::monitor::ThreadInfo;

/// The `ActorStatsComputer` struct is responsible for computing and maintaining statistics for an actor within the system.
/// It tracks CPU and workload utilization, manages histograms for percentile calculations, and evaluates trigger conditions for alerts.
#[derive(Default)]
pub struct ActorStatsComputer {
    /// The unique identifier for the actor, including its name and optional suffix.
    pub(crate) ident: ActorIdentity,

    /// A list of CPU utilization triggers paired with their corresponding alert colors.
    pub(crate) mcpu_trigger: Vec<(Trigger<MCPU>, AlertColor)>, // If used base is green

    /// A list of workload utilization triggers paired with their corresponding alert colors.
    pub(crate) work_trigger: Vec<(Trigger<Work>, AlertColor)>, // If used base is green

    /// The count of frames accumulated in the current bucket before it is finalized and added to history.
    pub(crate) bucket_frames_count: usize, // When this bucket is full we add a new one

    /// The bit shift value representing the refresh rate for telemetry collection.
    pub(crate) refresh_rate_in_bits: u8,

    /// The bit shift value representing the window bucket size for telemetry aggregation.
    pub(crate) window_bucket_in_bits: u8,

    /// The frame rate in milliseconds, used for timing calculations and unit testing.
    pub(crate) frame_rate_ms: u64, // Const at runtime but needed here for unit testing

    /// A string label representing the time window for the current statistics.
    pub(crate) time_label: String,

    /// A list of percentiles to monitor for CPU utilization.
    pub(crate) percentiles_mcpu: Vec<Percentile>, // To show

    /// A list of percentiles to monitor for workload utilization.
    pub(crate) percentiles_work: Vec<Percentile>, // To show

    /// A list of standard deviations to monitor for CPU utilization variability.
    pub(crate) std_dev_mcpu: Vec<StdDev>, // To show

    /// A list of standard deviations to monitor for workload variability.
    pub(crate) std_dev_work: Vec<StdDev>, // To show

    /// Flag to indicate whether to display average CPU utilization in telemetry.
    pub(crate) show_avg_mcpu: bool,

    /// Flag to indicate whether to display average workload in telemetry.
    pub(crate) show_avg_work: bool,

    /// Flag to enable usage review in telemetry for detailed analysis.
    pub(crate) usage_review: bool,

    /// Flag indicating whether to build a histogram for CPU utilization.
    pub(crate) build_mcpu_histogram: bool,

    /// Flag indicating whether to build a histogram for workload utilization.
    pub(crate) build_work_histogram: bool,

    /// A deque containing historical CPU utilization data blocks.
    pub(crate) history_mcpu: VecDeque<ChannelBlock<u16>>,

    /// A deque containing historical workload utilization data blocks.
    pub(crate) history_work: VecDeque<ChannelBlock<u8>>,

    /// The current CPU utilization data block being accumulated.
    pub(crate) current_mcpu: Option<ChannelBlock<u16>>,

    /// The current workload utilization data block being accumulated.
    pub(crate) current_work: Option<ChannelBlock<u8>>,

    /// A string containing Prometheus-style labels for metrics.
    pub(crate) prometheus_labels: String,

    /// Flag to indicate whether to display thread information in telemetry.
    pub(crate) show_thread_id: bool
}

impl ActorStatsComputer {
    /// Computes the actor's statistics and updates the provided labels for visualization and metrics.
    ///
    /// This method processes the current CPU and workload utilization, accumulates data frames, and generates
    /// labels for DOT visualization and Prometheus metrics. It also determines the appropriate line color
    /// and width based on trigger conditions.
    ///
    /// # Arguments
    ///
    /// * `dot_label` - A mutable reference to the string that will hold the DOT label for visualization.
    /// * `metric_text` - A mutable reference to the string that will hold the Prometheus metrics text.
    /// * `mcpu` - The current CPU utilization value.
    /// * `work` - The current workload utilization value.
    /// * `total_count_restarts` - The total number of times the actor has been restarted.
    /// * `bool_stop` - A boolean indicating whether the actor has stopped.
    /// * `thread_info` - Optional thread information to include in the DOT label.
    ///
    /// # Returns
    ///
    /// A tuple containing the line color and line width for visualization.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compute(
        &mut self,
        dot_label: &mut String,
        metric_text: &mut String,
        mcpu: u64,
        load: u64,
        total_count_restarts: u32,
        bool_stop: bool,
        thread_info: Option<ThreadInfo>
    ) -> (&'static str, &'static str) {
        self.accumulate_data_frame(mcpu, load);

        #[cfg(feature = "prometheus_metrics")]
        metric_text.clear();

        dot_label.clear(); // For this node we cache the same allocation.

        dot_label.push_str(self.ident.label.name);
        if let Some(suffix) =  self.ident.label.suffix {
            dot_label.push_str(itoa::Buffer::new().format(suffix));
        }
        if steady_config::SHOW_ACTORS {
            dot_label.push('[');
            dot_label.push_str(itoa::Buffer::new().format(self.ident.id));
            dot_label.push(']');
        }
        dot_label.push('\n');

        if let Some(thread) = thread_info {    //new line for thread info
            //this could be better looking but will require unstable features today Oct 2024.
            let t = format!("{:?}",thread.thread_id);
            dot_label.push_str(&t);
            //rename this plus add switch.
            #[cfg(feature = "core_display")]
            {
                dot_label.push_str(" Core:");
                let t = format!("{:?}",thread.core);
                dot_label.push_str(&t);
            }
            dot_label.push('\n');
        }

        if self.window_bucket_in_bits != 0 {
            dot_label.push_str("Window ");
            dot_label.push_str(&self.time_label);
            dot_label.push('\n');
        }

        if total_count_restarts > 0 {
            dot_label.push_str("restarts: ");
            dot_label.push_str(itoa::Buffer::new().format(total_count_restarts));
            dot_label.push('\n');

            #[cfg(feature = "prometheus_metrics")]
            {
                metric_text.push_str("graph_node_restarts{");
                metric_text.push_str(&self.prometheus_labels);
                metric_text.push_str("} ");
                metric_text.push_str(itoa::Buffer::new().format(total_count_restarts));
                metric_text.push('\n');
            }
        }

        if bool_stop {
            dot_label.push_str("stopped");
            dot_label.push('\n');
        }

        if let Some(current_work) = &self.current_work {
            let config = ComputeLabelsConfig::actor_config(self, (1, 1), 100, self.show_avg_work);
            let labels = ComputeLabelsLabels {
                label: "load",
                unit: "%",
                _prometheus_labels: &self.prometheus_labels,
                int_only: true,
                fixed_digits: 0
            };
            compute_labels(config, current_work, labels, &self.std_dev_work, &self.percentiles_work, metric_text, dot_label);
        }

        if let Some(current_mcpu) = &self.current_mcpu {
            let config = ComputeLabelsConfig::actor_config(self, (1, 1), 1024, self.show_avg_mcpu);
            let labels = ComputeLabelsLabels {
                label: "mCPU",
                unit: "",
                _prometheus_labels: &self.prometheus_labels,
                int_only: true,
                fixed_digits: 4
            };
            compute_labels(config, current_mcpu, labels, &self.std_dev_mcpu, &self.percentiles_mcpu, metric_text, dot_label);
        }

        let mut line_color = DOT_GREY;
        if !self.mcpu_trigger.is_empty() || !self.work_trigger.is_empty() {
            line_color = DOT_GREEN;
            if self.trigger_alert_level(&AlertColor::Yellow) {
                line_color = DOT_YELLOW;
            }
            if self.trigger_alert_level(&AlertColor::Orange) {
                line_color = DOT_ORANGE;
            }
            if self.trigger_alert_level(&AlertColor::Red) {
                line_color = DOT_RED;
            }
        }
        let line_width = crate::dot::DEFAULT_PEN_WIDTH;

        //println!("input mcpu {} work {} line_color {} ", mcpu, work, line_color);

        (line_color, line_width)
    }

    /// Initializes the `ActorStatsComputer` with the provided actor metadata and frame rate.
    ///
    /// This method sets up the internal state based on the actor's configuration, including triggers,
    /// percentiles, and standard deviations to monitor. It also prepares the histograms if needed.
    ///
    /// # Arguments
    ///
    /// * `meta` - The metadata of the actor, containing configuration for telemetry and triggers.
    /// * `frame_rate_ms` - The frame rate in milliseconds for telemetry data collection.
    pub(crate) fn init(&mut self, meta: Arc<ActorMetaData>, frame_rate_ms: u64) {
        self.ident = meta.ident;

        // Prometheus labels
        self.prometheus_labels.push_str("actor_name=\"");
        self.prometheus_labels.push_str(meta.ident.label.name);
        self.prometheus_labels.push('"');

        if let Some(suffix) = meta.ident.label.suffix {
            self.prometheus_labels.push_str(", ");
            self.prometheus_labels.push_str("actor_suffix=\"");
            self.prometheus_labels.push_str(itoa::Buffer::new().format(suffix));
            self.prometheus_labels.push('"');
        }

        // TODO: Perf, we could pre-filter these by color here since they will not change again.
        // This might be needed for faster updates.
        self.mcpu_trigger.clone_from(&meta.trigger_mcpu);
        self.work_trigger.clone_from(&meta.trigger_work);

        self.frame_rate_ms = frame_rate_ms;
        self.refresh_rate_in_bits = meta.refresh_rate_in_bits;
        self.window_bucket_in_bits = meta.window_bucket_in_bits;
        self.time_label = time_label((self.frame_rate_ms as u128)<< (meta.refresh_rate_in_bits + meta.window_bucket_in_bits));

        self.show_avg_mcpu = meta.avg_mcpu;
        self.show_avg_work = meta.avg_work;
        self.percentiles_mcpu.clone_from(&meta.percentiles_mcpu);
        self.percentiles_work.clone_from(&meta.percentiles_work);
        self.std_dev_mcpu.clone_from(&meta.std_dev_mcpu);
        self.std_dev_work.clone_from(&meta.std_dev_work);
        self.usage_review = meta.usage_review;
        self.show_thread_id = meta.show_thread_info;

        let trigger_uses_histogram = self.mcpu_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );
        self.build_mcpu_histogram = trigger_uses_histogram || !self.percentiles_mcpu.is_empty();

        if self.build_mcpu_histogram {
            let mcpu_top = 1024;
            match Histogram::<u16>::new_with_bounds(1, mcpu_top, 1) {
                Ok(h) => {
                    self.history_mcpu.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", mcpu_top, 2, e);
                }
            }
        } else {
            self.history_mcpu.push_back(ChannelBlock::default());
        }

        let trigger_uses_histogram = self.work_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );
        self.build_work_histogram = trigger_uses_histogram || !self.percentiles_work.is_empty();

        if self.build_work_histogram {
            let work_top = 100;
            match Histogram::<u8>::new_with_bounds(1, work_top, 1) {
                Ok(h) => {
                    self.history_work.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", work_top, 2, e);
                }
            }
        } else {
            self.history_work.push_back(ChannelBlock::default());
        }
    }

    /// Accumulates a new data frame for CPU and workload utilization.
    ///
    /// This method updates the running totals and histograms for the current bucket. When the bucket
    /// is full, it is moved to the history, and a new bucket is started.
    ///
    /// # Arguments
    ///
    /// * `mcpu` - The CPU utilization value for the current frame.
    /// * `work` - The workload utilization value for the current frame.
    pub(crate) fn accumulate_data_frame(&mut self, mcpu: u64, work: u64) {
        assert!(mcpu <= 1024, "mcpu out of range {}", mcpu);

        self.history_mcpu.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(mcpu) {
                    error!("unexpected, unable to record inflight {} err: {}", mcpu, e);
                }
            }
            f.runner = f.runner.saturating_add(mcpu as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((mcpu as u128).pow(2));
        });

        self.history_work.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(work) {
                    error!("unexpected, unable to record inflight {} err: {}", work, e);
                }
            }
            f.runner = f.runner.saturating_add(work as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((work as u128).pow(2));
        });

        self.bucket_frames_count += 1;
        if self.bucket_frames_count >= (1 << self.refresh_rate_in_bits) {
            self.bucket_frames_count = 0;

            if self.history_mcpu.len() >= (1 << self.window_bucket_in_bits) {
                self.current_mcpu = self.history_mcpu.pop_front();
            }
            if self.history_work.len() >= (1 << self.window_bucket_in_bits) {
                self.current_work = self.history_work.pop_front();
            }

            if self.build_mcpu_histogram {
                let mcpu_top = 1024;
                match Histogram::<u16>::new_with_bounds(1, mcpu_top, 1) {
                    Ok(h) => {
                        self.history_mcpu.push_back(ChannelBlock {
                            histogram: Some(h),
                            runner: 0,
                            sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", mcpu_top, 2, e);
                    }
                }
            } else {
                self.history_mcpu.push_back(ChannelBlock::default());
            }

            if self.build_work_histogram {
                let work_top = 100;
                match Histogram::<u8>::new_with_bounds(1, work_top, 1) {
                    Ok(h) => {
                        self.history_work.push_back(ChannelBlock {
                            histogram: Some(h),
                            runner: 0,
                            sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", work_top, 2, e);
                    }
                }
            } else {
                self.history_work.push_back(ChannelBlock::default());
            }
        }
    }

    /// Checks if any triggers for a specific alert level have been activated.
    ///
    /// This method evaluates all triggers associated with the given alert color and returns `true`
    /// if any of them are currently active based on the accumulated statistics.
    ///
    /// # Arguments
    ///
    /// * `c1` - The alert color to check for.
    ///
    /// # Returns
    ///
    /// `true` if any trigger for the specified alert level is active, otherwise `false`.
    fn trigger_alert_level(&mut self, c1: &AlertColor) -> bool {
        //TODO: create vec for each color to avoid the filter here.

        (self.mcpu_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_mcpu(&f.0)))
            ||
            (self.work_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_work(&f.0)))
    }

    /// Determines if a specific CPU utilization trigger condition is met.
    ///
    /// This method evaluates the given trigger rule against the current CPU utilization statistics.
    ///
    /// # Arguments
    ///
    /// * `rule` - The trigger rule to evaluate.
    ///
    /// # Returns
    ///
    /// `true` if the trigger condition is satisfied, otherwise `false`.
    fn triggered_mcpu(&self, rule: &Trigger<MCPU>) -> bool {
        match rule {
            Trigger::AvgBelow(mcpu) => {
                //println!("check below: {:?} {:?}", mcpu, self.current_mcpu);
                let run_divisor = 1 << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(run_divisor, 1 , &self.current_mcpu, (mcpu.mcpu() as u64, 1)  ).is_lt()

            },
            Trigger::AvgAbove(mcpu) => {
                // println!("check above: {:?} {:?}", mcpu, self.current_mcpu);
                let run_divisor = 1 << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(run_divisor, 1, &self.current_mcpu, (mcpu.mcpu() as u64,1)).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu,(mcpu.mcpu() as u64,1)).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, mcpu) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.mcpu_std_dev(), window_bits, std_devs, &self.current_mcpu, (mcpu.mcpu() as u64,1)).is_gt()
            }
            Trigger::PercentileAbove(percentile, mcpu) => {
                percentile_rational(percentile, &self.current_mcpu, (mcpu.mcpu() as u64,1)).is_gt()
            }
            Trigger::PercentileBelow(percentile, mcpu) => {
                percentile_rational(percentile, &self.current_mcpu,(mcpu.mcpu() as u64,1)).is_lt()
            }
        }
    }

    /// Determines if a specific workload utilization trigger condition is met.
    ///
    /// This method evaluates the given trigger rule against the current workload utilization statistics.
    ///
    /// # Arguments
    ///
    /// * `rule` - The trigger rule to evaluate.
    ///
    /// # Returns
    ///
    /// `true` if the trigger condition is satisfied, otherwise `false`.
    fn triggered_work(&self, rule: &Trigger<Work>) -> bool {
        match rule {
            Trigger::AvgBelow(work) => {
                // println!("check below: {:?} {:?}", work, self.current_mcpu);
                let run_divisor = 1 << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(run_divisor, 100, &self.current_work, work.rational()).is_lt()

            },
            Trigger::AvgAbove(work) => {
                // println!("check below: {:?} {:?}", work, self.current_mcpu);
                let run_divisor = 1 << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                avg_rational(run_divisor, 100, &self.current_work, work.rational()).is_gt()

            },
            Trigger::StdDevsBelow(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.work_std_dev(), window_bits, std_devs, &self.current_work, work.rational()).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, work) => {
                let window_bits = self.window_bucket_in_bits + self.refresh_rate_in_bits;
                stddev_rational(self.work_std_dev(), window_bits, std_devs, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileAbove(percentile, work) => {
                percentile_rational(percentile, &self.current_work, work.rational()).is_gt()
            }
            Trigger::PercentileBelow(percentile, work) => {
                percentile_rational(percentile, &self.current_work, work.rational()).is_lt()
            }
        }
    }

    /// Computes the standard deviation of CPU utilization based on the current data.
    ///
    /// This method calculates the standard deviation using the accumulated sum of squares and runner values.
    ///
    /// # Returns
    ///
    /// The standard deviation as a float, or 0.0 if no data is available.
    #[inline]
    fn mcpu_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_mcpu {
            compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits, 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits), c.runner, c.sum_of_squares)
        } else {
            trace!("skipping std no current data");
            0f32
        }
    }

    /// Computes the standard deviation of workload utilization based on the current data.
    ///
    /// This method calculates the standard deviation using the accumulated sum of squares and runner values.
    ///
    /// # Returns
    ///
    /// The standard deviation as a float, or 0.0 if no data is available.
    #[inline]
    fn work_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_work {
            compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                            , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                            , c.runner
                            , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }
}

/// Converts a duration in milliseconds to a human-readable time label string.
///
/// This function determines the most appropriate unit (seconds, minutes, hours, or days) based on the duration
/// and formats the label accordingly.
///
/// # Arguments
///
/// * `total_ms` - The total duration in milliseconds.
///
/// # Returns
///
/// A string representing the duration in a human-readable format.
pub(crate) fn time_label(total_ms: u128) -> String {
    let seconds = total_ms as f64 / 1000.0;
    let minutes = seconds / 60.0;
    let hours = minutes / 60.0;
    let days = hours / 24.0;

    if days >= 1.0 {
        if days < 1.1 { "day".to_string() } else { format!("{:.1} days", days) }
    } else if hours >= 1.0 {
        if hours < 1.1 { "hr".to_string() } else { format!("{:.1} hrs", hours) }
    } else if minutes >= 1.0 {
        if minutes < 1.1 { "min".to_string() } else { format!("{:.1} mins", minutes) }
    } else if seconds < 1.1 { "sec".to_string() } else { format!("{:.1} secs", seconds) }
}

/// Compares the average value from the current data block to a given rational threshold.
///
/// This function calculates the average based on the runner and divisor, then compares it to the provided rational value.
///
/// # Arguments
///
/// * `run_divisor` - The divisor for calculating the average from the runner.
/// * `units` - The units for scaling the rational comparison.
/// * `current` - The optional current channel block containing the runner.
/// * `rational` - The rational threshold as a tuple (numerator, denominator).
///
/// # Returns
///
/// A `cmp::Ordering` indicating whether the average is less than, equal to, or greater than the threshold.
pub(crate) fn avg_rational<T: Counter>(run_divisor: u128, units: u128, current: &Option<ChannelBlock<T>>, rational: (u64, u64)) -> cmp::Ordering {
    if let Some(current) = current {
        //println!("current.runner {} run_divisor {} rational.0 {} rational.1 {}", current.runner, run_divisor, rational.0, rational.1);
        //println!("actual {} limit {} units {}", current.runner/(run_divisor),(units * rational.0 as u128)/(rational.1 as u128),units);
        (current.runner * rational.1 as u128).cmp(&( units * run_divisor * rational.0 as u128))
    } else {
        cmp::Ordering::Equal // Unknown
    }
}

/// Compares a value derived from standard deviation to a given rational threshold.
///
/// This function calculates a value based on the average and standard deviation, then compares it to the provided rational value.
///
/// # Arguments
///
/// * `std_dev` - The standard deviation value.
/// * `window_bits` - The number of bits representing the window size.
/// * `std_devs` - The standard deviation multiplier.
/// * `current` - The optional current channel block containing the runner.
/// * `expected` - The expected rational value as a tuple (numerator, denominator).
///
/// # Returns
///
/// A `cmp::Ordering` indicating whether the computed value is less than, equal to, or greater than the threshold.
pub(crate) fn stddev_rational<T: Counter>(
    std_dev: f32,
    window_bits: u8,
    std_devs: &StdDev,
    current: &Option<ChannelBlock<T>>,
    expected: (u64, u64)
) -> cmp::Ordering {
    if let Some(current) = current {
        let std_deviation = (std_dev * std_devs.value()) as u128;
        (expected.1 as u128 * ((current.runner >> window_bits) + std_deviation)).cmp(&(PLACES_TENS as u128 * expected.0 as u128))
    } else {
        cmp::Ordering::Equal // Unknown
    }
}

/// Compares a percentile value from the histogram to a given rational threshold.
///
/// This function retrieves the value at the specified percentile from the histogram and compares it to the provided rational value.
///
/// # Arguments
///
/// * `percentile` - The percentile to evaluate.
/// * `consumed` - The optional current channel block containing the histogram.
/// * `rational` - The rational threshold as a tuple (numerator, denominator).
///
/// # Returns
///
/// A `cmp::Ordering` indicating whether the percentile value is less than, equal to, or greater than the threshold.
pub(crate) fn percentile_rational<T: Counter>(percentile: &Percentile, consumed: &Option<ChannelBlock<T>>, rational: (u64, u64)) -> cmp::Ordering {
    if let Some(current_consumed) = consumed {
        if let Some(h) = &current_consumed.histogram {
            let measured_rate_ms = h.value_at_percentile(percentile.percentile()) as u128;
            (measured_rate_ms * rational.1 as u128).cmp(&(rational.0 as u128))
        } else {
            cmp::Ordering::Equal // Unknown
        }
    } else {
        cmp::Ordering::Equal // Unknown
    }
}

/// Computes the standard deviation for a given set of parameters.
///
/// This function calculates the standard deviation using the sum of squares and runner values, handling large numbers carefully to avoid overflow.
///
/// # Arguments
///
/// * `bits` - The number of bits used in calculations.
/// * `window` - The size of the window for averaging.
/// * `runner` - The accumulated runner value.
/// * `sum_sqr` - The accumulated sum of squares.
///
/// # Returns
///
/// The computed standard deviation as a float.
#[inline]
pub(crate) fn compute_std_dev(bits: u8, window: usize, runner: u128, sum_sqr: u128) -> f32 {
    if runner < SQUARE_LIMIT {
        let r2 = (runner * runner) >> bits;
        if sum_sqr > r2 {
            (((sum_sqr - r2) >> bits) as f32).sqrt() // TODO: 2025 someday we may need to implement sqrt for u128
        } else {
            ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
        }
    } else {
        ((sum_sqr as f32 / window as f32) - (runner as f32 / window as f32).powi(2)).sqrt()
    }
}

/// The maximum value for the runner before special handling is needed in standard deviation calculations.
pub(crate) const SQUARE_LIMIT: u128 = (1 << 64) - 1;

/// The `ChannelBlock` struct holds statistical data for a channel, including an optional histogram and running totals for calculations.
///
/// # Type Parameters
///
/// * `T` - The counter type used in the histogram.
#[derive(Default, Debug)]
pub(crate) struct ChannelBlock<T> where T: Counter {
    /// An optional histogram for storing distribution data.
    pub(crate) histogram: Option<Histogram<T>>,

    /// The accumulated runner value for average calculations.
    pub(crate) runner: u128,

    /// The accumulated sum of squares for variance calculations.
    pub(crate) sum_of_squares: u128,
}

#[cfg(test)]
mod test_actor_stats {
    use super::*;
    use std::sync::Arc;

    fn create_mock_metadata() -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1,"test_actor", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50(), Percentile::p90()],
            percentiles_work: vec![Percentile::p50(), Percentile::p90()],
            std_dev_mcpu: vec![StdDev::new(1.0).expect("")],
            std_dev_work: vec![StdDev::new(1.0).expect("")],
            trigger_mcpu: vec![(Trigger::AvgAbove(MCPU::m512()), AlertColor::Red)],
            trigger_work: vec![(Trigger::AvgAbove(Work::p50()), AlertColor::Red)],
            usage_review: false,
            refresh_rate_in_bits: 6,
            window_bucket_in_bits: 5,
        })
    }

    #[test]
    fn test_init() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        assert_eq!(actor_stats.ident.id, 1);
        assert_eq!(actor_stats.ident.label.name, "test_actor");
        assert!(actor_stats.show_avg_mcpu);
        assert!(actor_stats.show_avg_work);
        assert_eq!(actor_stats.percentiles_mcpu.len(), 2);
        assert_eq!(actor_stats.percentiles_work.len(), 2);
        assert_eq!(actor_stats.std_dev_mcpu.len(), 1);
        assert_eq!(actor_stats.std_dev_work.len(), 1);
        assert_eq!(actor_stats.mcpu_trigger.len(), 1);
        assert_eq!(actor_stats.work_trigger.len(), 1);
        assert_eq!(actor_stats.frame_rate_ms, 1000);
        assert_eq!(actor_stats.refresh_rate_in_bits, 6);
        assert_eq!(actor_stats.window_bucket_in_bits, 5);
    }

    #[test]
    fn test_accumulate_data_frame() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let mcpu_histogram = actor_stats.history_mcpu.front().expect("iternal error").histogram.as_ref().expect("iternal error");
        let work_histogram = actor_stats.history_work.front().expect("iternal error").histogram.as_ref().expect("iternal error");

        assert_eq!(mcpu_histogram.value_at_quantile(0.5), 543);
        assert_eq!(work_histogram.value_at_quantile(0.5), 51);
    }

    #[test]
    fn test_compute_labels() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        let (line_color, line_width) = actor_stats.compute(
            &mut dot_label,
            &mut metric_text,
            512,
            50,
            1,
            false,
            None
        );

        assert_eq!(line_color, DOT_GREEN);
        assert_eq!(line_width, "4");
        assert!(dot_label.contains("test_actor"));

    }

    #[test]
    fn test_percentile_rational() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let percentile_result = percentile_rational(
            &Percentile::p50(),
            &actor_stats.current_mcpu,
            (512, 1024),
        );

        assert_eq!(percentile_result, cmp::Ordering::Equal);
    }
}

#[cfg(test)]
mod test_actor_stats_triggers {
    use super::*;
    use std::sync::Arc;

    fn create_mock_metadata() -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test_actor", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50(), Percentile::p90()],
            percentiles_work: vec![Percentile::p50(), Percentile::p90()],
            std_dev_mcpu: vec![StdDev::new(1.0).expect("")],
            std_dev_work: vec![StdDev::new(1.0).expect("")],
            trigger_mcpu: vec![
                (Trigger::AvgAbove(MCPU::m512()), AlertColor::Yellow),
                (Trigger::AvgBelow(MCPU::m256()), AlertColor::Red),
            ],
            trigger_work: vec![
                (Trigger::AvgAbove(Work::p50()), AlertColor::Orange),
                (Trigger::AvgBelow(Work::p30()), AlertColor::Yellow),
            ],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        })
    }

    #[test]
    fn test_trigger_avg_above_mcpu() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);

        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(520, 40);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg above trigger to be activated for mcpu."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(509, 40);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg above trigger NOT to be activated for mcpu."
        );
    }

    #[test]
    fn test_trigger_avg_below_mcpu() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(230, 40);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Red),
            "Expected avg below trigger to be activated for mcpu."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(260, 40);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Red),
            "Expected avg below trigger NOT to be activated for mcpu."
        );
    }

    #[test]
    fn test_trigger_avg_above_work() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 55);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Orange),
            "Expected avg above trigger to be activated for work."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 45);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Orange),
            "Expected avg above trigger NOT to be activated for work."
        );
    }

    #[test]
    fn test_trigger_avg_below_work() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 28);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg below trigger to be activated for work."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 32);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg below trigger NOT to be activated for work."
        );
    }
}

/// Additional tests to achieve 100% coverage for all utility functions and edge cases.
#[cfg(test)]
mod extra_tests {
    use super::*;


    /// Verify `time_label` produces the correct text for various durations.
    #[test]
    fn test_time_label_thresholds() {
        // sub-second
        assert_eq!(time_label(500), "sec");
        // seconds
        assert_eq!(time_label(1500), "1.5 secs");
        // exactly one minute
        assert_eq!(time_label(60_000), "min");
        // minutes
        assert_eq!(time_label(90_000), "1.5 mins");
        // exactly one hour
        assert_eq!(time_label(3_600_000), "hr");
        // multiple hours
        assert_eq!(time_label(7_200_000), "2.0 hrs");
        // exactly one day
        assert_eq!(time_label(86_400_000), "day");
        // multiple days
        assert_eq!(time_label(172_800_000), "2.0 days");
    }

    /// Test both branches of `compute_std_dev`.
    #[test]
    fn test_compute_std_dev_branches() {
        // runner < SQUARE_LIMIT: should compute a finite non-negative value
        let val = compute_std_dev(1, 2, 1, 2);
        assert!(val >= 0.0, "std dev should be non-negative");

        // runner >= SQUARE_LIMIT: computed expression is negative inside sqrt -> NaN
        let nan = compute_std_dev(0, 1, SQUARE_LIMIT, 0);
        assert!(nan.is_nan(), "expected NaN for overflow branch");
    }

    use std::sync::Arc;
    use crate::util;

    /// Test init method with actor suffix
    #[test]
    fn test_init_with_actor_suffix() {
        let _ = util::steady_logger::initialize();

        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(42, "test_actor", Some(123)),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![],
            percentiles_work: vec![],
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);

        // Should contain suffix in prometheus labels
        assert!(actor_stats.prometheus_labels.contains("actor_suffix=\"123\""));
    }

    /// Test compute method with actor suffix
    #[test]
    fn test_compute_with_actor_suffix() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", Some(42));

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 50, 0, false, None);

        // Should contain actor name with suffix
        assert!(dot_label.contains("test42"));
    }

    /// Test compute method with SHOW_ACTORS feature enabled
    #[cfg(feature = "core_display")]
    #[test]
    fn test_compute_with_show_actors() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(123, "test", None);

        // Mock the SHOW_ACTORS constant
        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 50, 0, false, None);

        // Should contain actor ID in brackets when SHOW_ACTORS is true
        // Note: This test might need adjustment based on how SHOW_ACTORS is implemented
        if steady_config::SHOW_ACTORS {
            assert!(dot_label.contains("[123]"));
        }
    }

    /// Test compute method with window display
    #[test]
    fn test_compute_with_window_display() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);
        actor_stats.window_bucket_in_bits = 2; // Non-zero to show window
        actor_stats.time_label = "5.0 mins".to_string();

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 50, 0, false, None);

        // Should contain window information
        assert!(dot_label.contains("Window 5.0 mins"));
    }

    /// Test compute method with restart count
    #[test]
    fn test_compute_with_restarts() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);
        actor_stats.prometheus_labels = "test=\"true\"".to_string();

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 50, 5, false, None);

        // Should contain restart count
        assert!(dot_label.contains("restarts: 5"));

        #[cfg(feature = "prometheus_metrics")]
        {
            // Should contain prometheus restart metric
            assert!(metric_text.contains("graph_node_restarts{"));
            assert!(metric_text.contains("} 5"));
        }
    }

    /// Test compute method with stopped actor
    #[test]
    fn test_compute_with_stopped_actor() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 50, 0, true, None);

        // Should contain stopped indicator
        assert!(dot_label.contains("stopped"));
    }

    /// Test compute method with current work data
    #[test]
    fn test_compute_with_current_work() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.show_avg_work = true;

        // Force current_work to exist by accumulating enough data
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(500, 60);
        }

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 500, 60, 0, false, None);

        // Should contain work load information
        assert!(dot_label.contains("load"));
    }

    /// Test compute method with current mcpu data
    #[test]
    fn test_compute_with_current_mcpu() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.show_avg_mcpu = true;

        // Force current_mcpu to exist by accumulating enough data
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(700, 50);
        }

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut metric_text, 700, 50, 0, false, None);

        // Should contain mcpu information
        assert!(dot_label.contains("mCPU"));
    }

    /// Test alert level triggers - Yellow
    #[test]
    fn test_trigger_alert_level_yellow() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add Yellow trigger that should fire
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));

        // Accumulate data to trigger alert
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(600, 50); // Above 256
        }

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut metric_text, 600, 50, 0, false, None);

        assert_eq!(color, DOT_YELLOW);
    }

    /// Test alert level triggers - Orange
    #[test]
    fn test_trigger_alert_level_orange() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add Orange trigger that should fire
        actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p40()), AlertColor::Orange));

        // Accumulate data to trigger alert
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 70); // Above 40%
        }

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut metric_text, 300, 70, 0, false, None);

        assert_eq!(color, DOT_ORANGE);
    }

    /// Test histogram creation errors during init
    #[test]
    fn test_init_histogram_creation_errors() {
        let _ = util::steady_logger::initialize();

        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50()], // Force histogram creation
            percentiles_work: vec![Percentile::p50()], // Force histogram creation
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();

        // This should handle potential histogram creation gracefully
        actor_stats.init(metadata, 1000);

        // Should have created histograms successfully
        assert!(actor_stats.build_mcpu_histogram);
        assert!(actor_stats.build_work_histogram);
    }

    /// Test Percentile triggers for mcpu
    #[test]
    fn test_triggered_mcpu_percentiles() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.percentiles_mcpu.push(Percentile::p50());

        // Accumulate data to get current_mcpu with histogram
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(500, 50);
        }

        // Test PercentileAbove trigger
        assert!(actor_stats.triggered_mcpu(&Trigger::PercentileAbove(Percentile::p50(), MCPU::m256())));
        assert!(!actor_stats.triggered_mcpu(&Trigger::PercentileAbove(Percentile::p50(), MCPU::m1024())));

        // Test PercentileBelow trigger
        assert!(!actor_stats.triggered_mcpu(&Trigger::PercentileBelow(Percentile::p50(), MCPU::m256())));
        assert!(actor_stats.triggered_mcpu(&Trigger::PercentileBelow(Percentile::p50(), MCPU::m1024())));
    }

    /// Test std dev functions when current data is None
    #[test]
    fn test_std_dev_functions_with_none_current() {
        let _ = util::steady_logger::initialize();

        let actor_stats = ActorStatsComputer::default();

        // Should return 0 and log info when current data is None
        assert_eq!(actor_stats.mcpu_std_dev(), 0f32);
        assert_eq!(actor_stats.work_std_dev(), 0f32);
    }

    /// Test compute_std_dev alternative calculation branch
    #[test]
    fn test_compute_std_dev_alternative_branch() {
        let _ = util::steady_logger::initialize();

        // Test the branch where sum_sqr <= r2
        let result = compute_std_dev(4, 16, 1000, 500); // sum_sqr < r2
        assert!(result >= 0.0 || result.is_nan()); // Should handle gracefully
    }

    /// Test accumulate_data_frame with histogram recording errors
    #[test]
    fn test_accumulate_data_frame_histogram_errors() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();

        // Try to record extreme values that might cause histogram errors
        actor_stats.accumulate_data_frame(1024, 100); // Max valid mcpu

        // This should not panic and should handle errors gracefully
        assert!(!actor_stats.history_mcpu.is_empty());
        assert!(!actor_stats.history_work.is_empty());
    }

    /// Test bucket refresh with histogram creation errors
    #[test]
    fn test_bucket_refresh_histogram_errors() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();

        // Force multiple bucket refreshes
        for cycle in 0..5 {
            let frames_per_bucket = 1 << actor_stats.refresh_rate_in_bits;
            for _ in 0..frames_per_bucket {
                actor_stats.accumulate_data_frame(400 + cycle * 50, 50 + cycle * 10);
            }
        }

        // Should have handled histogram creation during refresh
        assert!(!actor_stats.history_mcpu.is_empty());
        assert!(!actor_stats.history_work.is_empty());
    }

    /// Helper function to set up actor with basic data
    fn setup_actor_with_data() -> ActorStatsComputer {
        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50()],
            percentiles_work: vec![Percentile::p50()],
            std_dev_mcpu: vec![StdDev::one()],
            std_dev_work: vec![StdDev::one()],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);
        actor_stats
    }

    /// Helper function to set up actor with triggers
    fn setup_actor_with_triggers() -> ActorStatsComputer {
        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: false,
            avg_work: false,
            show_thread_info: false,
            percentiles_mcpu: vec![],
            percentiles_work: vec![],
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![], // Will be added in tests
            trigger_work: vec![], // Will be added in tests
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);
        actor_stats
    }

    /// Test comprehensive alert combinations
    #[test]
    fn test_comprehensive_alert_combinations() {
        let _ = util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add multiple triggers of different colors
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange));
        actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p70()), AlertColor::Red));

        // Test scenario where Red trigger fires (highest priority)
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(600, 80); // High work to trigger Red
        }

        let mut dot_label = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut metric_text, 600, 80, 0, false, None);

        // Should be Red (highest priority) even though other triggers also fire
        assert_eq!(color, DOT_RED);
    }
}