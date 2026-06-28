//! Telemetry metrics, triggers, and monitoring constants.

use std::time::Duration;

use crate::actor_builder_units::Percentile;

/// Constant representing an unknown monitor state.
///
/// Used in monitoring logic to indicate an undefined or uninitialized state.
// ss[related philosophy.structural-hierarchy]
pub const MONITOR_UNKNOWN: usize = usize::MAX;

/// Constant representing a "not monitored" state.
///
/// Used in monitoring logic to differentiate from `MONITOR_UNKNOWN`.
// ss[related philosophy.structural-hierarchy]
pub const MONITOR_NOT: usize = MONITOR_UNKNOWN - 1;

/// Represents the behavior of the system when a channel is saturated (i.e., full).
///
/// Defines how the system responds when attempting to send to a full channel, managing backpressure.
#[derive(Default, PartialEq, Eq, Debug, Copy, Clone)]
// ss[related philosophy.structural-hierarchy]
pub enum SendSaturation {
    /// Blocks the sender until space is available in the channel.
    AwaitForRoom,

    /// Returns an error immediately if the channel is full.
    #[deprecated(note = "Use try_send instead")]
    ReturnBlockedMsg,

    /// Logs a warning and waits for space (default behavior).
    #[default]
    WarnThenAwait,

    /// Logs a debug warning and waits, optimized for release builds.
    DebugWarnThenAwait,
}

/// Represents a standard deviation value for metrics and alerts.
///
/// Encapsulates a standard deviation within a valid range (0.0, 10.0).
#[derive(Debug, Clone, Copy, PartialEq)]
// ss[related philosophy.structural-hierarchy]
pub struct StdDev(f32);

impl StdDev {
    /// Creates a new `StdDev` if the value is within (0.0, 10.0).
    // ss[related philosophy.structural-hierarchy]
    pub fn new(value: f32) -> Option<Self> {
        if value > 0.0 && value < 10.0 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Creates a `StdDev` of 1.0.
    // ss[related philosophy.structural-hierarchy]
    pub fn one() -> Self {
        Self(1.0)
    }

    /// Creates a `StdDev` of 1.5.
    // ss[related philosophy.structural-hierarchy]
    pub fn one_and_a_half() -> Self {
        Self(1.5)
    }

    /// Creates a `StdDev` of 2.0.
    // ss[related philosophy.structural-hierarchy]
    pub fn two() -> Self {
        Self(2.0)
    }

    /// Creates a `StdDev` of 2.5.
    // ss[related philosophy.structural-hierarchy]
    pub fn two_and_a_half() -> Self {
        Self(2.5)
    }

    /// Creates a `StdDev` of 3.0.
    // ss[related philosophy.structural-hierarchy]
    pub fn three() -> Self {
        Self(3.0)
    }

    /// Creates a `StdDev` of 4.0.
    // ss[related philosophy.structural-hierarchy]
    pub fn four() -> Self {
        Self(4.0)
    }

    /// Creates a custom `StdDev` if within (0.0, 10.0).
    // ss[related philosophy.structural-hierarchy]
    pub fn custom(value: f32) -> Option<Self> {
        Self::new(value)
    }

    /// Retrieves the standard deviation value.
    // ss[related philosophy.structural-hierarchy]
    pub fn value(&self) -> f32 {
        self.0
    }
}

/// Base trait for all metrics used in telemetry and Prometheus.
// ss[related philosophy.structural-hierarchy]
pub trait Metric: PartialEq {}

/// Trait for metrics suitable for data channels.
// ss[related philosophy.structural-hierarchy]
pub trait DataMetric: Metric {}

/// Trait for metrics suitable for computational actors.
// ss[related philosophy.structural-hierarchy]
pub trait ComputeMetric: Metric {}

impl Metric for Duration {}

/// Represents the color of an alert.
///
/// Indicates the severity of an alert in the Steady State framework.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// ss[related philosophy.structural-hierarchy]
pub enum AlertColor {
    /// Warning level alert (non-critical).
    Yellow,

