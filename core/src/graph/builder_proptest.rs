//! Property tests for `GraphBuilder` configuration invariants.

// ss[related philosophy.structural-hierarchy]
use super::{GraphBuilder, MIN_MS_RATE};
// ss[related philosophy.structural-hierarchy]
use crate::ss_proptest;
// ss[related philosophy.structural-hierarchy]
use proptest::prelude::*;
// ss[related philosophy.structural-hierarchy]
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
        prop_assert!(!testing.strict_persist);
        let strict = testing.with_strict_persist();
        prop_assert!(strict.strict_persist);
    }

    /// Property: `with_pack_slots` pins slot budget for dynamic troupe packing.
    #[test]
    // ss[verify troupe.dynamic-slot-budget]
    // ss[verify verify.process.proptest]
    fn proptest_pack_slots_pinned(
        slots in 1usize..16,
    ) {
        let builder = GraphBuilder::for_testing().with_pack_slots(slots);
        prop_assert_eq!(builder.pack_slots, Some(slots));
    }
}

#[cfg(test)]
mod for_lambda_tests {
    use super::*;

    // ss[verify graph.for-lambda]
    #[test]
    #[should_panic(expected = "should not call for_lambda in tests")]
    fn for_lambda_panics_under_cfg_test() {
        let _ = GraphBuilder::for_lambda();
    }

    // ss[verify graph.for-testing]
    #[test]
    #[should_panic(expected = "should not call for_production in tests")]
    fn default_panics_under_cfg_test() {
        let _ = GraphBuilder::default();
    }

    // ss[verify graph.for-testing]
    #[test]
    #[should_panic(expected = "should not call for_production in tests")]
    fn for_production_panics_under_cfg_test() {
        let _ = GraphBuilder::for_production();
    }

    // ss[verify graph.for-lambda]
    // ss[verify state.dirty-at-park]
    #[test]
    fn with_strict_persist_enables_flag() {
        let b = GraphBuilder::for_testing().with_strict_persist();
        assert!(b.strict_persist);
        assert!(!b.telemetry_metric_features);
        assert_eq!(b.pack_slots, Some(1));
    }

    // ss[verify graph.lambda.shutdown-hook]
    #[test]
    fn build_registers_ctrlc_shutdown_path() {
        // GraphBuilder::build installs ctrlc → request_shutdown + watch_shutdown (sub-second drain).
        let _g = GraphBuilder::for_testing().with_strict_persist().build(());
    }
}
