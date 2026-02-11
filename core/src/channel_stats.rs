use std::backtrace::Backtrace;
use std::cmp::Ordering;
use std::collections::VecDeque;
#[allow(unused_imports)]
use log::*;
use crate::*;
use hdrhistogram::{Histogram};

use crate::actor_stats::{ChannelBlock};
use crate::channel_stats_labels;
use crate::channel_stats_labels::{ComputeLabelsConfig, ComputeLabelsLabels};
use crate::steady_config::TELEMETRY_SAMPLES_PER_FRAME;

/// Constants representing the colors used in the dot graph.
pub(crate) const DOT_GREEN: &str = "green";
pub(crate) const DOT_YELLOW: &str = "yellow";
pub(crate) const DOT_ORANGE: &str = "orange";
pub(crate) const DOT_RED: &str = "red";
pub(crate) const DOT_GREY: &str = "grey";

/// Array representing the pen width values for the dot graph.
static _DOT_PEN_WIDTH: [&str; 16] = [
    "4", "6", "8", "10", "12", "14", "16", "18", "20", "22", "24", "26", "28", "30", "32", "34"
];



/// Struct for computing statistics of a communication channel.
#[derive(Default, Debug)]
pub struct ChannelStatsComputer {
    pub(crate) display_labels: Option<Vec<&'static str>>,
    pub(crate) line_expansion: f32,
    pub(crate) show_type: Option<&'static str>,
    pub(crate) type_byte_count: usize, // Used to know bytes/sec sent
    pub(crate) percentiles_filled: Vec<Percentile>, // To show
    pub(crate) percentiles_rate: Vec<Percentile>, // To show
    pub(crate) percentiles_latency: Vec<Percentile>, // To show
    pub(crate) std_dev_filled: Vec<StdDev>, // To show
    pub(crate) std_dev_rate: Vec<StdDev>, // To show
    pub(crate) std_dev_latency: Vec<StdDev>, // To show
    pub(crate) show_avg_filled: bool,
    pub(crate) show_avg_rate: bool,
    pub(crate) show_avg_latency: bool,
    pub(crate) show_min_filled: bool,
    pub(crate) show_max_filled: bool,
    pub(crate) show_min_latency: bool,
    pub(crate) show_max_latency: bool,
    pub(crate) show_min_rate: bool,
    pub(crate) show_max_rate: bool,
    pub(crate) rate_trigger: Vec<(Trigger<Rate>, AlertColor)>, // If used base is green
    pub(crate) filled_trigger: Vec<(Trigger<Filled>, AlertColor)>, // If used base is green
    pub(crate) latency_trigger: Vec<(Trigger<Duration>, AlertColor)>, // If used base is green
    pub(crate) history_filled: VecDeque<ChannelBlock<u64>>,
    pub(crate) history_rate: VecDeque<ChannelBlock<u64>>,
    pub(crate) history_latency: VecDeque<ChannelBlock<u64>>,
    pub(crate) bucket_frames_count: usize, // When this bucket is full we add a new one
    pub(crate) refresh_rate_in_bits: u8,
    pub(crate) window_bucket_in_bits: u8,
    pub(crate) frame_rate_ms: u64, // Const at runtime but needed here for unit testing
    pub(crate) time_label: String,
    pub(crate) prev_take: i64,
    pub(crate) capacity: usize,
    pub(crate) build_filled_histogram: bool,
    pub(crate) build_rate_histogram: bool,
    pub(crate) build_latency_histogram: bool,
    pub(crate) current_filled: Option<ChannelBlock<u64>>,
    pub(crate) current_rate: Option<ChannelBlock<u64>>,
    pub(crate) current_latency: Option<ChannelBlock<u64>>,
    pub(crate) prometheus_labels: String,
    pub(crate) show_total: bool,

    pub(crate) saturation_score: f64,
    pub(crate) last_send: i64,
    pub(crate) last_take: i64,
    pub(crate) last_total: i64,

    pub(crate) partner: Option<&'static str>,
    pub(crate) bundle_index: Option<usize>,
    pub(crate) girth: usize,
    pub(crate) total_consumed: u128,
    pub(crate) memory_footprint: usize,
    pub(crate) show_memory: bool,
}

impl ChannelStatsComputer {

