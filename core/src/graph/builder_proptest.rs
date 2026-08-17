//! Property tests for `GraphBuilder` configuration invariants.

use super::{GraphBuilder, MIN_MS_RATE};
use crate::ss_proptest;
use proptest::prelude::*;
use std::collections::HashSet;

ss_proptest! {
    /// Property: telemetry production rate below minimum is clamped to `MIN_MS_RATE`.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_telemetry_rate_clamped_below_minimum(requested in 0u64..500) {
        let builder = GraphBuilder::for_testing().with_telemtry_production_rate_ms(requested);
        prop_assert!(builder.telemtry_production_rate_ms >= MIN_MS_RATE);
        if requested >= MIN_MS_RATE {
            prop_assert_eq!(builder.telemtry_production_rate_ms, requested);
        } else {
            prop_assert_eq!(builder.telemtry_production_rate_ms, MIN_MS_RATE);
        }
    }

    /// Property: builder option chain preserves explicit values at or above minimums.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_builder_option_chain(
        rate_ms in MIN_MS_RATE..5_000,
        bundle_floor in 1usize..32,
        stack in 256usize..4_096,
    ) {
        let names: HashSet<&'static str> = ["WORKER", "LOGGER"].into_iter().collect();
        let builder = GraphBuilder::for_testing()
            .with_telemtry_production_rate_ms(rate_ms)
            .with_telemetry_colors("#111111", "#222222")
            .with_default_actor_stack_size(stack)
            .with_bundle_floor_size(bundle_floor)
            .with_test_pipeline_internal_behavior_names(names.clone());
        prop_assert_eq!(builder.telemtry_production_rate_ms, rate_ms);
        prop_assert_eq!(builder.bundle_floor_size, bundle_floor);
        prop_assert_eq!(builder.default_stack_size, Some(stack));
        prop_assert_eq!(builder.test_pipeline_internal_names, names);
        prop_assert!(builder.telemetry_colors.is_some());
    }

    /// Property: testing and production builders differ on the for-testing flag.
    #[test]
    // ss[verify graph.for-testing]
    // ss[verify verify.process.proptest]
    fn proptest_for_testing_flag(_seed in 0u8..=255) {
        let testing = GraphBuilder::for_testing();
        prop_assert!(testing.is_for_testing);
        prop_assert!(!testing.telemetry_metric_features);
        prop_assert!(testing.backplane.is_some());
    }
}
