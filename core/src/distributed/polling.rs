//! A scheduler for polling with adaptive delays based on a bell curve.
//!
//! The `PollScheduler` helps manage how often a system checks (or "polls") for an event that happens
//! around a certain time but with some unpredictability. Imagine you’re waiting for a bus that usually
//! arrives at 3:00 PM, but sometimes it’s a bit early or late. This scheduler uses a bell curve—a
//! shape like a hill that’s highest in the middle—to decide how long to wait before checking again.
//! When the current time is close to when the event is expected, it checks more often. When it’s far
//! away, it waits longer between checks. This balances being quick to respond with not wasting effort.
//!
//! ### What It Does
//! - It predicts when the next event will happen (called the "expected moment").
//! - It adjusts the waiting time (delay) between checks based on how close the current time is to that moment.
//! - It uses a bell curve to make the delay short when the event is near and long when it’s far off.
//!
//! ### Key Pieces
//! - **Expected Moment**: The time when the event is most likely to happen, measured in nanoseconds (tiny fractions of a second).
//! - **Standard Deviation**: How much the event’s timing might vary, like how early or late the bus might be. A bigger number means more variation.
//! - **Minimum Delay**: The shortest time between checks, so the system doesn’t check too often and get overloaded.
//! - **Maximum Delay**: The longest time between checks, so the system doesn’t wait too long and miss the event.
//!
//! ### How It Adjusts the Delay
//! - If the current time is right at the expected moment, it uses the shortest delay.
//! - As the current time gets farther from the expected moment (before or after), the delay grows, up to the maximum.
//! - If there’s no variation (standard deviation is zero), it waits exactly until the expected moment if it hasn’t happened yet, or checks immediately if it’s already passed.

#[allow(unused_imports)]
use log::*;

/// A scheduler for polling with adaptive delays based on a bell curve.
///
/// This structure holds all the information needed to decide when to check for an event next.
/// It keeps track of when the event is expected, how much its timing might vary, and the shortest
/// and longest times allowed between checks.
pub struct PollScheduler {
    /// The expected moment for the next event (in nanoseconds).
    /// This is when the scheduler thinks the event will most likely happen.
    expected_moment_ns: u64,
    /// Standard deviation controlling the bell curve width (in nanoseconds).
    /// This measures how much the event’s timing can vary. A larger value means the event might happen
    /// farther from the expected moment, widening the bell curve.
    std_dev_ns: u64,
    /// Minimum delay between polls (in nanoseconds).
    /// This is the shortest time the scheduler will wait before checking again, preventing it from
    /// checking too frequently and wasting resources.
    min_delay_ns: u64,
    /// Maximum delay between polls (in nanoseconds).
    /// This is the longest time the scheduler will wait, ensuring it doesn’t miss events by waiting too long.
    max_delay_ns: u64,
}

impl Default for PollScheduler {
    /// Sets up a scheduler with default values if you don’t specify anything.
    fn default() -> Self {
        Self::new()
    }
}

impl PollScheduler {
    /// Creates a new scheduler with starting values.
    ///
    /// This sets up the scheduler with sensible defaults so it’s ready to use:
    /// - Expected moment is 0 (no event expected yet).
    /// - Standard deviation is 1 second (1,000,000,000 nanoseconds), meaning the event might vary by about a second.
    /// - Minimum delay is 1 millisecond (1,000,000 nanoseconds), a short wait to avoid overloading.
    /// - Maximum delay is 1 second (1,000,000,000 nanoseconds), a reasonable longest wait.
    ///
    /// You can change these later if needed.
    pub fn new() -> Self {
        PollScheduler {
            expected_moment_ns: 0,
            std_dev_ns: 1_000_000_000, // 1 second
            min_delay_ns: 1_000_000,   // 1 millisecond
            max_delay_ns: 1_000_000_000, // 1 second
        }
    }

    /// Sets when the next event is expected to happen.
    ///
    /// Use this to tell the scheduler when you think the event will occur.
    /// The time is in nanoseconds (1 billion nanoseconds = 1 second).
    pub fn set_expected_moment_ns(&mut self, time_ns: u64) {
        self.expected_moment_ns = time_ns;
    }

    /// Sets how much the event’s timing might vary.
    ///
    /// This is the standard deviation in nanoseconds. A bigger number means the event could happen
    /// farther from the expected time, making the scheduler more flexible in its timing.
    pub fn set_std_dev_ns(&mut self, std_dev_ns: u64) {
        self.std_dev_ns = std_dev_ns;
    }

