use crate::Metric;

/// Implements the `Metric` trait for `Work`, enabling it to be used as a telemetry metric.
impl Metric for Work {}

/// Represents a unit of work as a percentage, used for workload analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    /// The work value scaled to 0-10000, where 10000 represents 100%.
    pub(crate) work: u16,
}

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
    pub fn rational(&self) -> (u64, u64) {
        (self.work as u64, 10_000)
    }

    /// Returns a `Work` instance representing 10% work.
    pub fn p10() -> Self {
        Work { work: 1000 }
    }

    /// Returns a `Work` instance representing 20% work.
    pub fn p20() -> Self {
        Work { work: 2000 }
    }

    /// Returns a `Work` instance representing 30% work.
    pub fn p30() -> Self {
        Work { work: 3000 }
    }

    /// Returns a `Work` instance representing 40% work.
    pub fn p40() -> Self {
        Work { work: 4000 }
    }

    /// Returns a `Work` instance representing 50% work.
    pub fn p50() -> Self {
        Work { work: 5000 }
    }

    /// Returns a `Work` instance representing 60% work.
    pub fn p60() -> Self {
        Work { work: 6000 }
    }

    /// Returns a `Work` instance representing 70% work.
    pub fn p70() -> Self {
        Work { work: 7000 }
    }

    /// Returns a `Work` instance representing 80% work.
    pub fn p80() -> Self {
        Work { work: 8000 }
    }

    /// Returns a `Work` instance representing 90% work.
    pub fn p90() -> Self {
        Work { work: 9000 }
    }

    /// Returns a `Work` instance representing 100% work.
    pub fn p100() -> Self {
        Work { work: 10_000 }
    }
}

/// Implements the `Metric` trait for `MCPU`, enabling it to be used as a telemetry metric.
impl Metric for MCPU {}

/// Represents CPU usage in milli-CPUs (mCPU), used for performance analysis and monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MCPU {
    /// The mCPU value, ranging from 1 to 1024.
    pub(crate) mcpu: u16,
}

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
    pub fn new(value: u16) -> Option<Self> {
        if value <= 1024 && value > 0 {
            Some(Self { mcpu: value })
        } else {
            None
        }
    }

    /// Returns the mCPU value.
    pub fn mcpu(&self) -> u16 {
        self.mcpu
    }

    /// Returns an `MCPU` instance representing 16 mCPU.
    pub fn m16() -> Self {
        MCPU { mcpu: 16 }
    }

    /// Returns an `MCPU` instance representing 64 mCPU.
    pub fn m64() -> Self {
        MCPU { mcpu: 64 }
    }

    /// Returns an `MCPU` instance representing 256 mCPU.
    pub fn m256() -> Self {
        MCPU { mcpu: 256 }
    }

    /// Returns an `MCPU` instance representing 512 mCPU.
    pub fn m512() -> Self {
        MCPU { mcpu: 512 }
    }

    /// Returns an `MCPU` instance representing 768 mCPU.
    pub fn m768() -> Self {
        MCPU { mcpu: 768 }
    }

    /// Returns an `MCPU` instance representing 1024 mCPU.
    pub fn m1024() -> Self {
        MCPU { mcpu: 1024 }
    }
}

