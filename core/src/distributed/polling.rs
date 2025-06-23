#[allow(unused_imports)]
use log::*;

/// A scheduler for polling with adaptive delays based on a bell curve.
pub struct PollScheduler {
    /// The expected moment for the next event (in nanoseconds).
    expected_moment_ns: u64,
    /// Standard deviation controlling the bell curve width (in nanoseconds).
    std_dev_ns: u64,
    /// Minimum delay between polls (in nanoseconds).
    min_delay_ns: u64,
    /// Maximum delay between polls (in nanoseconds).
    max_delay_ns: u64,
}

impl Default for PollScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl PollScheduler {
    /// Creates a new `PollScheduler` with default values.
    pub fn new() -> Self {
        PollScheduler {
            expected_moment_ns: 0,
            std_dev_ns: 1_000_000_000, // 1 second
            min_delay_ns: 1_000_000,   // 1 millisecond
            max_delay_ns: 1_000_000_000, // 1 second
        }
    }

    /// Sets the expected moment for the next event.
    pub fn set_expected_moment_ns(&mut self, time_ns: u64) {
        self.expected_moment_ns = time_ns;
    }

    /// Sets the standard deviation for the bell curve.
    pub fn set_std_dev_ns(&mut self, std_dev_ns: u64) {
        self.std_dev_ns = std_dev_ns;
    }

    /// Sets the minimum delay between polls.
    pub fn set_min_delay_ns(&mut self, min_delay_ns: u64) {
        self.min_delay_ns = min_delay_ns;
    }

    /// Sets the maximum delay between polls.
    pub fn set_max_delay_ns(&mut self, max_delay_ns: u64) {
        self.max_delay_ns = max_delay_ns;
    }

    /// Gets the expected moment for the next event.
    pub fn get_expected_moment_ns(&self) -> u64 {
        self.expected_moment_ns
    }

    /// Gets the standard deviation for the bell curve.
    pub fn get_std_dev_ns(&self) -> u64 {
        self.std_dev_ns
    }

    /// Gets the minimum delay between polls.
    pub fn get_min_delay_ns(&self) -> u64 {
        self.min_delay_ns
    }

    /// Gets the maximum delay between polls.
    pub fn get_max_delay_ns(&self) -> u64 {
        self.max_delay_ns
    }

    pub fn compute_next_delay_ns(&self, current_time_ns: u64) -> u64 {
        let distance = if current_time_ns > self.expected_moment_ns {
            current_time_ns - self.expected_moment_ns
        } else {
            self.expected_moment_ns - current_time_ns
        };
        let distance_squared: u128 = distance as u128 * distance as u128;

        let three_sigma = 3 * self.std_dev_ns;
        let s: u128 = three_sigma as u128 * three_sigma as u128;

        let max_delay_ns;
        let min_delay_ns;
        if self.max_delay_ns < self.min_delay_ns {
            let avg = (self.max_delay_ns + self.min_delay_ns) >> 1;
            max_delay_ns = avg;
            min_delay_ns = avg;
        } else {
            max_delay_ns = self.max_delay_ns;
            min_delay_ns = self.min_delay_ns;
        }

        if s == 0 {
            return if current_time_ns > self.expected_moment_ns {
                self.min_delay_ns
            } else {
                self.expected_moment_ns - current_time_ns
            };
        }

        let delta_delay = max_delay_ns - min_delay_ns;
        let ratio = ((distance_squared * delta_delay as u128) / s) as u64;
        let delay_ns = min_delay_ns + ratio;

        delay_ns.min(max_delay_ns)
    }
}

#[cfg(test)]
mod tests {
    use super::PollScheduler;

    #[test]
    fn test_at_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let delay = scheduler.compute_next_delay_ns(expected_moment);
        assert_eq!(delay, 10_000_000); // Should be min_delay_ns
    }

    #[test]
    fn test_slightly_before_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 100_000_000; // 100 ms before
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((100_000_000_u128 * 100_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_slightly_after_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment + 100_000_000; // 100 ms after
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((100_000_000_u128 * 100_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_one_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 1_000_000_000; // 1s before
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((1_000_000_000_u128 * 1_000_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_three_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 3_000_000_000; // 3s before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should be max_delay_ns
    }

    #[test]
    fn test_three_std_dev_after() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment + 3_000_000_000; // 3s after
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should be max_delay_ns
    }

    #[test]
    fn test_zero_std_dev() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(0);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 1_000_000_000; // 1s before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 1_000_000_000); // Time until expected moment
    }

    #[test]
    fn test_beyond_max_delay() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 4_000_000_000; // 4s before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should be max_delay_ns
    }

    #[test]
    fn test_poll_sequence_with_delay_vector() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_times = [expected_moment - 3_000_000_000,
            expected_moment - 1_000_000_000,
            expected_moment - 500_000_000,
            expected_moment,
            expected_moment + 500_000_000,
            expected_moment + 1_000_000_000,
            expected_moment + 3_000_000_000];

        let expected_delays = [2_000_000_000, // 3s before
            231_111_111,   // 1s before
            65_277_777,    // 0.5s before
            10_000_000,    // at moment
            65_277_777,    // 0.5s after
            231_111_111,   // 1s after
            2_000_000_000];

        for (i, &current_time) in current_times.iter().enumerate() {
            let computed_delay = scheduler.compute_next_delay_ns(current_time);
            assert_eq!(
                computed_delay, expected_delays[i],
                "Delay mismatch at step {}: expected {}, got {}",
                i, expected_delays[i], computed_delay
            );
        }
    }
}