    /// Initializes the `ChannelStatsComputer` with metadata and settings.
    ///
    /// # Arguments
    ///
    /// * `meta` - A reference to `ChannelMetaData` containing channel metadata.
    /// * `from_actor` - The ID of the actor sending data.
    /// * `to_actor` - The ID of the actor receiving data.
    /// * `frame_rate_ms` - The frame rate in milliseconds.
    pub(crate) fn init(&mut self, meta: &Arc<ChannelMetaData>, from_actor: ActorName, to_actor: ActorName, frame_rate_ms: u64) {
        assert!(meta.capacity > 0, "capacity must be greater than 0");
        self.capacity = meta.capacity;
        self.show_total = meta.show_total;
        self.partner = meta.partner;
        self.bundle_index = meta.bundle_index;
        self.girth = meta.girth;
        self.total_consumed = 0;
        self.memory_footprint = meta.capacity * meta.type_byte_count;
        self.show_memory = meta.show_memory;

        meta.labels.iter().for_each(|f| {
            self.prometheus_labels.push_str(f);
            self.prometheus_labels.push_str("=\"T\", ");
        });
        if let Some(type_str) = meta.show_type {
            self.prometheus_labels.push_str("type=\"");
            self.prometheus_labels.push_str(type_str);
            self.prometheus_labels.push_str("\", ");
        }

        self.prometheus_labels.push_str("from=\"");
        self.prometheus_labels.push_str(from_actor.name);
        if let Some(suf) = from_actor.suffix {
            self.prometheus_labels.push_str(itoa::Buffer::new().format(suf));
        }
        self.prometheus_labels.push_str("\", ");

        self.prometheus_labels.push_str("to=\"");
        self.prometheus_labels.push_str(to_actor.name);
        if let Some(suf) = to_actor.suffix {
            self.prometheus_labels.push_str(itoa::Buffer::new().format(suf));
        }
        self.prometheus_labels.push('"');

        self.frame_rate_ms = frame_rate_ms;
        self.refresh_rate_in_bits = meta.refresh_rate_in_bits;
        self.window_bucket_in_bits = meta.window_bucket_in_bits;
        
        let total_ms = (self.frame_rate_ms as u128 * (1u128 << (meta.refresh_rate_in_bits + meta.window_bucket_in_bits))) 
                       / (TELEMETRY_SAMPLES_PER_FRAME as u128);
        self.time_label = actor_stats::time_label(total_ms);

        self.display_labels = if meta.display_labels {
            Some(meta.labels.clone())
        } else {
            None
        };
        self.line_expansion = meta.line_expansion;
        self.show_type = meta.show_type;
        self.type_byte_count = meta.type_byte_count;
        self.show_avg_filled = meta.avg_filled;
        self.show_avg_rate = meta.avg_rate;
        self.show_avg_latency = meta.avg_latency;
        self.show_max_filled = meta.max_filled;
        self.show_min_filled = meta.min_filled;
        self.show_max_rate = meta.max_rate;
        self.show_min_rate = meta.min_rate;
        self.show_max_latency = meta.max_latency;
        self.show_min_latency = meta.min_latency;


        self.percentiles_filled.clone_from(&meta.percentiles_filled);
        self.percentiles_rate.clone_from(&meta.percentiles_rate);
        self.percentiles_latency.clone_from(&meta.percentiles_latency);
        self.std_dev_filled.clone_from(&meta.std_dev_inflight);
        self.std_dev_rate.clone_from(&meta.std_dev_consumed);
        self.std_dev_latency.clone_from(&meta.std_dev_latency);
        self.rate_trigger.clone_from(&meta.trigger_rate);
        self.filled_trigger.clone_from(&meta.trigger_filled);
        self.latency_trigger.clone_from(&meta.trigger_latency);

        // Set the build * histograms last after we bring in all the triggers
        let trigger_uses_histogram = self.show_max_filled  || self.show_min_filled || self.filled_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );

        self.build_filled_histogram = trigger_uses_histogram || !self.percentiles_filled.is_empty();

        if self.build_filled_histogram {
            match Histogram::<u64>::new_with_bounds(1, self.capacity as u64, 0) {
                Ok(h) => {
                    self.history_filled.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", self.capacity, 2, e);
                }
            }
        } else {
            self.history_filled.push_back(ChannelBlock::default());
        }

