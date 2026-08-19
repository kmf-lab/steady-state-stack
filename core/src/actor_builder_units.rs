// ss[related actor.regeneration-survives]
use crate::Metric;

/// Implements the `Metric` trait for `Work`, enabling it to be used as a telemetry metric.
// ss[related actor.regeneration-survives]
impl Metric for Work {}

/// Represents a unit of work as a percentage, used for workload analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related actor.regeneration-survives]
pub struct Work {
    /// The work value scaled to 0-10000, where 10000 represents 100%.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) work: u16,
}

// ss[related actor.regeneration-survives]
impl Work {
    /// Creates a new `Work` instance from a percentage value.
    ///
    /// # Arguments
    ///
    /// * `value` - The percentage of work, must be between 0.0 and 100.0.
    ///
    /// # Returns
    ///
    /// An `Option<Work>` containing the instance if the value is valid, otherwise `None`.
    // ss[related actor.regeneration-survives]
    pub fn new(value: f32) -> Option<Self> {
        if (0.0..=100.00).contains(&value) {
            Some(Work {
                work: (value * 100.0) as u16,
            })
        } else {
            None
        }
    }

    /// Returns the work value as a rational tuple (numerator, denominator).
    ///
    /// # Returns
    ///
    /// A tuple representing the work as a fraction of 10,000.
    // ss[related actor.regeneration-survives]
    pub fn rational(&self) -> (u64, u64) {
        (self.work as u64, 10_000)
    }

    /// Returns a `Work` instance representing 10% work.
    // ss[related actor.regeneration-survives]
    pub fn p10() -> Self {
        Work { work: 1000 }
    }

    /// Returns a `Work` instance representing 20% work.
    // ss[related actor.regeneration-survives]
    pub fn p20() -> Self {
        Work { work: 2000 }
    }

    /// Returns a `Work` instance representing 30% work.
    // ss[related actor.regeneration-survives]
    pub fn p30() -> Self {
        Work { work: 3000 }
    }

    /// Returns a `Work` instance representing 40% work.
    // ss[related actor.regeneration-survives]
    pub fn p40() -> Self {
        Work { work: 4000 }
    }

    /// Returns a `Work` instance representing 50% work.
    // ss[related actor.regeneration-survives]
    pub fn p50() -> Self {
        Work { work: 5000 }
    }

    /// Returns a `Work` instance representing 60% work.
    // ss[related actor.regeneration-survives]
    pub fn p60() -> Self {
        Work { work: 6000 }
    }

    /// Returns a `Work` instance representing 70% work.
    // ss[related actor.regeneration-survives]
    pub fn p70() -> Self {
        Work { work: 7000 }
    }

    /// Returns a `Work` instance representing 80% work.
    // ss[related actor.regeneration-survives]
    pub fn p80() -> Self {
        Work { work: 8000 }
    }

    /// Returns a `Work` instance representing 90% work.
    // ss[related actor.regeneration-survives]
    pub fn p90() -> Self {
        Work { work: 9000 }
    }

    /// Returns a `Work` instance representing 100% work.
    // ss[related actor.regeneration-survives]
    pub fn p100() -> Self {
        Work { work: 10_000 }
    }
}

/// Implements the `Metric` trait for `MCPU`, enabling it to be used as a telemetry metric.
// ss[related actor.regeneration-survives]
impl Metric for MCPU {}

/// Represents CPU usage in milli-CPUs (mCPU), used for performance analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// ss[related actor.regeneration-survives]
pub struct MCPU {
    /// The mCPU value, ranging from 1 to 1024.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) mcpu: u16,
}

// ss[related actor.regeneration-survives]
impl MCPU {
    /// Creates a new `MCPU` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The mCPU value, must be between 1 and 1024.
    ///
    /// # Returns
    ///
    /// An `Option<MCPU>` containing the instance if the value is valid, otherwise `None`.
    // ss[related actor.regeneration-survives]
    pub fn new(value: u16) -> Option<Self> {
        if value <= 1024 && value > 0 {
            Some(Self { mcpu: value })
        } else {
            None
        }
    }

    /// Returns the mCPU value.
    // ss[related actor.regeneration-survives]
    pub fn mcpu(&self) -> u16 {
        self.mcpu
    }

    /// Returns an `MCPU` instance representing 16 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m16() -> Self {
        MCPU { mcpu: 16 }
    }

    /// Returns an `MCPU` instance representing 64 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m64() -> Self {
        MCPU { mcpu: 64 }
    }

    /// Returns an `MCPU` instance representing 256 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m256() -> Self {
        MCPU { mcpu: 256 }
    }

    /// Returns an `MCPU` instance representing 512 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m512() -> Self {
        MCPU { mcpu: 512 }
    }

    /// Returns an `MCPU` instance representing 768 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m768() -> Self {
        MCPU { mcpu: 768 }
    }

    /// Returns an `MCPU` instance representing 1024 mCPU.
    // ss[related actor.regeneration-survives]
    pub fn m1024() -> Self {
        MCPU { mcpu: 1024 }
    }
}

