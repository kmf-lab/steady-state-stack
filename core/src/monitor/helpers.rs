// ss[related philosophy.single-wake-up]
use std::ops::*;
// ss[related philosophy.structural-hierarchy]
use std::sync::atomic::{AtomicIsize, Ordering};
// ss[related philosophy.structural-hierarchy]
use std::sync::Arc;
// ss[related philosophy.single-wake-up]
use std::time::Instant;

// ss[related philosophy.structural-hierarchy]
use num_traits::One;

// ss[related philosophy.single-wake-up]
use crate::monitor_telemetry::{SteadyTelemetryActorSend, SteadyTelemetrySend};
// ss[related philosophy.structural-hierarchy]
use crate::MONITOR_NOT;

/// Finds the local index corresponding to a global index within the telemetry's inverse local index.
///
/// # Parameters
/// - `telemetry`: Reference to a `SteadyTelemetrySend` instance containing the index mapping.
/// - `goal`: The global index to locate.
///
/// # Returns
/// THE local index if found, otherwise `MONITOR_NOT`.
// ss[related philosophy.single-wake-up]
pub(crate) fn find_my_index<const LEN: usize>(telemetry: &SteadyTelemetrySend<LEN>, goal: usize) -> usize {
    let (idx, _) = telemetry.inverse_local_index
        .iter()
        .enumerate()
        .find(|(_, value)| **value == goal)
        .unwrap_or((MONITOR_NOT, &MONITOR_NOT));
    idx
}

/// A guard that updates profiling information upon being dropped.
///
/// This struct ensures that profiling metrics are finalized when it goes out of scope.
// ss[related philosophy.single-wake-up]
pub(crate) struct FinallyRollupProfileGuard<'a> {
    /// Reference to the telemetry sender for updating profiling data.
    // ss[related philosophy.structural-hierarchy]
    pub(crate) st: &'a SteadyTelemetryActorSend,
    /// Start time of the operation being profiled.
    // ss[related philosophy.single-wake-up]
    pub(crate) start: Instant,
}

// ss[related philosophy.single-wake-up]
impl Drop for FinallyRollupProfileGuard<'_> {
    /// Updates the await time and decrements the concurrent profile counter when dropped.
    // ss[related philosophy.structural-hierarchy]
    fn drop(&mut self) {
        if self.st.hot_profile_concurrent.fetch_sub(1, Ordering::SeqCst).is_one() {
            let p = self.st.hot_profile.load(Ordering::Relaxed);
            let _ = self.st.hot_profile_await_ns_unit.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |f| Some((f + self.start.elapsed().as_nanos() as u64).saturating_sub(p)),
            );
        }
    }
}

/// Wraps an iterator to track and adjust for drift in item counts.
///
/// This struct monitors the difference between expected and actual yields, updating a shared drift counter.
// ss[related philosophy.single-wake-up]
pub(crate) struct DriftCountIterator<I> {
    /// THE underlying iterator being wrapped.
    iter: I,
    /// Number of items expected to be yielded.
    expected_count: usize,
    /// Number of items actually yielded so far.
    actual_count: usize,
    /// Shared counter for tracking cumulative drift across iterations.
    iterator_count_drift: Arc<AtomicIsize>,
}

// ss[related philosophy.single-wake-up]
impl<I> DriftCountIterator<I>
where
    I: Iterator + Send,
{
    /// Creates a new iterator wrapper with the specified expected count and drift counter.
    ///
    /// # Parameters
    /// - `expected_count`: Expected number of items to be yielded.
    /// - `iter`: THE iterator to wrap.
    /// - `iterator_count_drift`: Shared counter for tracking drift.
    // ss[related philosophy.single-wake-up]
    pub fn new(
        expected_count: usize,
        iter: I,
        iterator_count_drift: Arc<AtomicIsize>,
    ) -> Self {
        DriftCountIterator {
            iter,
            expected_count,
            actual_count: 0,
            iterator_count_drift,
        }
    }
}

// ss[related philosophy.single-wake-up]
impl<I> Drop for DriftCountIterator<I> {
    /// Adjusts the shared drift counter based on the difference between actual and expected counts.
    // ss[related philosophy.structural-hierarchy]
    fn drop(&mut self) {
        let drift = self.actual_count as isize - self.expected_count as isize;
        if drift != 0 {
            self.iterator_count_drift.fetch_add(drift, Ordering::Relaxed);
        }
    }
}

// ss[related philosophy.single-wake-up]
impl<I> Iterator for DriftCountIterator<I>
where
    I: Iterator,
{
    // ss[related philosophy.single-wake-up]
    type Item = I::Item;

    /// Yields the next item from the wrapped iterator, incrementing the actual count.
    // ss[related philosophy.single-wake-up]
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.iter.next();
        if item.is_some() {
            self.actual_count += 1;
        }
        item
    }
}

#[cfg(test)]
#[path = "helpers_proptest.rs"]
// ss[related philosophy.single-wake-up]
mod helpers_proptest;