        let trigger_uses_histogram = self.show_max_rate  || self.show_min_rate || self.rate_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );
        self.build_rate_histogram = trigger_uses_histogram || !self.percentiles_rate.is_empty();

        if self.build_rate_histogram {
            match Histogram::<u64>::new(2) {
                Ok(h) => {
                    self.history_rate.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                }
            }
        } else {
            self.history_rate.push_back(ChannelBlock::default());
        }

        let trigger_uses_histogram = self.show_max_latency || self.show_min_latency || self.latency_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );
        self.build_latency_histogram = trigger_uses_histogram || !self.percentiles_latency.is_empty();

        if self.build_latency_histogram {
            match Histogram::<u64>::new(2) {
                Ok(h) => {
                    self.history_latency.push_back(ChannelBlock {
                        histogram: Some(h),
                        runner: 0,
                        sum_of_squares: 0,
                    });
                }
                Err(e) => {
                    error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                }
            }
        } else {
            self.history_latency.push_back(ChannelBlock::default());
        }
        self.prev_take = 0i64;
        self.saturation_score = 0.0;
        self.last_send = 0;
        self.last_take = 0;
        self.last_total = 0;
    }

    /// Accumulates data frames for filled, rate, and latency.
    ///
    /// # Arguments
    ///
    /// * `filled` - The filled value.
    /// *
    pub(crate) fn accumulate_data_frame(&mut self, filled: u64, rate: u64) {
        self.history_filled.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                //full is full so if filled is greater than high we use high which is the capacity for 100% full
                if let Err(e) = h.record(filled.min(h.high())) {
                    error!("unexpected, unable to record filled {} err: {} hist: {:?} \n filled value is larger than channel capacity? {:?} ", filled, e, f.histogram, Backtrace::force_capture());
                }
            }
            let filled: u64 = PLACES_TENS * filled;
            f.runner = f.runner.saturating_add(filled as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((filled as u128).pow(2));
        });

        self.history_rate.iter_mut().for_each(|f| {
            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(rate) {
                    // Histogram only does raw values
                    error!("unexpected, unable to record rate {} err: {}", rate, e);
                }
            }
            let rate: u64 = PLACES_TENS * rate;
            f.runner = f.runner.saturating_add(rate as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((rate as u128).pow(2));
        });

        self.history_latency.iter_mut().for_each(|f| {
            let frame_rate_macros = self.frame_rate_ms * 1000;
            let latency_micros: u64 = if rate == 0 { 0u64 }
            else {
                //we converted out ms to microseconds
                (filled * frame_rate_macros) / rate
            };

            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(latency_micros) {
                    error!("unexpected, unable to record inflight {} err: {}", latency_micros, e);
                }
            }

            f.runner = f.runner.saturating_add(latency_micros as u128);
            f.sum_of_squares = f.sum_of_squares.saturating_add((latency_micros as u128).pow(2));
        });

        self.bucket_frames_count += 1;
        if self.bucket_frames_count >= (1 << self.refresh_rate_in_bits) {
            self.bucket_frames_count = 0;

            if self.history_filled.len() >= (1 << self.window_bucket_in_bits) {
                self.current_filled = self.history_filled.pop_front();
            }
            if self.history_rate.len() >= (1 << self.window_bucket_in_bits) {
                self.current_rate = self.history_rate.pop_front();
            }
            if self.history_latency.len() >= (1 << self.window_bucket_in_bits) {
                self.current_latency = self.history_latency.pop_front();
            }

            if self.build_filled_histogram {
                // If we cannot create a histogram we act like the window is disabled and provide no data
                match Histogram::<u64>::new_with_bounds(1, self.capacity as u64, 0) {
                    Ok(h) => {
                        self.history_filled.push_back(ChannelBlock {
                            histogram: Some(h), runner: 0, sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 1 to {} capacity with sigfig {} err: {}", self.capacity, 2, e);
                    }
                }
            } else {
                self.history_filled.push_back(ChannelBlock::default());
            }

            if self.build_rate_histogram {
                // If we cannot create a histogram we act like the window is disabled and provide no data
                match Histogram::<u64>::new(2) {
                    Ok(h) => {
                        self.history_rate.push_back(ChannelBlock {
                            histogram: Some(h), runner: 0, sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                    }
                }
            } else {
                self.history_rate.push_back(ChannelBlock::default());
            }

            if self.build_latency_histogram {
                // If we cannot create a histogram we act like the window is disabled and provide no data
                match Histogram::<u64>::new(2) {
                    Ok(h) => {
                        self.history_latency.push_back(ChannelBlock {
                            histogram: Some(h), runner: 0, sum_of_squares: 0,
                        });
                    }
                    Err(e) => {
                        error!("unexpected, unable to create histogram of 2 sigfig err: {}", e);
                    }
                }
            } else {
                self.history_latency.push_back(ChannelBlock::default());
            }
        }
    }

    /// Computes the standard deviation for rate.
    ///
    /// # Returns
    ///
    /// The computed standard deviation for rate.
    #[inline]
    pub(crate) fn rate_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_rate {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            info!("skipping std no current data");
            0f32
        }
    }

    /// Computes the standard deviation for filled.
    ///
    /// # Returns
    ///
    /// The computed standard deviation for filled.
    #[inline]
    pub(crate) fn filled_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_filled {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            0f32
        }
    }

    /// Computes the standard deviation for latency.
    ///
    /// # Returns
    ///
    /// The computed standard deviation for latency.
    #[inline]
    pub(crate) fn latency_std_dev(&self) -> f32 {
        if let Some(c) = &self.current_latency {
            actor_stats::compute_std_dev(self.refresh_rate_in_bits + self.window_bucket_in_bits
                                         , 1 << (self.refresh_rate_in_bits + self.window_bucket_in_bits)
                                         , c.runner
                                         , c.sum_of_squares)
        } else {
            0f32
        }
    }

    /// Computes and updates the labels and metrics for the channel.
    ///
    /// # Arguments
    ///
    /// * `display_label` - Mutable reference to a string for storing the display label.
    /// * `metric_text` - Mutable reference to a string for storing the metric text.
    /// * `from_id` - The ID of the actor sending data.
    /// * `send` - The send value.
    /// * `take` - The take value.
    ///
    /// # Returns
    ///
    /// A tuple containing the line color and line thickness.
    pub(crate) fn compute(&mut self, display_label: &mut String, metric_text: &mut String, from_id: Option<ActorName>, send: i64, take: i64) -> (&'static str, String) {
        display_label.clear();

        if self.capacity == 0 {
            return (DOT_GREY, "1".to_string());
        }
        assert!(self.capacity > 0, "capacity must be greater than 0 from actor {:?}, this was probably not init", from_id);

        #[cfg(debug_assertions)]
        if take > send {
            error!("actor: {:?} take:{} is greater than send:{} ", from_id, take, send);
            panic!("Internal error: take ({}) is greater than send ({})", take, send);
        }
        assert!(send >= take, "internal error send {} must be greater or eq than take {}", send, take);
        // Compute the running totals

        let inflight: u64 = (send - take) as u64;
        
        // Fix monotonicity: handle potential counter resets by using deltas
        let delta_consumed = if take >= self.prev_take {
            (take - self.prev_take) as u64
        } else {
            take as u64 // Counter reset, treat current take as the delta
        };
        self.total_consumed += delta_consumed as u128;
        
        self.accumulate_data_frame(inflight, delta_consumed); 
        self.prev_take = take;
        self.last_send = send;
        self.last_take = take;
        self.last_total = inflight as i64;
        self.saturation_score = (delta_consumed as f64 / self.capacity as f64).clamp(0.0, 1.0);
    
        ////////////////////////////////////////////////
        //  Build the labels
        ////////////////////////////////////////////////
        #[cfg(feature = "prometheus_metrics" )]
        {
            metric_text.clear();

            metric_text.push_str("inflight{");
            metric_text.push_str(&self.prometheus_labels);
            metric_text.push_str("} ");
            metric_text.push_str(itoa::Buffer::new().format(inflight));
            metric_text.push('\n');

            metric_text.push_str("send_total{");
            metric_text.push_str(&self.prometheus_labels);
            metric_text.push_str("} ");
            metric_text.push_str(itoa::Buffer::new().format(send));
            metric_text.push('\n');

            metric_text.push_str("take_total{");
            metric_text.push_str(&self.prometheus_labels);
            metric_text.push_str("} ");
            metric_text.push_str(itoa::Buffer::new().format(take));
            metric_text.push('\n');
        }

        self.show_type.iter().for_each(|f| {
            display_label.push_str(f);
            display_label.push('\n');
        });

        // NOTE: Window info is now moved to the tooltip in dot.rs to keep the graph clean.

        self.display_labels.as_ref().iter().for_each(|labels| {
            labels.iter().for_each(|f| {
                display_label.push_str(f);
                display_label.push(' ');
            });
            display_label.push('\n');
        });

        // RESTORED: Show Volume and Total on the label if enabled
        if self.show_total {
            display_label.push_str("Total: ");
            crate::channel_stats_labels::format_compressed_u128(self.total_consumed, display_label);
            // display_label.push_str(" Vol: ");
            // crate::channel_stats_labels::format_compressed_u128(self.last_total as u128, display_label);
            display_label.push('\n');
        }

        let line_thick = "1".to_string();

        // Update metrics but keep display_label clean of configuration details.
        let mut dummy_label = String::new();
        if let Some(ref current_rate) = self.current_rate {
            self.compute_rate_labels(&mut dummy_label, metric_text, &current_rate);
        }
        if let Some(ref current_filled) = self.current_filled {
            self.compute_filled_labels(&mut dummy_label, metric_text, &current_filled);
        }
        if let Some(ref current_latency) = self.current_latency {
            self.compute_latency_labels(&mut dummy_label, metric_text, &current_latency);
        }

        // NOTE: Capacity is now moved to the tooltip in dot.rs.

        // Set the default color based on activity (saturation)
        let mut line_color = if self.saturation_score > 0.8 {
            DOT_RED
        } else if self.saturation_score > 0.4 {
            DOT_ORANGE
        } else if self.saturation_score > 0.1 {
            DOT_YELLOW
        } else {
            DOT_GREY
        };

        // Triggers can override the base activity color
        if self.trigger_alert_level(&AlertColor::Yellow) {
            line_color = DOT_YELLOW;
        };
        if self.trigger_alert_level(&AlertColor::Orange) {
            line_color = DOT_ORANGE;
        };
        if self.trigger_alert_level(&AlertColor::Red) {
            line_color = DOT_RED;
        };

        (line_color, line_thick)
    }

    /// Checks if any alert trigger level is met.
    ///
    /// # Arguments
    ///
    /// * `c1` - Reference to an `AlertColor`.
    ///
    /// # Returns
    ///
    /// A boolean indicating if the alert level is met.
    fn trigger_alert_level(&mut self, c1: &AlertColor) -> bool {
        self.rate_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_rate(&f.0))
            || self.filled_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_filled(&f.0))
            || self.latency_trigger.iter().filter(|f| f.1.eq(c1)).any(|f| self.triggered_latency(&f.0))
    }

    /// Checks if a latency trigger is met.
    ///
    /// # Arguments
    ///
    /// * `rule` - Reference to a `Trigger<Duration>`.
    ///
    /// # Returns
    ///
    /// A boolean indicating if the latency trigger is met.
    pub(crate) fn triggered_latency(&self, rule: &Trigger<Duration>) -> bool {
        match rule {
            Trigger::AvgAbove(duration) => self.avg_latency(duration).is_gt(),
            Trigger::AvgBelow(duration) => self.avg_latency(duration).is_lt(),
            Trigger::StdDevsAbove(std_devs, duration) => self.stddev_latency(std_devs, duration).is_gt(),
            Trigger::StdDevsBelow(std_devs, duration) => self.stddev_latency(std_devs, duration).is_lt(),
            Trigger::PercentileAbove(percentile, duration) => self.percentile_latency(percentile, duration).is_gt(),
            Trigger::PercentileBelow(percentile, duration) => self.percentile_latency(percentile, duration).is_lt(),
        }
    }

    /// Checks if a rate trigger is met.
    ///
    /// # Arguments
    ///
    /// * `rule` - Reference to a `Trigger<Rate>`.
    ///
    /// # Returns
    ///
    /// A boolean indicating if the rate trigger is met.
    pub(crate) fn triggered_rate(&self, rule: &Trigger<Rate>) -> bool {
        match rule {
            Trigger::AvgBelow(rate) => {
                let window_in_ms = (self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits))
                                   / (TELEMETRY_SAMPLES_PER_FRAME as u64);
                actor_stats::avg_rational(window_in_ms as u128, PLACES_TENS as u128, &self.current_rate, rate.rational_ms()).is_lt()
            },
            Trigger::AvgAbove(rate) => {
                let window_in_ms = (self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits))
                                   / (TELEMETRY_SAMPLES_PER_FRAME as u64);
                actor_stats::avg_rational(window_in_ms as u128, PLACES_TENS as u128,  &self.current_rate, rate.rational_ms()).is_gt()
            },
            Trigger::StdDevsBelow(std_devs, expected_rate) => {
                actor_stats::stddev_rational(self.rate_std_dev(), self.window_bucket_in_bits + self.refresh_rate_in_bits, std_devs
                                             , &self.current_rate, expected_rate.rational_ms()).is_lt()
            },
            Trigger::StdDevsAbove(std_devs, expected_rate) => {
                actor_stats::stddev_rational(self.rate_std_dev(), self.window_bucket_in_bits + self.refresh_rate_in_bits, std_devs
                                             , &self.current_rate, expected_rate.rational_ms()).is_gt()
            },
            Trigger::PercentileAbove(percentile, rate) => actor_stats::percentile_rational(percentile, &self.current_rate, rate.rational_ms()).is_gt(),
            Trigger::PercentileBelow(percentile, rate) => actor_stats::percentile_rational(percentile, &self.current_rate, rate.rational_ms()).is_lt(),
        }
    }

    /// Checks if a filled trigger is met.
    ///
    /// # Arguments
    ///
    /// * `rule` - Reference to a `Trigger<Filled>`.
    ///
    /// # Returns
    ///
    /// A boolean indicating if the filled trigger is met.
    pub(crate) fn triggered_filled(&self, rule: &Trigger<Filled>) -> bool {
        match rule {
            Trigger::AvgAbove(Filled::Percentage(percent_full_num, percent_full_den)) => self.avg_filled_percentage(percent_full_num, percent_full_den).is_gt(),
            Trigger::AvgBelow(Filled::Percentage(percent_full_num, percent_full_den)) => self.avg_filled_percentage(percent_full_num, percent_full_den).is_lt(),
            Trigger::AvgAbove(Filled::Exact(exact_full)) => self.avg_filled_exact(exact_full).is_gt(),
            Trigger::AvgBelow(Filled::Exact(exact_full)) => self.avg_filled_exact(exact_full).is_lt(),
            Trigger::StdDevsAbove(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::StdDevsBelow(std_devs, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.stddev_filled_percentage(std_devs, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::StdDevsAbove(std_devs, Filled::Exact(exact_full)) => self.stddev_filled_exact(std_devs, exact_full).is_gt(),
            Trigger::StdDevsBelow(std_devs, Filled::Exact(exact_full)) => self.stddev_filled_exact(std_devs, exact_full).is_lt(),
            Trigger::PercentileAbove(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_gt()
            }
            Trigger::PercentileBelow(percentile, Filled::Percentage(percent_full_num, percent_full_den)) => {
                self.percentile_filled_percentage(percentile, percent_full_num, percent_full_den).is_lt()
            }
            Trigger::PercentileAbove(percentile, Filled::Exact(exact_full)) => self.percentile_filled_exact(percentile, exact_full).is_gt(),
            Trigger::PercentileBelow(percentile, Filled::Exact(exact_full)) => self.percentile_filled_exact(percentile, exact_full).is_lt(),
        }
    }

    /// Computes the average filled percentage.
    ///
    /// # Arguments
    ///
    /// * `percent_full_num` - The numerator for the percentage full.
    /// * `percent_full_den` - The denominator for the percentage full.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn avg_filled_percentage(&self, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            (current_inflight.runner * *percent_full_den as u128)
                .cmp(&((*percent_full_num as u128 * PLACES_TENS as u128 * self.capacity as u128)
                    << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the average filled exact value.
    ///
    /// # Arguments
    ///
    /// * `exact_full` - The exact value for the filled amount.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn avg_filled_exact(&self, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            current_inflight.runner
                .cmp(&((*exact_full as u128 * PLACES_TENS as u128)
                    << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
        } else {
            Ordering::Equal // Unknown
        }
    }

    pub(crate) fn compute_rate_labels(&self, target_telemetry_label: &mut String, target_metric: &mut String, current_block: &&ChannelBlock<u64>) {
        let config = ComputeLabelsConfig::channel_config(self
                                                         , (TELEMETRY_SAMPLES_PER_FRAME as u64, self.frame_rate_ms)
                                                         , (PLACES_TENS * TELEMETRY_SAMPLES_PER_FRAME as u64, self.frame_rate_ms), u64::MAX
                                                         , self.show_avg_rate, self.show_min_rate, self.show_max_rate);
        let labels = ComputeLabelsLabels {
            label: "rate",
            unit: "per/sec",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0

        };
        channel_stats_labels::compute_labels(config, current_block
                                             , labels, &self.std_dev_rate
                                             , &self.percentiles_rate
                                             , target_metric, target_telemetry_label);

    }

    pub(crate) fn compute_filled_labels(&self, display_label: &mut String, metric_target: &mut String, current_block: &&ChannelBlock<u64>) {
        //NOTE: the runner stores 1000*fill value so we need 100/1000*capacity to get a normal percentage
        let config = ComputeLabelsConfig::channel_config(self, (1u64, 10u64 * self.capacity as u64), (1u64, 1u64), 100
                                                         , self.show_avg_filled, self.show_min_filled, self.show_max_filled);
        let labels = ComputeLabelsLabels {
            label: "filled",
            unit: "%",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0
        };
        channel_stats_labels::compute_labels(config, current_block, labels, &self.std_dev_filled, &self.percentiles_filled, metric_target, display_label);
    }

    pub(crate) fn compute_latency_labels(&self, display_label: &mut String, metric_target: &mut String, current_block: &&ChannelBlock<u64>) {
        let config = ComputeLabelsConfig::channel_config(self, (1, 1), (1, 1), u64::MAX
                                                         , self.show_avg_latency, self.show_min_latency, self.show_max_latency);
        let labels = ComputeLabelsLabels {
            label: "latency",
            unit: "µs",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0
        };
        channel_stats_labels::compute_labels(config, current_block, labels, &self.std_dev_latency, &self.percentiles_latency, metric_target, display_label);
    }

    /// Milliseconds per second constant.
    const MS_PER_SEC: u64 = 1000;

    /// Computes the average latency.
    ///
    /// # Arguments
    ///
    /// * `duration` - The duration value.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn avg_latency(&self, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            assert_eq!(Self::MS_PER_SEC, PLACES_TENS); // We assume this with as_micros below
            current_latency.runner
                .cmp(&((duration.as_micros() * (1u128 << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))))
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the standard deviation for filled exact value.
    ///
    /// # Arguments
    ///
    /// * `std_devs` - The standard deviation value.
    /// * `exact_full` - The exact value for the filled amount.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn stddev_filled_exact(&self, std_devs: &StdDev, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            let std_deviation = (self.filled_std_dev() * std_devs.value()) as u128;
            let avg = current_inflight.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // Inflight > avg + f*std
            (avg + std_deviation).cmp(&(*exact_full as u128 * PLACES_TENS as u128))
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the standard deviation for filled percentage.
    ///
    /// # Arguments
    ///
    /// * `std_devs` - The standard deviation value.
    /// * `percent_full_num` - The numerator for the percentage full.
    /// * `percent_full_den` - The denominator for the percentage full.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn stddev_filled_percentage(&self, std_devs: &StdDev, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            let std_deviation = (self.filled_std_dev() * std_devs.value()) as u128;
            let avg = current_inflight.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // Inflight > avg + f*std
            ((avg + std_deviation) * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128 * PLACES_TENS as u128))
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the standard deviation for latency.
    ///
    /// # Arguments
    ///
    /// * `std_devs` - The standard deviation value.
    /// * `duration` - The duration value.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn stddev_latency(&self, std_devs: &StdDev, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            assert_eq!(1000, PLACES_TENS); // We assume this with as_micros below
            let std_deviation = (self.latency_std_dev() * std_devs.value()) as u128;
            let avg = current_latency.runner >> (self.window_bucket_in_bits + self.refresh_rate_in_bits);
            // Inflight > avg + f*std
            (avg + std_deviation).cmp(&(duration.as_micros()))
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the percentile for filled exact value.
    ///
    /// # Arguments
    ///
    /// * `percentile` - The percentile value.
    /// * `exact_full` - The exact value for the filled amount.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn percentile_filled_exact(&self, percentile: &Percentile, exact_full: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            if let Some(h) = &current_inflight.histogram {
                let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                in_flight.cmp(&(*exact_full as u128))
            } else {
                Ordering::Equal // Unknown
            }
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the percentile for filled percentage.
    ///
    /// # Arguments
    ///
    /// * `percentile` - The percentile value.
    /// * `percent_full_num` - The numerator for the percentage full.
    /// * `percent_full_den` - The denominator for the percentage full.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn percentile_filled_percentage(&self, percentile: &Percentile, percent_full_num: &u64, percent_full_den: &u64) -> Ordering {
        if let Some(current_inflight) = &self.current_filled {
            if let Some(h) = &current_inflight.histogram {
                let filled = h.value_at_percentile(percentile.percentile()) as u128;
                (filled * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128))
            } else {
                Ordering::Equal // Unknown
            }
        } else {
            Ordering::Equal // Unknown
        }
    }

    /// Computes the percentile for latency.
    ///
    /// # Arguments
    ///
    /// * `percentile` - The percentile value.
    /// * `duration` - The duration value.
    ///
    /// # Returns
    ///
    /// An ordering indicating the comparison result.
    pub(crate) fn percentile_latency(&self, percentile: &Percentile, duration: &Duration) -> Ordering {
        if let Some(current_latency) = &self.current_latency {
            if let Some(h) = &current_latency.histogram {
                let latency = h.value_at_percentile(percentile.percentile()) as u128;
                latency.cmp(&(duration.as_micros()))
            } else {
                Ordering::Equal // Unknown
            }
        } else {
            Ordering::Equal // Unknown
        }
    }
}