    /// Elevated alert level (serious).
    Orange,

    /// Critical alert level (immediate action required).
    Red,
}

/// Represents a trigger condition for a metric.
///
/// Defines conditions that trigger alerts based on metric values.
///
/// # Type Parameters
/// - `T`: The metric type implementing `Metric`.
#[derive(Clone, Copy, Debug, PartialEq)]
// ss[related philosophy.structural-hierarchy]
pub enum Trigger<T>
where
    T: Metric,
{
    /// Triggers when the average exceeds the threshold.
    AvgAbove(T),

    /// Triggers when the average falls below the threshold.
    AvgBelow(T),

    /// Triggers when above mean plus standard deviations.
    StdDevsAbove(StdDev, T),

    /// Triggers when below mean minus standard deviations.
    StdDevsBelow(StdDev, T),

    /// Triggers when above a percentile threshold.
    PercentileAbove(Percentile, T),

    /// Triggers when below a percentile threshold.
    PercentileBelow(Percentile, T),
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::time::Duration;

    use super::{
        AlertColor, SendSaturation, StdDev, Trigger, MONITOR_NOT, MONITOR_UNKNOWN,
    };
    use crate::actor_builder_units::{Percentile, Work};
    use crate::channel_builder_units::{Filled, Rate};

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn monitor_constants_are_ordered() {
        assert_eq!(MONITOR_NOT, MONITOR_UNKNOWN - 1);
        assert!(MONITOR_NOT < MONITOR_UNKNOWN);
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn send_saturation_default_is_warn_then_await() {
        assert_eq!(SendSaturation::default(), SendSaturation::WarnThenAwait);
        let _ = SendSaturation::AwaitForRoom;
        let _ = SendSaturation::DebugWarnThenAwait;
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn alert_color_variants_distinct() {
        assert_ne!(AlertColor::Yellow, AlertColor::Orange);
        assert_ne!(AlertColor::Orange, AlertColor::Red);
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn stddev_presets_match_constructors() {
        assert_eq!(StdDev::one().value(), 1.0);
        assert_eq!(StdDev::one_and_a_half().value(), 1.5);
        assert_eq!(StdDev::two().value(), 2.0);
        assert_eq!(StdDev::two_and_a_half().value(), 2.5);
        assert_eq!(StdDev::three().value(), 3.0);
        assert_eq!(StdDev::four().value(), 4.0);
        assert_eq!(StdDev::custom(2.0), Some(StdDev::two()));
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn trigger_avg_below_and_duration_metric() {
        let work = Work::new(25.0).expect("work");
        let below = Trigger::AvgBelow(work);
        assert_eq!(below, Trigger::AvgBelow(work));

        let latency = Trigger::AvgAbove(Duration::from_millis(50));
        assert_eq!(latency, Trigger::AvgAbove(Duration::from_millis(50)));
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn trigger_stddev_and_percentile_above_variants() {
        let sd = StdDev::two();
        let filled = Filled::p50();
        let above = Trigger::StdDevsAbove(sd, filled);
        let below = Trigger::StdDevsBelow(sd, filled);
        assert_ne!(above, below);

        let pct = Percentile::p90();
        let rate = Rate::per_seconds(100);
        let pct_above = Trigger::PercentileAbove(pct, rate);
        if let Trigger::PercentileAbove(p, r) = pct_above {
            assert_eq!(p, pct);
            assert_eq!(r, rate);
        } else {
            panic!("expected PercentileAbove");
        }
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn trigger_avg_below_and_percentile_below() {
        let work = Work::p50();
        let below = Trigger::AvgBelow(work);
        assert_eq!(below, Trigger::AvgBelow(work));

        let pct = Percentile::p75();
        let filled = Filled::p80();
        let pct_below = Trigger::PercentileBelow(pct, filled);
        if let Trigger::PercentileBelow(p, f) = pct_below {
            assert_eq!(p, pct);
            assert_eq!(f, filled);
        } else {
            panic!("expected PercentileBelow");
        }
    }

    #[test]
    // ss[verify philosophy.structural-hierarchy]
    fn stddev_new_rejects_boundary_values() {
        assert!(StdDev::new(0.0).is_none());
        assert!(StdDev::new(10.0).is_none());
        assert!(StdDev::new(-1.0).is_none());
    }

    #[test]
    #[allow(deprecated)]
    // ss[verify philosophy.structural-hierarchy]
    fn send_saturation_deprecated_return_blocked_msg() {
        assert_ne!(SendSaturation::ReturnBlockedMsg, SendSaturation::AwaitForRoom);
    }

    ss_proptest! {

        /// Property: StdDev accepts exactly (0, 10) open interval.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_stddev_valid_range(value: f32) {
            let sd = StdDev::new(value);
            if value > 0.0 && value < 10.0 {
                prop_assert!(sd.is_some());
                prop_assert!((sd.expect("some").value() - value).abs() < f32::EPSILON);
            } else {
                prop_assert!(sd.is_none());
            }
        }

        /// Property: Trigger variants preserve threshold metrics through clone.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_threshold_clone_consistent(work_pct in 0.0f32..=100.0f32) {
            if let Some(work) = Work::new(work_pct) {
                let t = Trigger::AvgAbove(work);
                let cloned = t;
                prop_assert_eq!(t, cloned);
                if let Trigger::AvgAbove(w) = t {
                    prop_assert_eq!(w, work);
                }
            }
        }

        /// Property: StdDevsAbove embeds valid StdDev and metric unchanged.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_stddev_above_parts(sd in 0.1f32..9.9f32, fill in 0.0f32..=100.0f32) {
            if let (Some(std_dev), Some(filled)) = (StdDev::new(sd), Filled::percentage(fill)) {
                let t = Trigger::StdDevsAbove(std_dev, filled);
                if let Trigger::StdDevsAbove(s, f) = t {
                    prop_assert_eq!(s.value(), std_dev.value());
                    prop_assert_eq!(f, filled);
                }
            }
        }

        /// Property: PercentileBelow pairs percentile with metric.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_percentile_below(p in 0.0f64..=100.0f64, work_pct in 0.0f32..=100.0f32) {
            if let (Some(pct), Some(work)) = (Percentile::new(p), Work::new(work_pct)) {
                let t = Trigger::PercentileBelow(pct, work);
                if let Trigger::PercentileBelow(p, w) = t {
                    prop_assert!((p.percentile() - pct.percentile()).abs() < f64::EPSILON);
                    prop_assert_eq!(w, work);
                }
            }
        }

        /// Property: AvgBelow preserves work threshold.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_avg_below(work_pct in 0.0f32..=100.0f32) {
            if let Some(work) = Work::new(work_pct) {
                let t = Trigger::AvgBelow(work);
                if let Trigger::AvgBelow(w) = t {
                    prop_assert_eq!(w, work);
                }
            }
        }

        /// Property: StdDevsBelow embeds valid StdDev and metric unchanged.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_stddev_below_parts(sd in 0.1f32..9.9f32, rate in 1u64..10_000u64) {
            if let Some(std_dev) = StdDev::new(sd) {
                let rate_metric = Rate::per_seconds(rate);
                let t = Trigger::StdDevsBelow(std_dev, rate_metric);
                if let Trigger::StdDevsBelow(s, r) = t {
                    prop_assert_eq!(s.value(), std_dev.value());
                    prop_assert_eq!(r, rate_metric);
                }
            }
        }

        /// Property: PercentileAbove pairs percentile with latency metric.
        #[test]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_percentile_above(p in 0.0f64..=100.0f64, ms in 1u64..5000u64) {
            if let Some(pct) = Percentile::new(p) {
                let latency = Duration::from_millis(ms);
                let t = Trigger::PercentileAbove(pct, latency);
                if let Trigger::PercentileAbove(p, d) = t {
                    prop_assert!((p.percentile() - pct.percentile()).abs() < f64::EPSILON);
                    prop_assert_eq!(d, latency);
                }
            }
        }
    }
}
