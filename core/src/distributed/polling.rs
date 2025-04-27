/// A module for computing polling schedules based on a normal distribution.
/// This is designed to be simple, fast, and easy to use for engineers, with verbose
/// comments explaining every step. It generates polling times for message arrivals,
/// respecting minimum and maximum delay bounds.
pub mod polling_schedule {
    use statrs::distribution::{Normal, ContinuousCDF};
    use std::vec::Vec;

    /// Configuration struct for generating a polling schedule.
    /// Holds all the settings needed to compute polling times based on a normal distribution.
    pub struct PollingScheduleConfig {
        /// The average time gap between message arrivals (in milliseconds).
        /// This is the center of the distribution.
        average_gap: f64,

        /// How much the time gaps vary (in milliseconds).
        /// Think of this as the spread of the distribution. Must be > 0 for normal calculations,
        /// but if it’s 0, we’ll handle it by returning a single time.
        gap_variation: f64,

        /// What fraction of the distribution to cover (e.g., 0.99999 for 99.999%).
        /// Must be between 0 and 1. This controls how much of the possible range we include.
        coverage_fraction: f64,

        /// How many polling points to generate (e.g., 10 means 11 points, including start and end).
        /// This splits the covered range into equal probability steps.
        number_of_points: usize,

        /// The smallest delay allowed (in milliseconds).
        /// No polling time will be less than this.
        minimum_delay: f64,

        /// The largest delay allowed (in milliseconds).
        /// No polling time will be more than this.
        maximum_delay: f64,
    }

    impl PollingScheduleConfig {
        /// Creates a new polling schedule configuration with super obvious argument names.
        ///
        /// # Arguments
        /// - `average_gap`: The average time between message arrivals (in milliseconds).
        /// - `gap_variation`: The standard deviation of the time gaps (in milliseconds).
        /// - `coverage_fraction`: The fraction of the distribution to cover (0 to 1, e.g., 0.99999).
        /// - `number_of_points`: The number of polling points to generate (>= 1).
        /// - `minimum_delay`: The smallest allowed polling time (in milliseconds).
        /// - `maximum_delay`: The largest allowed polling time (in milliseconds).
        ///
        /// # Returns
        /// A new `PollingScheduleConfig` instance ready to compute the schedule.
        ///
        /// # Notes
        /// - We don’t do heavy validation here to keep it simple, but you should ensure:
        ///   - `gap_variation` is non-negative.
        ///   - `coverage_fraction` is between 0 and 1.
        ///   - `number_of_points` is at least 1 for a useful schedule.
        ///   - `minimum_delay` <= `maximum_delay`.
        pub fn new(
            average_gap: f64,
            gap_variation: f64,
            coverage_fraction: f64,
            number_of_points: usize,
            minimum_delay: f64,
            maximum_delay: f64,
        ) -> Self {
            Self {
                average_gap,
                gap_variation,
                coverage_fraction,
                number_of_points,
                minimum_delay,
                maximum_delay,
            }
        }

        /// Computes the polling schedule based on the configuration.
        ///
        /// This generates a list of times (in milliseconds) when you should poll for messages.
        /// The times are spaced to cover equal chunks of probability in the distribution,
        /// and they always stay within `minimum_delay` and `maximum_delay`.
        ///
        /// # Returns
        /// A vector of polling times (in milliseconds). If `gap_variation` is 0,
        /// you get a single time at `average_gap` (adjusted to fit the bounds).
        ///
        /// # How It Works
        /// 1. If there’s no variation, we return just one time.
        /// 2. Otherwise, we use a normal distribution to:
        ///    - Find the range that covers `coverage_fraction` of the distribution.
        ///    - Adjust that range to fit within `minimum_delay` and `maximum_delay`.
        ///    - Split that range into `number_of_points` equal probability steps.
        ///    - Convert those steps back into actual times.
        ///
        /// # Performance
        /// - Fast and low-CPU: Uses efficient math from `statrs` and a single loop.
        /// - Not super accurate with window sizes (as requested), but always respects bounds.
        pub fn compute_schedule(&self) -> Vec<f64> {
            // Check for no variation (degenerate case)
            if self.gap_variation == 0.0 {
                // If there’s no spread, all messages arrive at the average gap time.
                // We clamp it to stay within our bounds and return just that one time.
                let adjusted_time = self.average_gap.clamp(self.minimum_delay, self.maximum_delay);
                return vec![adjusted_time];
            }

            // Create a standard normal distribution (mean 0, std dev 1) for calculations.
            // This is our reference for probability math.
            let normal = Normal::new(0.0, 1.0).unwrap();

            // Figure out how far out we need to go to cover `coverage_fraction`.
            // For example, for 99.999%, we want 0.000005% in each tail (outside our range).
            let tail_fraction = (1.0 - self.coverage_fraction) / 2.0;
            let z_score = normal.inverse_cdf(1.0 - tail_fraction); // Distance in standard deviations

            // Calculate the initial range based on the average and variation.
            let initial_lower_time = self.average_gap - z_score * self.gap_variation;
            let initial_upper_time = self.average_gap + z_score * self.gap_variation;

            // Force the range to stay within our bounds.
            let lower_time = initial_lower_time.max(self.minimum_delay);
            let upper_time = initial_upper_time.min(self.maximum_delay);

            // Convert these times to z-scores (standardized distances from the average).
            let z_lower = (lower_time - self.average_gap) / self.gap_variation;
            let z_upper = (upper_time - self.average_gap) / self.gap_variation;

            // Get the probabilities at these z-scores.
            // This tells us where these times sit in the standard normal distribution.
            let prob_lower = normal.cdf(z_lower);
            let prob_upper = normal.cdf(z_upper);

            // Prepare to store our polling times.
            // We’ll make `number_of_points` points, including the start and end.
            let mut polling_times = Vec::with_capacity(self.number_of_points);

            // Generate the polling times by splitting the probability range evenly.
            for i in 0..=self.number_of_points {
                // Calculate the probability at this step.
                let step_prob = prob_lower
                    + (i as f64) * (prob_upper - prob_lower) / (self.number_of_points as f64);
                // Convert that probability back to a z-score.
                let step_z = normal.inverse_cdf(step_prob);
                // Turn the z-score into a time using our average and variation.
                let step_time = self.average_gap + self.gap_variation * step_z;
                polling_times.push(step_time);
            }

            // Return the list of polling times, which are guaranteed to be within bounds
            // because they’re based on probabilities between prob_lower and prob_upper.
            polling_times
        }
    }

    /// Example usage to show how simple this is to use.
    #[test]
    fn example_usage() {
        // Set up a schedule: average gap of 1000ms, variation of 50ms,
        // covering 99.999% of the distribution, with 10 points,
        // between 0ms and 10000ms.
        let config = PollingScheduleConfig::new(1000.0, 50.0, 0.99999, 10, 0.0, 10000.0);
        let schedule = config.compute_schedule();
        println!("Polling times: {:?}", schedule);
        assert_eq!(schedule.len(), 11); // 10 points + 1 (start)
        assert!(schedule.iter().all(|&t| t >= 0.0 && t <= 10000.0)); // Respects bounds
    }
}