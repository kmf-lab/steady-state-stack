use crate::Metric;

impl Metric for Rate {}

/**
 * Represents a rate of occurrence over time for telemetry triggers and metrics.
 *
 * Uses a rational number (numerator/denominator) for precision, avoiding floating-point arithmetic.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    // Internal representation as a rational number of the rate per ms
    // Numerator: units, Denominator: time in ms
    numerator: u64,
    denominator: u64,
}

impl Rate {
    /** Creates a rate representing units per millisecond. */
    pub fn per_millis(units: u64) -> Self {
        Self { numerator: units, denominator: 1 }
    }

    /** Creates a rate representing units per second. */
    pub fn per_seconds(units: u64) -> Self {
        Self { numerator: units, denominator: 1000 }
    }

    /** Creates a rate representing units per minute. */
    pub fn per_minutes(units: u64) -> Self {
        Self { numerator: units, denominator: 60_000 }
    }

    /** Creates a rate representing units per hour. */
    pub fn per_hours(units: u64) -> Self {
        Self { numerator: units, denominator: 3_600_000 }
    }

    /** Creates a rate representing units per day. */
    pub fn per_days(units: u64) -> Self {
        Self { numerator: units, denominator: 86_400_000 }
    }

    /** Returns the rate as a rational number (numerator, denominator) per millisecond. */
    pub fn rational_ms(&self) -> (u64, u64) {
        (self.numerator, self.denominator)
    }
}

impl Metric for Filled {}

/**
 * Represents fill levels for channel capacity in telemetry triggers and metrics.
 *
 * Supports percentage-based or exact count representations for flexible monitoring.
 */
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filled {
    /// Represents a fill level as a percentage, stored as a rational number (numerator, denominator).
    Percentage(u64, u64),
    /// Represents an exact fill level as a count of items.
    Exact(u64),
}

impl Filled {
    /** Creates a `Filled` instance from a percentage value (0.0 to 100.0), returning `None` if out of range. */
    pub fn percentage(value: f32) -> Option<Self> {
        if (0.0..=100.0).contains(&value) {
            Some(Self::Percentage((value * 1_000f32) as u64, 100_000u64))
        } else {
            None
        }
    }

    /** Creates a `Filled` instance representing an exact item count. */
    pub fn exact(value: u64) -> Self {
        Self::Exact(value)
    }

    /** Returns a `Filled` instance for 10% capacity. */
    pub fn p10() -> Self { Self::Percentage(10, 100) }

    /** Returns a `Filled` instance for 20% capacity. */
    pub fn p20() -> Self { Self::Percentage(20, 100) }

    /** Returns a `Filled` instance for 30% capacity. */
    pub fn p30() -> Self { Self::Percentage(30, 100) }

    /** Returns a `Filled` instance for 40% capacity. */
    pub fn p40() -> Self { Self::Percentage(40, 100) }

    /** Returns a `Filled` instance for 50% capacity. */
    pub fn p50() -> Self { Self::Percentage(50, 100) }

    /** Returns a `Filled` instance for 60% capacity. */
    pub fn p60() -> Self { Self::Percentage(60, 100) }

    /** Returns a `Filled` instance for 70% capacity. */
    pub fn p70() -> Self { Self::Percentage(70, 100) }

    /** Returns a `Filled` instance for 80% capacity. */
    pub fn p80() -> Self { Self::Percentage(80, 100) }

    /** Returns a `Filled` instance for 90% capacity. */
    pub fn p90() -> Self { Self::Percentage(90, 100) }

    /** Returns a `Filled` instance for 100% capacity. */
    pub fn p100() -> Self { Self::Percentage(100, 100) }
}
