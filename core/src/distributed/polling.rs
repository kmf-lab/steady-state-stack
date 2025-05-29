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
    // Timestamp of the last poll (in nanoseconds) is assumed to be zero
}

impl PollScheduler {
    /// Creates a new `PollScheduler` with default values.
    pub fn new() -> Self {
        PollScheduler {
            expected_moment_ns: 0,
            std_dev_ns: 1_000_000_000, // 1 second
            min_delay_ns: 1_000_000,    // 1 millisecond
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

    pub fn compute_next_delay_ns(&self, current_time_ns: u64) -> u64 {
        // Calculate the absolute distance from the expected moment
        let distance = if current_time_ns > self.expected_moment_ns {
            current_time_ns - self.expected_moment_ns
        } else {
            self.expected_moment_ns - current_time_ns
        };
        let distance_squared: u128 = distance as u128 * distance as u128;

        // Define the scaling factor based on 3 standard deviations
        let three_sigma = 3 * self.std_dev_ns;
        let s:u128 = three_sigma as u128 * three_sigma as u128;

        let max_delay_ns;
        let min_delay_ns;
        if self.max_delay_ns < self.min_delay_ns {
            let avg = (self.max_delay_ns+self.min_delay_ns)>>1;
            max_delay_ns = avg;
            min_delay_ns = avg;
        } else {
            max_delay_ns = self.max_delay_ns;
            min_delay_ns = self.min_delay_ns;
        }

        // Handle edge case where std_dev_ns is 0
        if s == 0 {
            return if current_time_ns > self.expected_moment_ns {
                self.min_delay_ns
            } else {
                self.expected_moment_ns - current_time_ns
            }
        }

        // Calculate the delay using a parabolic approximation
        let delta_delay = max_delay_ns - min_delay_ns;
        let ratio = ((distance_squared * delta_delay as u128) / s) as u64;
        let delay_ns = min_delay_ns + ratio;

        // Ensure the delay does not exceed the maximum
        delay_ns.min(max_delay_ns)
    }
}


#[cfg(test)]
mod tests {
    use super::PollScheduler;
    use std::time::Duration;

    // Helper function to simulate current_time_ns for testing
    // fn fixed_time_ns(value: u64) -> u64 {
    //     value
    // }

    #[test]
    fn test_at_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment; // Exactly at 5 seconds
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 10_000_000); // Delay should be 10 ms (min_delay_ns)
    }

    #[test]
    fn test_slightly_before_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment - 100_000_000; // 100 ms before (4.9 seconds)
        let delay = scheduler.compute_next_delay_ns(current_time);
        // Expected delay: min_delay + (distance^2 / s) * delta_delay
        let distance = 100_000_000; // 100 ms
        let s= 9_000_000_000_000_000_000 as u128; // (3 * 1e9)^2 = 9e18
        let delta_delay = 1_990_000_000; // 2e9 - 10e6
        let ratio = ((distance as u128 * distance as u128) as u128 * delta_delay as u128) / s as u128;
        let expected_delay = 10_000_000 + ratio as u64; // ~10,221,111 ns
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_slightly_after_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment + 100_000_000; // 100 ms after (5.1 seconds)
        let delay = scheduler.compute_next_delay_ns(current_time);
        let distance = 100_000_000; // 100 ms
        let s = 9_000_000_000_000_000_000 as u128; // (3 * 1e9)^2 = 9e18
        let delta_delay = 1_990_000_000; // 2e9 - 10e6
        let ratio = ((distance as u128 * distance as u128)  * delta_delay as u128) / s as u128;
        let expected_delay = 10_000_000 + ratio as u64; // ~10,221,111 ns
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_one_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment - 1_000_000_000; // 1 second before (4 seconds)
        let delay = scheduler.compute_next_delay_ns(current_time);
        let distance = 1_000_000_000; // 1 second
        let s = 9_000_000_000_000_000_000 as u128; // (3 * 1e9)^2 = 9e18
        let delta_delay = 1_990_000_000; // 2e9 - 10e6
        let ratio = ((distance as u128 * distance as u128) as u128 * delta_delay as u128) / s as u128;
        let expected_delay = 10_000_000 + ratio as u64; // ~231,111,111 ns
        assert_eq!(delay, expected_delay);
    }

    #[test]
    fn test_three_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment - 3_000_000_000; // 3 seconds before (2 seconds)
        let delay = scheduler.compute_next_delay_ns(current_time);
        // Distance is 3e9 (3 std_dev), so delay should be max_delay_ns
        assert_eq!(delay, 2_000_000_000); // 2 seconds
    }

    #[test]
    fn test_three_std_dev_after() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment + 3_000_000_000; // 3 seconds after (8 seconds)
        let delay = scheduler.compute_next_delay_ns(current_time);
        // Distance is 3e9 (3 std_dev), so delay should be max_delay_ns
        assert_eq!(delay, 2_000_000_000); // 2 seconds
    }

    #[test]
    fn test_zero_std_dev() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(0); // 0 seconds
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment - 1_000_000_000; // 1 second before
        let delay = scheduler.compute_next_delay_ns(current_time);
        // With std_dev = 0, delay should be max_delay_ns
        assert_eq!(delay, 1_000_000_000);
    }

    #[test]
    fn test_beyond_max_delay() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let current_time = expected_moment - 4_000_000_000; // 4 seconds before (1 second)
        let delay = scheduler.compute_next_delay_ns(current_time);
        // Distance is 4e9 (> 3 std_dev), so delay should be max_delay_ns
        assert_eq!(delay, 2_000_000_000); // 2 seconds
    }

    #[test]
    fn test_poll_sequence_with_delay_vector() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds in nanoseconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 ms
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        // Vector of current times (in nanoseconds) approaching and passing the moment
        let current_times = vec![
            expected_moment - 3_000_000_000, // 3 seconds before
            expected_moment - 1_000_000_000, // 1 second before
            expected_moment - 500_000_000,   // 0.5 seconds before
            expected_moment,                 // exactly at the moment
            expected_moment + 500_000_000,   // 0.5 seconds after
            expected_moment + 1_000_000_000, // 1 second after
            expected_moment + 3_000_000_000, // 3 seconds after
        ];

        // Vector of expected delays (in nanoseconds) corresponding to each current time
        let expected_delays = vec![
            2_000_000_000, // max_delay_ns at 3 seconds before (3 std devs)
            231_111_111,   // calculated delay at 1 second before
            65_277_777,    // calculated delay at 0.5 seconds before
            10_000_000,    // min_delay_ns at the moment
            65_277_777,    // symmetric delay at 0.5 seconds after
            231_111_111,   // symmetric delay at 1 second after
            2_000_000_000, // max_delay_ns at 3 seconds after (3 std devs)
        ];

        // Simulate polling sequence and verify delays
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