    /// Sets the shortest time between checks.
    ///
    /// This is the minimum delay in nanoseconds. It stops the scheduler from checking too often,
    /// which could slow down the system.
    pub fn set_min_delay_ns(&mut self, min_delay_ns: u64) {
        self.min_delay_ns = min_delay_ns;
    }

    /// Sets the longest time between checks.
    ///
    /// This is the maximum delay in nanoseconds. It ensures the scheduler doesn’t wait too long
    /// and miss the event.
    pub fn set_max_delay_ns(&mut self, max_delay_ns: u64) {
        self.max_delay_ns = max_delay_ns;
    }

    /// Gets the current expected moment for the next event.
    ///
    /// This tells you when the scheduler thinks the event will happen, in nanoseconds.
    pub fn get_expected_moment_ns(&self) -> u64 {
        self.expected_moment_ns
    }

    /// Gets the current standard deviation.
    ///
    /// This shows how much variation the scheduler expects in the event’s timing, in nanoseconds.
    pub fn get_std_dev_ns(&self) -> u64 {
        self.std_dev_ns
    }

    /// Gets the minimum delay between checks.
    ///
    /// This is the shortest wait time the scheduler will use, in nanoseconds.
    pub fn get_min_delay_ns(&self) -> u64 {
        self.min_delay_ns
    }

    /// Gets the maximum delay between checks.
    ///
    /// This is the longest wait time the scheduler will use, in nanoseconds.
    pub fn get_max_delay_ns(&self) -> u64 {
        self.max_delay_ns
    }

    /// Figures out how long to wait before checking again.
    ///
    /// This is the main function that decides the delay based on the current time. It uses the bell
    /// curve idea to make the delay shorter when the event is near and longer when it’s far away.
    ///
    /// ### How It Works Step-by-Step
    /// 1. **Find the Distance**: It calculates how far the current time is from the expected moment,
    ///    whether before or after. This distance is always a positive number.
    /// 2. **Check for No Variation**: If the standard deviation is zero (no variation), it does one of two things:
    ///    - If the current time is after the expected moment, it uses the minimum delay (check right away).
    ///    - If before, it waits exactly until the expected moment.
    /// 3. **Use the Bell Curve**: If there’s variation (standard deviation isn’t zero):
    ///    - It squares the distance to emphasize bigger gaps.
    ///    - It compares this to three times the standard deviation squared, which sets the bell curve’s width.
    ///    - It calculates a ratio that grows as the distance increases, making the delay longer.
    /// 4. **Adjust the Delay**: It starts with the minimum delay and adds an amount based on the ratio,
    ///    but never goes over the maximum delay.
    /// 5. **Handle Weird Cases**: If the maximum delay is less than the minimum (which shouldn’t happen),
    ///    it averages them to avoid problems.
    ///
    /// ### What You Give It
    /// - `current_time_ns`: The current time in nanoseconds.
    ///
    /// ### What You Get Back
    /// - The delay in nanoseconds before the next check.
    pub fn compute_next_delay_ns(&self, current_time_ns: u64) -> u64 {
        // Step 1: Calculate how far the current time is from the expected moment
        let distance = if current_time_ns > self.expected_moment_ns {
            current_time_ns - self.expected_moment_ns
        } else {
            self.expected_moment_ns - current_time_ns
        };
        // Square the distance to use in the bell curve calculation
        let distance_squared: u128 = distance as u128 * distance as u128;

        // Step 2: Calculate three standard deviations to define the bell curve’s width
        let three_sigma = 3 * self.std_dev_ns;
        let s: u128 = three_sigma as u128 * three_sigma as u128;

        // Step 3: Ensure max and min delays make sense (fix if max < min)
        let max_delay_ns;
        let min_delay_ns;
        if self.max_delay_ns < self.min_delay_ns {
            // If max is less than min, average them to avoid errors
            let avg = (self.max_delay_ns + self.min_delay_ns) >> 1;
            max_delay_ns = avg;
            min_delay_ns = avg;
        } else {
            max_delay_ns = self.max_delay_ns;
            min_delay_ns = self.min_delay_ns;
        }

        // Step 4: Handle the no-variation case (standard deviation = 0)
        if s == 0 {
            return if current_time_ns > self.expected_moment_ns {
                // If we’re past the moment, check immediately with the minimum delay
                self.min_delay_ns
            } else {
                // If we’re before, wait exactly until the expected moment
                self.expected_moment_ns - current_time_ns
            };
        }

        // Step 5: Calculate the delay using the bell curve
        // How much room we have between min and max delays
        let delta_delay = max_delay_ns - min_delay_ns;
        // Ratio of distance squared to three standard deviations squared, scaled by the delay range
        let ratio = ((distance_squared * delta_delay as u128) / s) as u64;
        // Start with min delay and add the bell curve adjustment
        let delay_ns = min_delay_ns + ratio;

        // Step 6: Make sure the delay doesn’t go over the maximum
        delay_ns.min(max_delay_ns)
    }
}

