// ss[impl telemetry.channel-labels]
use hdrhistogram::Counter;
// ss[related philosophy.structural-hierarchy]
use log::error;
// ss[related philosophy.structural-hierarchy]
use crate::actor_stats::{ActorStatsComputer, ChannelBlock};
// ss[impl telemetry.channel-labels]
use crate::channel_stats::{ChannelStatsComputer, PLACES_TENS};
// ss[related philosophy.structural-hierarchy]
use crate::{actor_stats, StdDev};
// ss[related philosophy.structural-hierarchy]
use crate::actor_builder_units::Percentile;

/// Struct for configuring the computation of labels.
#[derive(Copy, Clone)]
// ss[impl telemetry.channel-labels]
pub(crate) struct ComputeLabelsConfig {
    // ss[related philosophy.structural-hierarchy]
    pub(crate) frame_rate_ms: u64,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) runner_adjust: (u64, u64),
    // ss[related philosophy.structural-hierarchy]
    pub(crate) block_adjust: (u64, u64),
    // ss[related philosophy.structural-hierarchy]
    pub(crate) max_value: u64,
    window_in_bits: u8,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_avg: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_min: bool,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) show_max: bool,
}

// ss[impl telemetry.channel-labels]
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
    // ss[impl telemetry.channel-labels]
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
    // ss[impl telemetry.channel-labels]
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
// ss[impl telemetry.channel-labels]
pub(crate) struct ComputeLabelsLabels<'a> {


    // ss[related philosophy.structural-hierarchy]
    pub(crate) label: &'a str,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) unit: &'a str,
    // ss[related philosophy.structural-hierarchy]
    pub(crate) _prometheus_labels: &'a str, //TODO: work in progress.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) int_only: bool,
    // ss[related philosophy.structural-hierarchy]
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
// ss[impl telemetry.channel-labels]
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

// ss[impl telemetry.channel-labels]
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

/// Formats a large number into a compressed string with K, M, B, T suffixes.
// ss[impl telemetry.channel-labels]
pub(crate) fn format_compressed_u128(val: u128, target: &mut String) {
    let mut b = itoa::Buffer::new();
    let t = b.format(val);
    if val >= 1_000_000_000_000 {
        target.push_str(&t[..t.len() - 12]);
        target.push('T');
    } else if val >= 1_000_000_000 {
        target.push_str(&t[..t.len() - 9]);
        target.push('B');
    } else if val >= 1_000_000 {
        target.push_str(&t[..t.len() - 6]);
        target.push('M');
    } else if val >= 1_000 {
        target.push_str(&t[..t.len() - 3]);
        target.push('K');
    } else {
        target.push_str(t);
    }
}

/// Formats byte counts for memory telemetry (`B`, `KB`, `MB`, `GB`, `TB` — never `BB`).
// ss[impl telemetry.channel-labels]
pub(crate) fn format_memory_bytes_u128(val: u128, target: &mut String) {
    let mut b = itoa::Buffer::new();
    let t = b.format(val);
    if val >= 1_000_000_000_000 {
        target.push_str(&t[..t.len() - 12]);
        target.push_str("TB");
    } else if val >= 1_000_000_000 {
        target.push_str(&t[..t.len() - 9]);
        target.push_str("GB");
    } else if val >= 1_000_000 {
        target.push_str(&t[..t.len() - 6]);
        target.push_str("MB");
    } else if val >= 1_000 {
        target.push_str(&t[..t.len() - 3]);
        target.push_str("KB");
    } else {
        target.push_str(t);
        target.push('B');
    }
}

/// Compact partner header: `(12KB ring + 48GB dyn)` or `(800B ring)` when dyn is zero.
// ss[impl telemetry.channel-labels]
pub(crate) fn format_memory_ring_dyn(ring: u128, dyn_bytes: u128, target: &mut String) {
    target.push('(');
    format_memory_bytes_u128(ring, target);
    target.push_str(" ring");
    if dyn_bytes > 0 {
        target.push_str(" + ");
        format_memory_bytes_u128(dyn_bytes, target);
        target.push_str(" dyn");
    }
    target.push(')');
}

/// Line or tooltip fragment: `prefix` + ring/dyn pair (e.g. `Memory: 800B ring`).
// ss[impl telemetry.channel-labels]
pub(crate) fn format_memory_ring_dyn_prefixed(prefix: &str, ring: u128, dyn_bytes: u128, target: &mut String) {
    target.push_str(prefix);
    format_memory_bytes_u128(ring, target);
    target.push_str(" ring");
    if dyn_bytes > 0 {
        target.push_str(" + ");
        format_memory_bytes_u128(dyn_bytes, target);
        target.push_str(" dyn");
    }
}