/// Represents a percentile value for statistical analysis in telemetry metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
// ss[related actor.regeneration-survives]
pub struct Percentile(pub f64);

// ss[related philosophy.structural-hierarchy]
impl Percentile {
    /// Creates a new `Percentile` instance with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The percentile value, must be between 0.0 and 100.0.
    ///
    /// # Returns
    ///
    /// An `Option<Percentile>` containing the instance if the value is valid, otherwise `None`.
    // ss[related actor.regeneration-survives]
    pub(crate) fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns a `Percentile` instance for the 25th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p25() -> Self {
        Self(25.0)
    }

    /// Returns a `Percentile` instance for the 50th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p50() -> Self {
        Self(50.0)
    }

    /// Returns a `Percentile` instance for the 75th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p75() -> Self {
        Self(75.0)
    }

    /// Returns a `Percentile` instance for the 90th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p90() -> Self {
        Self(90.0)
    }

    /// Returns a `Percentile` instance for the 80th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p80() -> Self {
        Self(80.0)
    }

    /// Returns a `Percentile` instance for the 96th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p96() -> Self {
        Self(96.0)
    }

    /// Returns a `Percentile` instance for the 99th percentile.
    // ss[related actor.regeneration-survives]
    pub fn p99() -> Self {
        Self(99.0)
    }

    /// Creates a custom percentile value.
    ///
    /// # Arguments
    ///
    /// * `value` - The custom percentile value.
    ///
    /// # Returns
    ///
    /// An `Option<Percentile>` if the value is within the valid range.
    // ss[related actor.regeneration-survives]
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    /// Returns the percentile value.
    // ss[related actor.regeneration-survives]
    pub fn percentile(&self) -> f64 {
        self.0
    }
}

#[cfg(test)]
// ss[related actor.regeneration-survives]
mod tests {
    // ss[related philosophy.structural-hierarchy]
    use crate::actor_builder_units::{Percentile, Work, MCPU};

    #[test]
    // ss[verify actor.regeneration-survives]
    fn test_work_rational() {
        let work = Work::new(25.0).expect("internal error");
        assert_eq!(work.rational(), (2500, 10_000));
    }

    #[test]
    // ss[verify actor.regeneration-survives]
    fn test_mcpu_rational() {
        let mcpu = MCPU::new(256).expect("internal error");
        assert_eq!(mcpu.mcpu(), 256);
    }

    // ss[related actor.regeneration-survives]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: Work::new accepts exactly [0, 100] percent inputs.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_work_valid_range(value in -10.0f32..110.0f32) {
            let work = Work::new(value);
            if value >= 0.0 && value <= 100.0 {
                prop_assert!(work.is_some());
                prop_assert_eq!(work.expect("some").work, (value * 100.0) as u16);
            } else {
                prop_assert!(work.is_none());
            }
        }

        /// Property: MCPU::new accepts exactly (0, 1024] millicores.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_mcpu_valid_range(value in 0u16..1030u16) {
            let mcpu = MCPU::new(value);
            if value > 0 && value <= 1024 {
                prop_assert!(mcpu.is_some());
                prop_assert_eq!(mcpu.expect("some").mcpu, value);
            } else {
                prop_assert!(mcpu.is_none());
            }
        }

        /// Property: Percentile::new accepts exactly [0, 100].
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_percentile_valid_range(value in -5.0f64..105.0f64) {
            let percentile = Percentile::new(value);
            if value >= 0.0 && value <= 100.0 {
                prop_assert!(percentile.is_some());
                prop_assert!((percentile.expect("some").percentile() - value).abs() < f64::EPSILON);
            } else {
                prop_assert!(percentile.is_none());
            }
        }

        /// Property: Work::pN() ≡ Work::new(N*10) for N = 1..10.
        #[test]
        // ss[verify actor.regeneration-survives]
        // ss[verify verify.process.proptest]
        fn proptest_work_pN_equivalence(n in 1u64..=10u64) {
            let percent = (n * 10) as f32;
            let from_new = Work::new(percent).expect("valid percent");
            let from_pN = match n {
                1 => Work::p10(),
                2 => Work::p20(),
                3 => Work::p30(),
                4 => Work::p40(),
                5 => Work::p50(),
                6 => Work::p60(),
                7 => Work::p70(),
                8 => Work::p80(),
                9 => Work::p90(),
                10 => Work::p100(),
                _ => unreachable!(),
            };
            prop_assert_eq!(from_new, from_pN);
            prop_assert_eq!(from_new.rational(), (from_new.work as u64, 10_000));
        }
    }
}