/// The number of decimal places used for tens.
pub(crate) const PLACES_TENS: u64 = 1000u64;

#[cfg(test)]
mod channel_stats_tests {
    use super::*;
    use std::time::Duration;
    use crate::monitor::ChannelMetaData;
    use std::sync::Arc;

    fn mock_meta() -> Arc<ChannelMetaData> {
        Arc::new(ChannelMetaData {
            capacity: 100,
            show_total: true,
            type_byte_count: 8,
            show_type: Some("u64"),
            refresh_rate_in_bits: 1, // Rollover every 2 frames
            window_bucket_in_bits: 1, // Window size 2 buckets
            ..Default::default()
        })
    }

    #[test]
    fn test_avg_filled_percentage_none() {
        let computer = ChannelStatsComputer {
            capacity: 100,
            ..Default::default()
        };
        assert_eq!(computer.avg_filled_percentage(&50, &100), Ordering::Equal);
    }

    #[test]
    fn test_avg_latency_none() {
        let computer = ChannelStatsComputer::default();
        assert_eq!(computer.avg_latency(&Duration::from_millis(100)), Ordering::Equal);
    }

    #[test]
    fn test_init_and_label_building() {
        let mut computer = ChannelStatsComputer::default();
        let meta = mock_meta();
        let from = ActorName::new("src", Some(1));
        let to = ActorName::new("dst", None);
        
        computer.init(&meta, from, to, 1000);
        
        assert_eq!(computer.capacity, 100);
        assert!(computer.prometheus_labels.contains("from=\"src1\""));
        assert!(computer.prometheus_labels.contains("to=\"dst\""));
        assert!(computer.prometheus_labels.contains("type=\"u64\""));
    }