/// Tests to make sure the scheduler works as expected in different situations.
#[cfg(test)]
mod tests {
    use super::PollScheduler;

    /// Checks the delay when the current time is exactly at the expected moment.
    #[test]
    fn test_at_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000; // 5 seconds
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000); // 1 second
        scheduler.set_min_delay_ns(10_000_000);  // 10 milliseconds
        scheduler.set_max_delay_ns(2_000_000_000); // 2 seconds

        let delay = scheduler.compute_next_delay_ns(expected_moment);
        assert_eq!(delay, 10_000_000); // Should use the minimum delay
    }

    /// Checks the delay when the current time is a little before the expected moment.
    #[test]
    fn test_slightly_before_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 100_000_000; // 100 milliseconds before
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((100_000_000_u128 * 100_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    /// Checks the delay when the current time is a little after the expected moment.
    #[test]
    fn test_slightly_after_expected_moment() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment + 100_000_000; // 100 milliseconds after
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((100_000_000_u128 * 100_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    /// Checks the delay when the current time is one standard deviation before the expected moment.
    #[test]
    fn test_one_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 1_000_000_000; // 1 second before
        let delay = scheduler.compute_next_delay_ns(current_time);
        let expected_delay = 10_000_000 + (((1_000_000_000_u128 * 1_000_000_000_u128) * 1_990_000_000_u128) / 9_000_000_000_000_000_000_u128) as u64;
        assert_eq!(delay, expected_delay);
    }

    /// Checks the delay when the current time is three standard deviations before the expected moment.
    #[test]
    fn test_three_std_dev_before() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 3_000_000_000; // 3 seconds before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should use the maximum delay
    }

    /// Checks the delay when the current time is three standard deviations after the expected moment.
    #[test]
    fn test_three_std_dev_after() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment + 3_000_000_000; // 3 seconds after
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should use the maximum delay
    }

    /// Checks the delay when there’s no variation (standard deviation is zero).
    #[test]
    fn test_zero_std_dev() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(0);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 1_000_000_000; // 1 second before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 1_000_000_000); // Should wait until the expected moment
    }

    /// Checks the delay when the current time is far beyond the expected moment.
    #[test]
    fn test_beyond_max_delay() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_time = expected_moment - 4_000_000_000; // 4 seconds before
        let delay = scheduler.compute_next_delay_ns(current_time);
        assert_eq!(delay, 2_000_000_000); // Should use the maximum delay
    }

    /// Checks a series of delays for different times relative to the expected moment.
    #[test]
    fn test_poll_sequence_with_delay_vector() {
        let mut scheduler = PollScheduler::new();
        let expected_moment = 5_000_000_000;
        scheduler.set_expected_moment_ns(expected_moment);
        scheduler.set_std_dev_ns(1_000_000_000);
        scheduler.set_min_delay_ns(10_000_000);
        scheduler.set_max_delay_ns(2_000_000_000);

        let current_times = [
            expected_moment - 3_000_000_000, // 3 seconds before
            expected_moment - 1_000_000_000, // 1 second before
            expected_moment - 500_000_000,   // 0.5 seconds before
            expected_moment,                 // Right at the moment
            expected_moment + 500_000_000,   // 0.5 seconds after
            expected_moment + 1_000_000_000, // 1 second after
            expected_moment + 3_000_000_000, // 3 seconds after
        ];

        let expected_delays = [
            2_000_000_000, // Far away, so maximum delay
            231_111_111,   // A bit closer, delay increases from minimum
            65_277_777,    // Even closer, shorter delay
            10_000_000,    // At the moment, minimum delay
            65_277_777,    // Slightly after, short delay
            231_111_111,   // A bit farther, longer delay
            2_000_000_000, // Far away, maximum delay
        ];

        for (i, current_time) in current_times.iter().enumerate() {
            let computed_delay = scheduler.compute_next_delay_ns(*current_time);
            assert_eq!(
                computed_delay, expected_delays[i],
                "Delay mismatch at step {}: expected {}, got {}",
                i, expected_delays[i], computed_delay
            );
        }
    }
}