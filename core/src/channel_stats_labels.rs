use hdrhistogram::Counter;
use log::error;
use crate::actor_stats::{ActorStatsComputer, ChannelBlock};
use crate::channel_stats::{ChannelStatsComputer, PLACES_TENS};
use crate::{actor_stats, StdDev};
use crate::actor_builder_units::Percentile;

/// Struct for configuring the computation of labels.
#[derive(Copy, Clone)]
pub(crate) struct ComputeLabelsConfig {
    pub(crate) frame_rate_ms: u64,
    pub(crate) runner_adjust: (u64, u64),
    pub(crate) block_adjust: (u64, u64),
    pub(crate) max_value: u64,
    window_in_bits: u8,
    pub(crate) show_avg: bool,
    pub(crate) show_min: bool,
    pub(crate) show_max: bool,
}

impl ComputeLabelsConfig {
    /// Creates a new `ComputeLabelsConfig` for a channel.
    ///
    /// # Arguments
    ///
    /// * `that` - A reference to a `ChannelStatsComputer`.
    /// * `rational_adjust` - A tuple containing the rational adjustment values.
    /// * `max_value` - The maximum value.
    /// * `show_avg` - A boolean indicating whether to show the average.
    ///
    /// # Returns
    ///
    /// A new instance of `ComputeLabelsConfig`.
    #[inline]
    pub(crate) fn channel_config(that: &ChannelStatsComputer, runner_adjust: (u64, u64), block_adjust: (u64, u64), max_value: u64, show_avg: bool, show_min: bool, show_max: bool) -> Self {

        Self {
            frame_rate_ms: that.frame_rate_ms,
            runner_adjust,
            block_adjust,
            max_value,
            window_in_bits: that.window_bucket_in_bits + that.refresh_rate_in_bits,
            show_avg,
            show_min,
            show_max
        }
    }

    /// Creates a new `ComputeLabelsConfig` for an actor.
    ///
    /// # Arguments
    ///
    /// * `that` - A reference to an `ActorStatsComputer`.
    /// * `rational_adjust` - A tuple containing the rational adjustment values.
    /// * `max_value` - The maximum value.
    /// * `show_avg` - A boolean indicating whether to show the average.
    ///
    /// # Returns
    ///
    /// A new instance of `ComputeLabelsConfig`.
    #[inline]
    pub(crate) fn actor_config(that: &ActorStatsComputer, runner_adjust: (u64, u64), block_adjust: (u64, u64), max_value: u64, show_avg: bool, show_min: bool, show_max: bool) -> Self {
        Self {
            frame_rate_ms: that.frame_rate_ms,
            runner_adjust,
            block_adjust,
            max_value,
            window_in_bits: that.window_bucket_in_bits + that.refresh_rate_in_bits,
            show_avg,
            show_min,
            show_max
        }
    }
}

/// Struct for holding label information for computing labels.
#[derive(Copy, Clone)]
pub(crate) struct ComputeLabelsLabels<'a> {


    pub(crate) label: &'a str,
    pub(crate) unit: &'a str,
    pub(crate) _prometheus_labels: &'a str, //TODO: work in progress.
    pub(crate) int_only: bool,
    pub(crate) fixed_digits: usize
}