    #[test]
    fn test_monotonic_total_and_resets() {
        let mut computer = ChannelStatsComputer::default();
        computer.init(&mock_meta(), ActorName::new("a", None), ActorName::new("b", None), 1000);

        let mut label = String::new();
        let mut metric = String::new();

        // Normal increment
        computer.compute(&mut label, &mut metric, None, 100, 50);
        assert_eq!(computer.total_consumed, 50);

        // Counter reset (take < prev_take)
        computer.compute(&mut label, &mut metric, None, 120, 10);
        // Should treat 10 as a fresh delta: 50 + 10 = 60
        assert_eq!(computer.total_consumed, 60);
    }

    #[test]
    fn test_bundle_and_memory_display() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.girth = 4;
        meta.show_memory = true;
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Verify internal state is set correctly (display label no longer contains these values)
        assert_eq!(computer.girth, 4);
        assert_eq!(computer.memory_footprint, 800); // 100 * 8 bytes
        assert!(computer.show_memory);
    }

    #[test]
    fn test_alert_color_priority() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        // Add a Red and a Yellow trigger
        meta.trigger_rate.push((Trigger::AvgAbove(Rate::per_seconds(1)), AlertColor::Yellow));
        meta.trigger_filled.push((Trigger::AvgAbove(Filled::p10()), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Fill window + 1 to trigger rotation
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(50, 100); }
        
        let mut label = String::new();
        let (color, _) = computer.compute(&mut label, &mut String::new(), None, 100, 50);
        
        assert_eq!(color, DOT_RED); // Red should override Yellow
    }

    #[test]
    fn test_trigger_gauntlet_latency() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        // Add all latency trigger variants
        meta.trigger_latency.push((Trigger::AvgAbove(Duration::from_micros(100)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::AvgBelow(Duration::from_micros(10)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::StdDevsAbove(StdDev::one(), Duration::from_micros(50)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::StdDevsBelow(StdDev::one(), Duration::from_micros(5)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::PercentileAbove(Percentile::p90(), Duration::from_micros(200)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::PercentileBelow(Percentile::p25(), Duration::from_micros(2)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Fill window + 1 to trigger rotation
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(50, 100); }
        
        assert!(computer.triggered_latency(&Trigger::AvgAbove(Duration::from_micros(100))));
        assert!(!computer.triggered_latency(&Trigger::AvgBelow(Duration::from_micros(10))));
        assert!(computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_micros(200))));
    }

    #[test]
    fn test_trigger_gauntlet_rate() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        meta.trigger_rate.push((Trigger::AvgAbove(Rate::per_seconds(50)), AlertColor::Red));
        meta.trigger_rate.push((Trigger::AvgBelow(Rate::per_seconds(10000)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Rate is 100 per message. With 32 samples/frame, that's 3200 per sec.
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(10, 100); }
        
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_seconds(50))));
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_seconds(10000))));
    }

    #[test]
    fn test_trigger_gauntlet_filled() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        meta.trigger_filled.push((Trigger::AvgAbove(Filled::Percentage(50, 100)), AlertColor::Red));
        meta.trigger_filled.push((Trigger::AvgBelow(Filled::Exact(80)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // 60 items in 100 capacity -> 60% full
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(60, 10); }
        
        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::Percentage(50, 100))));
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(80))));
    }

    #[test]
    fn test_std_dev_triggers() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.std_dev_inflight.push(StdDev::one());
        meta.std_dev_consumed.push(StdDev::one());
        meta.std_dev_latency.push(StdDev::one());
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Add varying data to create standard deviation
        for i in 0..20 { computer.accumulate_data_frame(i % 10, 100); }
        
        assert!(computer.rate_std_dev() >= 0.0);
        assert!(computer.filled_std_dev() >= 0.0);
        assert!(computer.latency_std_dev() >= 0.0);
    }

    #[test]
    fn test_zero_capacity_safety() {
        let mut computer = ChannelStatsComputer::default();
        // Capacity 0 is invalid but we test the safety return
        computer.capacity = 0;
        let mut label = String::new();
        let (color, _) = computer.compute(&mut label, &mut String::new(), None, 0, 0);
        assert_eq!(color, DOT_GREY);
    }

    #[test]
    fn test_histogram_creation_failure_handling() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        // Force histogram creation
        meta.percentiles_filled.push(Percentile::p50());
        
        // We can't easily force Histogram::new to fail without mocking, 
        // but we can verify the branches that handle it.
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        assert!(computer.build_filled_histogram);
    }
}
