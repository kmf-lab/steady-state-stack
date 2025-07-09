use std::collections::VecDeque;
use hdrhistogram::{Counter, Histogram};
#[allow(unused_imports)]
use log::*;
use std::cmp;

use crate::*;
use crate::channel_stats::{DOT_GREEN, DOT_GREY, DOT_ORANGE, DOT_RED, DOT_YELLOW, PLACES_TENS};
use crate::channel_stats_labels::{compute_labels, ComputeLabelsConfig, ComputeLabelsLabels};
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

    /// Min mCPU
    pub(crate) show_min_mcpu: bool,

    /// Max mCPU
    pub(crate) show_max_mcpu: bool,

    /// Flag to indicate whether to display average workload in telemetry.
    pub(crate) show_avg_work: bool,

    /// Max work value
    pub(crate) show_min_work: bool,

    /// Min work value
    pub(crate) show_max_work: bool,

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
        mcpu_load: Option<(u16,u16)>,
        total_count_restarts: u32,
        bool_stop: bool,
        thread_info: Option<ThreadInfo>
    ) -> (&'static str, &'static str) {
        if let Some((mcpu,load)) = mcpu_load {
            self.accumulate_data_frame(mcpu, load);
        }

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
            let config = ComputeLabelsConfig::actor_config(self, (1, 1), 100
                                                           , self.show_avg_work, self.show_min_work, self.show_max_work);
            let labels = ComputeLabelsLabels {
                label: "load",
                unit: "%",
                _prometheus_labels: &self.prometheus_labels,
                int_only: true,
                fixed_digits: 0
            };
            compute_labels(config, current_work, labels, &self.std_dev_work, &self.percentiles_work, metric_text, dot_label);
        }

        if let Some(current_mcpu) = &self.current_mcpu { //TODO: urgent problem with mCPU compute.
            let config = ComputeLabelsConfig::actor_config(self, (1, 1), 1024
                                                           , self.show_avg_mcpu, self.show_min_mcpu, self.show_max_mcpu);
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
    pub(crate) fn accumulate_data_frame(&mut self, mcpu: u16, work: u16) {
        assert!(mcpu <= 1024, "mcpu out of range {}", mcpu);
        // trace!("accumulating data frame on mCPU {}",mcpu);

        self.history_mcpu.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(mcpu as u64) {
                    error!("unexpected, unable to record inflight {} err: {}", mcpu, e);
                }
            }
            f.runner = f.runner.saturating_add(mcpu as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((mcpu as u128).pow(2));
        });

        self.history_work.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(work as u64) {
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
    pub(crate) fn trigger_alert_level(&mut self, c1: &AlertColor) -> bool {
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
    pub(crate) fn triggered_mcpu(&self, rule: &Trigger<MCPU>) -> bool {
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
    pub(crate) fn mcpu_std_dev(&self) -> f32 {
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
    pub(crate) fn work_std_dev(&self) -> f32 {
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

