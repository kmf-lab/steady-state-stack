use std::future::Future;
use std::time::{Duration, Instant};
use async_std::task;

/// this implementation requires we never schedule for more than this in the future
/// the reason for this is to enable shutdown signals to be detected so we can exit clean
pub const MAX_SLEEP_SECONDS: u64 = 10;

/// A general-purpose scheduler for executing async tasks with specified delays.
///
/// This scheduler manages a fixed number of tasks, identified by indices from 0 to `MAX_ITEMS - 1`.
/// Each task has a scheduled execution time, and an async function is called when that time arrives,
/// returning the next delay for the task.
///
/// # Type Parameters
/// - `MAX_ITEMS`: The number of tasks the scheduler can handle, set at compile time.
/// - `F`: The type of the closure that generates futures for each task.
/// - `Fut`: The future type returned by the closure, which resolves to a `Duration`.
pub struct PerpetualScheduler<const MAX_ITEMS: usize, F, Fut>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Duration>,
{
    /// The next execution time for each task.
    next_times: [Instant; MAX_ITEMS],
    /// The user-provided async function that generates a future for a given task index.
    func: F,
}

impl<const MAX_ITEMS: usize, F, Fut> PerpetualScheduler<MAX_ITEMS, F, Fut>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Duration>,
{
    /// Creates a new scheduler with initial delays for each task and an async function.
    ///
    /// # Arguments
    /// - `initial_delays`: An array of `Duration`s specifying the initial delay for each task.
    ///   For tasks after the first (index > 0), the first task's delay is subtracted to adjust timing.
    /// - `func`: A closure that takes a task index (`usize`) and returns a future resolving to the next delay.
    ///
    /// # Panics
    /// Panics if `MAX_ITEMS` is 0, as the scheduler requires at least one task to operate.
    pub fn new(initial_delays: [Duration; MAX_ITEMS], func: F) -> Self {
        assert!(MAX_ITEMS > 0, "Scheduler requires at least one task (MAX_ITEMS > 0)");

        let now = Instant::now();
        let first_delay = initial_delays[0];
        let mut next_times = [now; MAX_ITEMS];

        for i in 0..MAX_ITEMS {
            let effective_delay = if i > 0 {
                initial_delays[i].saturating_sub(first_delay)
            } else {
                initial_delays[i]
            };
            next_times[i] = now + effective_delay;
        }

        Self { next_times, func }
    }

    /// Runs single pass of the scheduler
    ///
    /// The scheduler finds the task with the earliest execution time, waits until that time,
    /// executes the task's async function, and updates its next execution time based on the returned delay.
    pub async fn run_single_pass(&mut self, now: Instant) -> Instant {
            // Find the task with the earliest next_time

            // Safe because MAX_ITEMS > 0 ensures at least one task exists
            let mut earliest_idx = 0;
            let mut earliest_time = self.next_times[0];
            for i in 1..MAX_ITEMS {
                if self.next_times[i] < earliest_time {
                    earliest_time = self.next_times[i];
                    earliest_idx = i;
                }
            }

            //now based on the next one we will seep until that time and run it
            if earliest_time > now {
                let dur = earliest_time - now;
                assert!(dur.as_secs() < MAX_SLEEP_SECONDS);
                task::sleep(dur).await;
            }

            // Execute the task's async function and get the next delay
            let delay = (self.func)(earliest_idx).await;
            assert!(delay.as_secs() < MAX_SLEEP_SECONDS);

            // Update the next execution time for this task
            let now = Instant::now();
            self.next_times[earliest_idx] = now + delay;
            now
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc};
    use super::*;
    use std::time::Duration;
    use async_std::sync::Mutex;
    use async_std::task;

    #[async_std::test]
    async fn test_perpetual_scheduler() {
        let initial_delays = [Duration::from_millis(10), Duration::from_millis(20), Duration::from_millis(30)];

        // Create a shared, mutable array of call counts
        let call_counts = Arc::new(Mutex::new([0; 3]));
        let call_counts_clone = call_counts.clone();

        // Initialize the scheduler with a closure that clones the Arc for each call
        let mut scheduler = PerpetualScheduler::new(initial_delays, move |idx| {
            let call_counts_clone2 = call_counts_clone.clone();
            async move {
                let mut counts = call_counts_clone2.lock().await; // Lock the mutex asynchronously
                counts[idx] += 1;
                Duration::from_millis(50) // Reschedule every 50ms
            }
        });

        // Run the scheduler in a background task
        let handle = task::spawn(async move {
            let mut now = Instant::now();
            loop { //in production this loop would check for shutdown signal
                now = scheduler.run_single_pass(now).await;
            }
        });

        // Let the scheduler run briefly
        task::sleep(Duration::from_millis(250)).await;
        handle.cancel().await;

        // Check the results
        let counts = call_counts.lock().await;
        assert!(counts[0] > 0);
        assert!(counts[1] > 0);
        assert!(counts[2] > 0);
    }
}