/// Computes labels and updates the metric and label targets.
///
/// # Arguments
///
/// * `config` - A `ComputeLabelsConfig` instance.
/// * `current` - A reference to a `ChannelBlock`.
/// * `labels` - A `ComputeLabelsLabels` instance.
/// * `std_dev` - A slice of `StdDev` values.
/// * `percentile` - A slice of `Percentile` values.
/// * `metric_target` - A mutable reference to a string for storing the metric target.
/// * `label_target` - A mutable reference to a string for storing the label target.
#[inline]
pub(crate) fn compute_labels<T: Counter>(
    config: ComputeLabelsConfig,
    current: &ChannelBlock<T>,
    labels: ComputeLabelsLabels,
    std_dev: &[StdDev],
    percentile: &[Percentile],
    metric_target: &mut String, //TODO: work in progress.
    label_target: &mut String,
) {

    if config.show_avg {
        format_label_prefix(labels, metric_target, label_target, "Avg ", "avg_");
        // Compute the average value components
        let denominator = config.runner_adjust.1;
        let avg_per_sec_numer = (config.runner_adjust.0 as u128 * current.runner) >> config.window_in_bits;
        let int_value = avg_per_sec_numer / denominator as u128;
        let float_value = avg_per_sec_numer as f32 / denominator as f32;
        // error!(" int value: {}  float value: {} runner: {} window bits: {}", int_value,float_value,current.runner,  config.window_in_bits);
        format_value(labels, metric_target, label_target, int_value, Some(float_value));
    }

    if let Some(h) = &current.histogram {
        if config.show_min {
            let min_per_frame = h.min().min(config.max_value) as u128;
            let adjusted = (config.block_adjust.0 as u128 *min_per_frame) / config.block_adjust.1 as u128;
            format_label_prefix(labels, metric_target, label_target, "Min ", "min_");
            format_value(labels, metric_target, label_target, adjusted, None);
        }
        if config.show_max {
            let max_per_frame = h.max().min(config.max_value) as u128; //histogram gets a little over excited
            let adjusted = (config.block_adjust.0 as u128 *max_per_frame) / config.block_adjust.1 as u128;
            format_label_prefix(labels, metric_target, label_target, "Max ", "max_");
            format_value(labels, metric_target, label_target, adjusted, None);
        }
    }



    // Compute standard deviation if required
    let std = if !std_dev.is_empty() {
        actor_stats::compute_std_dev(config.window_in_bits, 1 << config.window_in_bits, current.runner, current.sum_of_squares)
    } else {
        0f32
    };

    // Format standard deviation entries
    std_dev.iter().for_each(|f| {
        label_target.push_str(labels.label);
        label_target.push(' ');

        let n_units = format!("{:.1}", f.value());
        if *f != StdDev::one() {
            label_target.push_str(&n_units);
        }
        label_target.push_str("StdDev: ");
        let value = &format!("{:.3}", (f.value() * std) / PLACES_TENS as f32);
        label_target.push_str(value);

        label_target.push_str(" per frame (");
        label_target.push_str(itoa::Buffer::new().format(config.frame_rate_ms));
        label_target.push_str("ms duration)\n");

        #[cfg(feature = "prometheus_metrics")]
        {
            metric_target.push_str("std_");
            metric_target.push_str(labels.label);
            metric_target.push('{');
            metric_target.push_str(labels._prometheus_labels);
            metric_target.push_str(", n=");
            metric_target.push_str(&n_units);
            metric_target.push_str("} ");
            metric_target.push_str(value);
            metric_target.push('\n');
        }
    });





    // Format percentile entries
    percentile.iter().for_each(|p| {
        label_target.push_str(labels.label);
        label_target.push(' ');

        label_target.push_str(itoa::Buffer::new().format(p.percentile() as usize));
        label_target.push_str("%ile ");

        if let Some(h) = &current.histogram {
            let value = (h.value_at_percentile(p.percentile()).min(config.max_value) as f32) as usize;
            label_target.push_str(itoa::Buffer::new().format(value));

            #[cfg(feature = "prometheus_metrics")]
            {
                metric_target.push_str("percentile_");
                metric_target.push_str(labels.label);
                metric_target.push('{');
                metric_target.push_str(labels._prometheus_labels);
                metric_target.push_str(", p=");
                metric_target.push_str(itoa::Buffer::new().format((100.0f64 * p.percentile()) as usize));
                metric_target.push_str("} ");
                metric_target.push_str(itoa::Buffer::new().format(value));
                metric_target.push('\n');
            }
        } else {
            label_target.push_str("InternalError");
            error!("InternalError: no histogram for required percentile {:?}", p);
        }
        label_target.push(' ');
        label_target.push_str(labels.unit);
        label_target.push('\n');
    });
}

fn format_label_prefix(labels: ComputeLabelsLabels, _metric_target: &mut String, label_target: &mut String, telemetry_name: &str, prometheus_name: &str) {
    // Prefix the label
    label_target.push_str(telemetry_name);
    label_target.push_str(labels.label);
    assert!(prometheus_name.len() < 96, "prometheus_name must be less than 96 characters long");
    assert!(prometheus_name.len() >0, "prometheus_name must be at least 1 character long");

    // Prefix the metric for Prometheus
    #[cfg(feature = "prometheus_metrics")]
    {
        _metric_target.push_str(prometheus_name);
        _metric_target.push_str(labels.label);
        _metric_target.push('{');
        _metric_target.push_str(labels._prometheus_labels);
        _metric_target.push('}');
    }
}

