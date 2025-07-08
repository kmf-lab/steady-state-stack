use std::backtrace::Backtrace;
use std::cmp::Ordering;
use std::collections::VecDeque;
#[allow(unused_imports)]
use log::*;
use num_traits::Zero;
use crate::*;
use hdrhistogram::{Histogram};

use crate::actor_stats::{ChannelBlock};
use crate::channel_stats_labels;
use crate::channel_stats_labels::{ComputeLabelsConfig, ComputeLabelsLabels};

/// Constants representing the colors used in the dot graph.
pub(crate) const DOT_GREEN: &str = "green";
pub(crate) const DOT_YELLOW: &str = "yellow";
pub(crate) const DOT_ORANGE: &str = "orange";
pub(crate) const DOT_RED: &str = "red";
pub(crate) const DOT_GREY: &str = "grey";

/// Array representing the pen width values for the dot graph.
static DOT_PEN_WIDTH: [&str; 16] = [
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
    pub(crate) show_max_filled: bool, // TODO: show max filled
    pub(crate) show_min_filled: bool, // TODO: show min filled
    pub(crate) rate_trigger: Vec<(Trigger<Rate>, AlertColor)>, // If used base is green
    pub(crate) filled_trigger: Vec<(Trigger<Filled>, AlertColor)>, // If used base is green
    pub(crate) latency_trigger: Vec<(Trigger<Duration>, AlertColor)>, // If used base is green
    pub(crate) history_filled: VecDeque<ChannelBlock<u16>>, // Biggest channel length is 64K?
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
    pub(crate) current_filled: Option<ChannelBlock<u16>>,
    pub(crate) current_rate: Option<ChannelBlock<u64>>,
    pub(crate) current_latency: Option<ChannelBlock<u64>>,
    pub(crate) prometheus_labels: String,
    pub(crate) show_total: bool,
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
        self.time_label = actor_stats::time_label((self.frame_rate_ms as u128) << (meta.refresh_rate_in_bits + meta.window_bucket_in_bits));

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
        let trigger_uses_histogram = self.filled_trigger.iter().any(|t|
            matches!(t, (Trigger::PercentileAbove(_, _), _) | (Trigger::PercentileBelow(_, _), _))
        );
        self.build_filled_histogram = trigger_uses_histogram || !self.percentiles_filled.is_empty();

        if self.build_filled_histogram {
            match Histogram::<u16>::new_with_bounds(1, self.capacity as u64, 0) {
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

        let trigger_uses_histogram = self.rate_trigger.iter().any(|t|
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

        let trigger_uses_histogram = self.latency_trigger.iter().any(|t|
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
                if let Err(e) = h.record(filled) {



                    error!("unexpected, unable to record filled {} err: {} \n {:?} ", filled, e, Backtrace::capture());
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
            let latency_micros: u64 = if rate == 0 { 0u64 }
            else {
                (filled * self.frame_rate_ms) / rate
            };

            if let Some(h) = &mut f.histogram {
                if let Err(e) = h.record(latency_micros) {
                    error!("unexpected, unable to record inflight {} err: {}", latency_micros, e);
                }
            }

            let latency_micros: u64 = if rate == 0 { 0u64 }
            else {
                (filled * PLACES_TENS * self.frame_rate_ms) / rate  
            };

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
                match Histogram::<u16>::new_with_bounds(1, self.capacity as u64, 0) {
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
    pub(crate) fn compute(&mut self, display_label: &mut String, metric_text: &mut String, from_id: Option<ActorName>, send: i64, take: i64) -> (&'static str, &'static str) {
        display_label.clear();

        if self.capacity == 0 {
            return (DOT_GREY, "1");
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
        let consumed: u64 = (take - self.prev_take) as u64;
        self.accumulate_data_frame(inflight, consumed); 
        self.prev_take = take;
    
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

        if !self.window_bucket_in_bits.is_zero() {
            display_label.push_str("Window ");
            display_label.push_str(&self.time_label);
            display_label.push('\n');
        }

        self.display_labels.as_ref().iter().for_each(|labels| {
            labels.iter().for_each(|f| {
                display_label.push_str(f);
                display_label.push(' ');
            });
            display_label.push('\n');
        });

        let mut line_thick = DOT_PEN_WIDTH[0];//default

        // Does nothing if the value is None
        if let Some(ref current_rate) = self.current_rate {
            self.compute_rate_labels(display_label, metric_text, &current_rate);

            if let Some(h) = &current_rate.histogram {
                if !self.line_expansion.is_nan() {
                    let per_sec = h.value_at_percentile(80f64);

                    let adjusted_rate = ((per_sec as f32)*self.line_expansion) as u64;
                    //max of 16 step sizes.
                    let traffic_index = 64usize - (adjusted_rate >> 10).leading_zeros() as usize;

                    // Get the line thickness from the DOT_PEN_WIDTH array
                    // NOTE: [0] is 1 and they grow as a factorial after that.
                    line_thick = DOT_PEN_WIDTH[traffic_index.min(DOT_PEN_WIDTH.len() - 1)];

                }
            }
        }

        if let Some(ref current_filled) = self.current_filled {
            self.compute_filled_labels(display_label, metric_text, &current_filled);
        }

        if let Some(ref current_latency) = self.current_latency {
            self.compute_latency_labels(display_label, metric_text, &current_latency);
        }

        if self.show_total {
            display_label.push_str("Capacity: ");
            display_label.push_str(itoa::Buffer::new().format(self.capacity));
            display_label.push_str(" Total: ");
            let mut b = itoa::Buffer::new();
            let t = b.format(take);
            let len = t.len();
            let mut i = len % 3;
            if i == 0 {
                i = 3; // Start with a full group of 3 digits if the number length is a multiple of 3
            }
            // Push the first group (which could be less than 3 digits)
            display_label.push_str(&t[..i]);
            // Now loop through the rest of the string in chunks of 3
            while i < len {
                display_label.push(',');
                display_label.push_str(&t[i..i + 3]);
                i += 3;
            }
            display_label.push('\n');
        }

        // Set the default color in case we have no alerts.
        let mut line_color = DOT_GREY;

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
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
                actor_stats::avg_rational(window_in_ms as u128, PLACES_TENS as u128, &self.current_rate, rate.rational_ms()).is_lt()
            },
            Trigger::AvgAbove(rate) => {
                let window_in_ms = self.frame_rate_ms << (self.window_bucket_in_bits + self.refresh_rate_in_bits);
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
            Ordering::

            Equal // Unknown
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

    pub(crate) fn compute_rate_labels(&self, target_telemetry_label: &mut String, target_metric: &mut String, current_rate: &&ChannelBlock<u64>) {
        let config = ComputeLabelsConfig::channel_config(self
                                                         , (1, self.frame_rate_ms as usize)
                                                         , u64::MAX
                                                         , self.show_avg_rate);
        let labels = ComputeLabelsLabels {
            label: "rate",
            unit: "per/sec",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0

        };
        channel_stats_labels::compute_labels(config, current_rate
                                             , labels, &self.std_dev_rate
                                             , &self.percentiles_rate
                                             , target_metric, target_telemetry_label);

    }

    pub(crate) fn compute_filled_labels(&self, display_label: &mut String, metric_target: &mut String, current_filled: &&ChannelBlock<u16>) {
        let config = ComputeLabelsConfig::channel_config(self, (1, 10*self.capacity), 100, self.show_avg_filled);
        let labels = ComputeLabelsLabels {
            label: "filled",
            unit: "%",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0

        };
        channel_stats_labels::compute_labels(config, current_filled, labels, &self.std_dev_filled, &self.percentiles_filled, metric_target, display_label);
    }

    pub(crate) fn compute_latency_labels(&self, display_label: &mut String, metric_target: &mut String, current_latency: &&ChannelBlock<u64>) {
        let config = ComputeLabelsConfig::channel_config(self, (1, 1), u64::MAX, self.show_avg_latency);
        let labels = ComputeLabelsLabels {
            label: "latency",
            unit: "ms",
            _prometheus_labels: &self.prometheus_labels,
            int_only: false,
            fixed_digits: 0

        };

        channel_stats_labels::compute_labels(config, current_latency, labels, &self.std_dev_latency, &self.percentiles_latency, metric_target, display_label);
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
                .cmp(&((duration.as_micros())
                    << (self.window_bucket_in_bits + self.refresh_rate_in_bits)))
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
                let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                (in_flight * *percent_full_den as u128).cmp(&(*percent_full_num as u128 * self.capacity as u128))
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
                let in_flight = h.value_at_percentile(percentile.percentile()) as u128;
                in_flight.cmp(&(duration.as_millis()))
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

