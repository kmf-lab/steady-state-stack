//! Property tests for telemetry setup backoff scaling.

use crate::ss_proptest;
use crate::telemetry::setup::calculate_exponential_channel_backoff;
use proptest::prelude::*;

ss_proptest! {
    /// Property: backoff is monotonic as vacant space shrinks (fixed capacity).
    #[test]
    // ss[verify telemetry.builtin-server]
    // ss[verify verify.process.proptest]
    fn proptest_exponential_backoff_monotonic(capacity in 8usize..512) {
        let cap = capacity.next_power_of_two();
        let mut prev = calculate_exponential_channel_backoff(cap, cap);
        for vacant in (0..cap).rev() {
            let backoff = calculate_exponential_channel_backoff(cap, vacant);
            prop_assert!(backoff >= prev);
            prev = backoff;
        }
    }

    /// Property: full channel yields larger backoff than half-full for power-of-two capacities.
    #[test]
    // ss[verify telemetry.builtin-server]
    // ss[verify verify.process.proptest]
    fn proptest_exponential_backoff_full_exceeds_half(cap_log2 in 3u32..10) {
        let capacity = 1usize << cap_log2;
        let full = calculate_exponential_channel_backoff(capacity, 0);
        let half = calculate_exponential_channel_backoff(capacity, capacity / 2);
        prop_assert!(full > half);
        prop_assert!(full >= 1);
    }
}