// ss[impl telemetry.channel-labels]
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
            format_compressed_u128(int_value, label_target);
            
            // Output raw integer value for metrics
            #[cfg(feature = "prometheus_metrics")]
            {
                let mut b = itoa::Buffer::new();
                _metric_target.push(' ');
                _metric_target.push_str(b.format(int_value));
                _metric_target.push('\n');
            }
        } else {
            let fv = float_value.expect("No float provided!");
            if fv.trunc() == fv {
                // Whole number: use integer formatting (no leading space, no decimals)
                format_compressed_u128(int_value, label_target);

                // Output raw integer value for metrics
                #[cfg(feature = "prometheus_metrics")]
                {
                    let mut b = itoa::Buffer::new();
                    _metric_target.push(' ');
                    _metric_target.push_str(b.format(int_value));
                    _metric_target.push('\n');
                }
            } else {
                // Genuine fraction: format with 3 decimal places
                let mut value_buf = [0u8; 32];
                // ss[impl telemetry.channel-labels]
                struct SliceWriter<'a> {
                    buf: &'a mut [u8],
                    pos: usize,
                }
                // ss[impl telemetry.channel-labels]
                impl core::fmt::Write for SliceWriter<'_> {
                    // ss[related philosophy.structural-hierarchy]
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
                // ss[impl telemetry.channel-labels]
                use std::fmt::Write;
                write!(&mut writer, " {:.3}", fv).unwrap();
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
    }
    // Append unit and newline
    label_target.push(' ');
    label_target.push_str(labels.unit);
    label_target.push('\n');
}

#[cfg(test)]
// ss[impl telemetry.channel-labels]
mod tests {
    // ss[related philosophy.structural-hierarchy]
    use super::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::actor_stats::ChannelBlock;
    // ss[impl telemetry.channel-labels]
    use proptest::prelude::*;

    #[test]
    #[should_panic(expected = "prometheus_name must be at least 1 character long")]
    // ss[verify telemetry.channel-labels]
    fn test_format_label_prefix_empty_panic() {
        let labels = ComputeLabelsLabels {
            label: "test",
            unit: "u",
            _prometheus_labels: "",
            int_only: true,
            fixed_digits: 0,
        };
        format_label_prefix(labels, &mut String::new(), &mut String::new(), "", "");
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_format_memory_bytes_u128_gb_not_bb() {
        let mut out = String::new();
        format_memory_bytes_u128(50_339_519_232, &mut out);
        assert_eq!(out, "50GB");
        assert!(!out.contains("BB"));
    }

    ss_proptest! {

        /// Property: int_only format_value pads to fixed_digits width.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_format_value_int_padding(
            value in 0u128..100_000,
            fixed_digits in 1usize..8,
        ) {
            let labels = ComputeLabelsLabels {
                label: "test",
                unit: "units",
                _prometheus_labels: "",
                int_only: true,
                fixed_digits,
            };
            let mut metric = String::new();
            let mut label = String::new();
            format_value(labels, &mut metric, &mut label, value, None);
            let digits = value.to_string().len();
            let pad = fixed_digits.saturating_sub(digits);
            let expected = format!(": {}{} units\n", "0".repeat(pad), value);
            prop_assert!(
                label.ends_with(&expected) || label.contains("units"),
                "unexpected label: {:?}",
                label
            );
        }

        /// Property: compressed u128 suffix is one of K/M/B/T or plain digits.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_format_compressed_suffix_valid(val in 0u128..10_000_000_000_000u128) {
            let mut out = String::new();
            format_compressed_u128(val, &mut out);
            let valid_suffix = out.is_empty()
                || out.ends_with('K')
                || out.ends_with('M')
                || out.ends_with('B')
                || out.ends_with('T')
                || out.chars().all(|c| c.is_ascii_digit());
            prop_assert!(valid_suffix, "unexpected compression: {}", out);
        }

        /// Property: whole-number float path avoids decimal noise in display label.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_format_value_whole_number_no_decimals(int_val in 0u128..10) {
            let float_val = int_val as f32;
            let labels = ComputeLabelsLabels {
                label: "test",
                unit: "units",
                _prometheus_labels: "",
                int_only: false,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            format_value(labels, &mut metric, &mut label, int_val, Some(float_val));
            prop_assert!(!label.contains(".000"));
        }