fn format_value(labels: ComputeLabelsLabels, _metric_target: &mut String, label_target: &mut String, int_value: u128, float_value: Option<f32>) {
    // Format the label based on int_only flag
    if labels.int_only {
        let mut itoa_buf = itoa::Buffer::new();
        let int_str = itoa_buf.format(int_value);
        let int_len = int_str.len();
        let pad = labels.fixed_digits.saturating_sub(int_len);
        label_target.push_str(": ");
        for _ in 0..pad {
            label_target.push('0');
        }
        label_target.push_str(int_str);

        // Output raw integer value for metrics
        #[cfg(feature = "prometheus_metrics")]
        {
            _metric_target.push(' ');
            _metric_target.push_str(int_str);
            _metric_target.push('\n');
        }
    } else {
        label_target.push_str(": ");
        if int_value >= 10 || float_value.is_none() {
            let mut b = itoa::Buffer::new();
            let t = b.format(int_value);

            if int_value >= 7_999_999_999_999 {
                label_target.push_str(&t[..t.len() - 9]);
                label_target.push('T');
            } else if int_value >= 7_999_999_999 {
                label_target.push_str(&t[..t.len() - 9]);
                label_target.push('B');
            } else if int_value >= 7_999_999 {
                label_target.push_str(&t[..t.len() - 6]);
                label_target.push('M');
            } else if int_value >= 7_999 {
                label_target.push_str(&t[..t.len() - 3]);
                label_target.push('K');
            } else {
                label_target.push_str(t);
            }
            // Output raw integer value for metrics
            #[cfg(feature = "prometheus_metrics")]
            {
                _metric_target.push(' ');
                _metric_target.push_str(t);
                _metric_target.push('\n');
            }
        } else {
            // Format float with 3 decimal places
            let mut value_buf = [0u8; 32];
            struct SliceWriter<'a> {
                buf: &'a mut [u8],
                pos: usize,
            }
            impl core::fmt::Write for SliceWriter<'_> {
                fn write_str(&mut self, s: &str) -> core::fmt::Result {
                    let bytes = s.as_bytes();
                    if self.pos + bytes.len() > self.buf.len() {
                        return Err(core::fmt::Error);
                    }
                    self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
                    self.pos += bytes.len();
                    Ok(())
                }
            }
            let mut writer = SliceWriter {
                buf: &mut value_buf,
                pos: 0,
            };
            use std::fmt::Write;
            write!(&mut writer, " {:.3}", float_value.expect("No float provided!")).unwrap();
            let offset = writer.pos;
            label_target.push_str(core::str::from_utf8(&value_buf[..offset]).expect("internal error"));

            // Output raw float value for metrics
            #[cfg(feature = "prometheus_metrics")]
            {
                _metric_target.push(' ');
                _metric_target.push_str(core::str::from_utf8(&value_buf[..offset]).expect("internal error"));
                _metric_target.push('\n');
            }
        }
    }
    // Append unit and newline
    label_target.push(' ');
    label_target.push_str(labels.unit);
    label_target.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value_int_padding() {
        let labels = ComputeLabelsLabels {
            label: "test",
            unit: "units",
            _prometheus_labels: "",
            int_only: true,
            fixed_digits: 5,
        };
        let mut metric = String::new();
        let mut label = String::new();
        
        format_value(labels, &mut metric, &mut label, 42, None);
        // Should pad with 3 zeros to reach 5 digits
        assert!(label.contains(": 00042 units\n"));
    }

    // #[test]
    // fn test_format_value_scaling() {
    //     let labels = ComputeLabelsLabels {
    //         label: "test",
    //         unit: "units",
    //         _prometheus_labels: "",
    //         int_only: false,
    //         fixed_digits: 0,
    //     };
    //
    //     let test_cases = [
    //         (5, "0.005", "small float"),
    //         (5000, "5K", "thousands"),
    //         (5_000_000, "5M", "millions"),
    //         (5_000_000_000, "5B", "billions"),
    //         (5_000_000_000_000, "5T", "trillions"),
    //     ];
    //
    //     for (val, expected, msg) in test_cases {
    //         let mut metric = String::new();
    //         let mut label = String::new();
    //         let float_val = if val < 10 { Some(val as f32 / 1000.0) } else { None };
    //         format_value(labels, &mut metric, &mut label, val, float_val);
    //         assert!(label.contains(expected), "Failed {}: expected {} in {}", msg, expected, label);
    //     }
    // }

    #[test]
    fn test_format_label_prefix_assertions() {
        let labels = ComputeLabelsLabels {
            label: "test",
            unit: "u",
            _prometheus_labels: "job=\"test\"",
            int_only: true,
            fixed_digits: 0,
        };
        let mut metric = String::new();
        let mut label = String::new();
        
        format_label_prefix(labels, &mut metric, &mut label, "Prefix ", "prom_");
        assert_eq!(label, "Prefix test");
        #[cfg(feature = "prometheus_metrics")]
        assert!(metric.starts_with("prom_test{job=\"test\"}"));
    }

    #[test]
    #[should_panic(expected = "prometheus_name must be at least 1 character long")]
    fn test_format_label_prefix_empty_panic() {
        let labels = ComputeLabelsLabels {
            label: "test", unit: "u", _prometheus_labels: "", int_only: true, fixed_digits: 0,
        };
        format_label_prefix(labels, &mut String::new(), &mut String::new(), "", "");
    }
}