/// Represents a percentile value for statistical analysis in telemetry metrics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Percentile(pub f64);

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
    pub(crate) fn new(value: f64) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns a `Percentile` instance for the 25th percentile.
    pub fn p25() -> Self {
        Self(25.0)
    }

    /// Returns a `Percentile` instance for the 50th percentile.
    pub fn p50() -> Self {
        Self(50.0)
    }

    /// Returns a `Percentile` instance for the 75th percentile.
    pub fn p75() -> Self {
        Self(75.0)
    }

    /// Returns a `Percentile` instance for the 90th percentile.
    pub fn p90() -> Self {
        Self(90.0)
    }

    /// Returns a `Percentile` instance for the 80th percentile.
    pub fn p80() -> Self {
        Self(80.0)
    }

    /// Returns a `Percentile` instance for the 96th percentile.
    pub fn p96() -> Self {
        Self(96.0)
    }

    /// Returns a `Percentile` instance for the 99th percentile.
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
    pub fn custom(value: f64) -> Option<Self> {
        Self::new(value)
    }

    /// Returns the percentile value.
    pub fn percentile(&self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use crate::actor_builder_units::{Percentile, Work, MCPU};

    #[test]
    fn test_work_new_valid() {
        assert_eq!(Work::new(50.0), Some(Work { work: 5000 }));
        assert_eq!(Work::new(0.0), Some(Work { work: 0 }));
        assert_eq!(Work::new(100.0), Some(Work { work: 10_000 }));
    }

    #[test]
    fn test_work_new_invalid() {
        assert_eq!(Work::new(-1.0), None);
        assert_eq!(Work::new(101.0), None);
    }

    #[test]
    fn test_work_rational() {
        let work = Work::new(25.0).expect("internal error");
        assert_eq!(work.rational(), (2500, 10_000));
    }

    #[test]
    fn test_work_percent_methods() {
        assert_eq!(Work::p10(), Work { work: 1000 });
        assert_eq!(Work::p20(), Work { work: 2000 });
        assert_eq!(Work::p30(), Work { work: 3000 });
        assert_eq!(Work::p40(), Work { work: 4000 });
        assert_eq!(Work::p50(), Work { work: 5000 });
        assert_eq!(Work::p60(), Work { work: 6000 });
        assert_eq!(Work::p70(), Work { work: 7000 });
        assert_eq!(Work::p80(), Work { work: 8000 });
        assert_eq!(Work::p90(), Work { work: 9000 });
        assert_eq!(Work::p100(), Work { work: 10_000 });
    }

    #[test]
    fn test_mcpu_new_valid() {
        assert_eq!(MCPU::new(512), Some(MCPU { mcpu: 512 }));
        assert_eq!(MCPU::new(0), None);
        assert_eq!(MCPU::new(1024), Some(MCPU { mcpu: 1024 }));
    }

    #[test]
    fn test_mcpu_new_invalid() {
        assert_eq!(MCPU::new(1025), None);
    }

    #[test]
    fn test_mcpu_rational() {
        let mcpu = MCPU::new(256).expect("internal error");
        assert_eq!(mcpu.mcpu(), 256);
    }

    #[test]
    fn test_mcpu_methods() {
        assert_eq!(MCPU::m16(), MCPU { mcpu: 16 });
        assert_eq!(MCPU::m64(), MCPU { mcpu: 64 });
        assert_eq!(MCPU::m256(), MCPU { mcpu: 256 });
        assert_eq!(MCPU::m512(), MCPU { mcpu: 512 });
        assert_eq!(MCPU::m768(), MCPU { mcpu: 768 });
        assert_eq!(MCPU::m1024(), MCPU { mcpu: 1024 });
    }

    #[test]
    fn test_percentile_new_valid() {
        assert_eq!(Percentile::new(25.0), Some(Percentile(25.0)));
        assert_eq!(Percentile::new(0.0), Some(Percentile(0.0)));
        assert_eq!(Percentile::new(100.0), Some(Percentile(100.0)));
    }

    #[test]
    fn test_percentile_new_invalid() {
        assert_eq!(Percentile::new(-1.0), None);
        assert_eq!(Percentile::new(101.0), None);
    }

    #[test]
    fn test_percentile_methods() {
        assert_eq!(Percentile::p25(), Percentile(25.0));
        assert_eq!(Percentile::p50(), Percentile(50.0));
        assert_eq!(Percentile::p75(), Percentile(75.0));
        assert_eq!(Percentile::p80(), Percentile(80.0));
        assert_eq!(Percentile::p90(), Percentile(90.0));
        assert_eq!(Percentile::p96(), Percentile(96.0));
        assert_eq!(Percentile::p99(), Percentile(99.0));
    }

    #[test]
    fn test_percentile_custom() {
        assert_eq!(Percentile::custom(42.0), Some(Percentile(42.0)));
        assert_eq!(Percentile::custom(-1.0), None);
        assert_eq!(Percentile::custom(101.0), None);
    }

    #[test]
    fn test_percentile_getter() {
        let percentile = Percentile::new(42.0).expect("internal error");
        assert_eq!(percentile.percentile(), 42.0);
    }
}