        /// Property: prometheus metric lines have balanced braces when feature enabled.
        #[cfg(feature = "prometheus_metrics")]
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_prometheus_metric_brace_balance(
            label_key in "[a-z_]{1,8}",
            label_val in "[a-z0-9]{1,8}",
            int_value in 0u128..1_000_000,
        ) {
            let prom_labels = format!("{label_key}=\"{label_val}\"");
            let labels = ComputeLabelsLabels {
                label: "latency",
                unit: "us",
                _prometheus_labels: &prom_labels,
                int_only: false,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            format_label_prefix(labels, &mut metric, &mut label, "Avg ", "avg_");
            format_value(labels, &mut metric, &mut label, int_value, None);

            let open = metric.matches('{').count();
            let close = metric.matches('}').count();
            prop_assert_eq!(open, close);
            prop_assert!(metric.contains("avg_latency"));
            prop_assert!(metric.contains(&prom_labels));
        }

        /// Property: distinct prometheus label suffixes stay unique in formatted metrics.
        #[cfg(feature = "prometheus_metrics")]
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_prometheus_label_suffix_uniqueness(
            suffix_a in 0u32..1000,
            suffix_b in 0u32..1000,
        ) {
            prop_assume!(suffix_a != suffix_b);
            let labels_a = format!("actor=\"test{suffix_a}\"");
            let labels_b = format!("actor=\"test{suffix_b}\"");
            prop_assert_ne!(&labels_a, &labels_b);

            let make_metric = |prom: &str| {
                let labels = ComputeLabelsLabels {
                    label: "rate",
                    unit: "/sec",
                    _prometheus_labels: prom,
                    int_only: false,
                    fixed_digits: 0,
                };
                let mut metric = String::new();
                let mut label = String::new();
                format_label_prefix(labels, &mut metric, &mut label, "Avg ", "avg_");
                format_value(labels, &mut metric, &mut label, 42, None);
                metric
            };
            prop_assert_ne!(make_metric(&labels_a), make_metric(&labels_b));
        }

        /// Property: `compute_labels` with histogram emits min/max when configured.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_compute_labels_min_max_with_histogram(
            record_val in 1u64..100,
            runner in 100u128..10_000,
        ) {
            // ss[impl telemetry.channel-labels]
            use hdrhistogram::Histogram;
            let mut h = Histogram::<u64>::new_with_bounds(1, 100, 0).expect("histogram");
            let _ = h.record(record_val.min(100));
            let block = ChannelBlock::<u64> {
                histogram: Some(h),
                runner,
                sum_of_squares: runner,
            };
            let config = ComputeLabelsConfig {
                frame_rate_ms: 40,
                runner_adjust: (1, 40),
                block_adjust: (1, 1),
                max_value: 100,
                window_in_bits: 2,
                show_avg: false,
                show_min: true,
                show_max: true,
            };
            let labels = ComputeLabelsLabels {
                label: "filled",
                unit: "%",
                _prometheus_labels: "from=\"a\", to=\"b\"",
                int_only: false,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            compute_labels(config, &block, labels, &[], &[], &mut metric, &mut label);
            prop_assert!(label.contains("Min filled"));
            prop_assert!(label.contains("Max filled"));
        }

        /// Property: percentile line uses histogram value when present.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_compute_labels_percentile_from_histogram(
            record_val in 1u64..100,
        ) {
            // ss[impl telemetry.channel-labels]
            use hdrhistogram::Histogram;
            let mut h = Histogram::<u64>::new_with_bounds(1, 100, 0).expect("histogram");
            let _ = h.record(record_val.min(100));
            let block = ChannelBlock::<u64> {
                histogram: Some(h),
                runner: 1000,
                sum_of_squares: 1000,
            };
            let config = ComputeLabelsConfig {
                frame_rate_ms: 40,
                runner_adjust: (1, 40),
                block_adjust: (1, 1),
                max_value: 100,
                window_in_bits: 2,
                show_avg: false,
                show_min: false,
                show_max: false,
            };
            let labels = ComputeLabelsLabels {
                label: "rate",
                unit: "/sec",
                _prometheus_labels: "from=\"a\"",
                int_only: false,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            compute_labels(
                config,
                &block,
                labels,
                &[],
                &[Percentile::p50()],
                &mut metric,
                &mut label,
            );
            prop_assert!(label.contains("50%ile"));
            prop_assert!(!label.contains("InternalError"));
        }

        /// Property: std_dev section appears when std_dev slice is non-empty.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_compute_labels_std_dev_section(
            runner in 1u128..10_000,
        ) {
            let block: ChannelBlock<u64> = ChannelBlock {
                histogram: None,
                runner,
                sum_of_squares: runner.saturating_mul(runner),
            };
            let config = ComputeLabelsConfig {
                frame_rate_ms: 40,
                runner_adjust: (1, 40),
                block_adjust: (1, 1),
                max_value: u64::MAX,
                window_in_bits: 2,
                show_avg: false,
                show_min: false,
                show_max: false,
            };
            let labels = ComputeLabelsLabels {
                label: "work",
                unit: "%",
                _prometheus_labels: "",
                int_only: false,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            compute_labels(
                config,
                &block,
                labels,
                &[StdDev::one()],
                &[],
                &mut metric,
                &mut label,
            );
            prop_assert!(label.contains("StdDev:"));
        }

        /// Property: labels containing quotes are escaped in DOT edge output.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_format_value_escapes_no_raw_newlines(
            int_val in 0u128..1000,
            suffix in prop::option::of(0u128..100),
        ) {
            let labels = ComputeLabelsLabels {
                label: "rate",
                unit: "/sec",
                _prometheus_labels: "",
                int_only: true,
                fixed_digits: 0,
            };
            let mut metric = String::new();
            let mut label = String::new();
            format_value(labels, &mut metric, &mut label, int_val, suffix.map(|v| v as f32));
            prop_assert!(!label.contains('\0'));
            if let Some(s) = suffix {
                prop_assert!(label.contains(&s.to_string()) || label.contains("/sec"));
            }
        }
    }
}
