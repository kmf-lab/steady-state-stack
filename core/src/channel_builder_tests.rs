//! Tests extracted from `channel_builder.rs` so the production file stays under the 1,200-line budget.
//! Nested test mods keep their own imports; `use super::*` at the top re-exports the parent.

use super::*;

#[cfg(test)]
// ss[related channel.lazy.defer-allocation]
impl ChannelBuilder {
    /// Isolated builder for unit tests (private shutdown vec, not shared with `Default`).
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn test_channel_builder() -> Self {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        ChannelBuilder::new(channel_count, oneshot_shutdown_vec, 1000)
    }
}

#[cfg(test)]
// ss[related channel.lazy.defer-allocation]
mod tests_inputs {
    // ss[related philosophy.structural-hierarchy]
    use super::super::*;
    // ss[related philosophy.structural-hierarchy]
    use proptest::prelude::*;

    ss_proptest! {

        /// Property: Filled::percentage succeeds iff p ∈ [0, 100].
        #[test]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_filled_percentage_valid_range(value in crate::proptest_support::percent_f32()) {
            let filled = Filled::percentage(value);
            if (0.0..=100.0).contains(&value) {
                prop_assert!(filled.is_some());
            } else {
                prop_assert!(filled.is_none());
            }
        }

        /// Property: Filled::pN() ≡ Percentage(N*10, 100) for N = 1..10.
        #[test]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_filled_pN_equivalence(n in 1u64..=10u64) {
            let expected = Filled::Percentage(n * 10, 100);
            let actual = match n {
                1 => Filled::p10(),
                2 => Filled::p20(),
                3 => Filled::p30(),
                4 => Filled::p40(),
                5 => Filled::p50(),
                6 => Filled::p60(),
                7 => Filled::p70(),
                8 => Filled::p80(),
                9 => Filled::p90(),
                10 => Filled::p100(),
                _ => unreachable!(),
            };
            prop_assert_eq!(actual, expected);
        }

        /// Property: Rate rational_ms scales consistently across time units.
        #[test]
        // ss[verify channel.default-capacity]
        // ss[verify verify.process.proptest]
        fn proptest_rate_rational_ms_consistent(units in 1u64..10_000u64) {
            prop_assert_eq!(Rate::per_millis(units).rational_ms(), (units, 1));
            prop_assert_eq!(Rate::per_seconds(units).rational_ms(), (units, 1000));
            prop_assert_eq!(Rate::per_minutes(units).rational_ms(), (units, 60_000));
            prop_assert_eq!(Rate::per_hours(units).rational_ms(), (units, 3_600_000));
            prop_assert_eq!(Rate::per_days(units).rational_ms(), (units, 86_400_000));
        }

        /// Property: longer denominators imply lower per-ms rate for the same numerator.
        #[test]
        // ss[verify channel.default-capacity]
        // ss[verify verify.process.proptest]
        fn proptest_rate_denominator_ordering(units in 1u64..1000u64) {
            let (_, d_millis) = Rate::per_millis(units).rational_ms();
            let (_, d_seconds) = Rate::per_seconds(units).rational_ms();
            let (_, d_minutes) = Rate::per_minutes(units).rational_ms();
            prop_assert!(d_millis < d_seconds);
            prop_assert!(d_seconds < d_minutes);
        }

        /// Property: `with_compute_refresh_window_floor` matches shared frame math.
        #[test]
        // ss[verify channel.lazy.defer-allocation]
        // ss[verify verify.process.proptest]
        fn proptest_refresh_window_floor_matches_shared_math(
            frame_rate_ms in 10u64..500,
            refresh_secs in 1u64..5,
            window_secs in 5u64..30,
        ) {
            let channel_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let oneshot_shutdown_vec = std::sync::Arc::new(futures_util::lock::Mutex::new(Vec::new()));
            let builder = ChannelBuilder::new(channel_count, oneshot_shutdown_vec, frame_rate_ms);
            let refresh = Duration::from_secs(refresh_secs);
            let window = Duration::from_secs(window_secs);
            let configured = builder.with_compute_refresh_window_floor(refresh, window);
            let expected = crate::telemetry_window::compute_refresh_window_frames(
                frame_rate_ms as u128,
                refresh,
                window,
            );
            prop_assert_eq!(
                (configured.refresh_rate_in_bits, configured.window_bucket_in_bits),
                expected
            );
        }

        /// Property: `with_no_refresh_window` zeros telemetry window bits.
        #[test]
        // ss[verify channel.lazy.defer-allocation]
        // ss[verify verify.process.proptest]
        fn proptest_no_refresh_window_zeros_bits(frame_rate_ms in 10u64..1000) {
            let channel_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let oneshot_shutdown_vec = std::sync::Arc::new(futures_util::lock::Mutex::new(Vec::new()));
            let builder = ChannelBuilder::new(channel_count, oneshot_shutdown_vec, frame_rate_ms)
                .with_avg_filled()
                .with_avg_rate();
            let disabled = builder.with_no_refresh_window();
            prop_assert_eq!(disabled.refresh_rate_in_bits, 0);
            prop_assert_eq!(disabled.window_bucket_in_bits, 0);
        }

        /// Property: builder methods return new instances without mutating the original.
        #[test]
        // ss[verify channel.lazy.defer-allocation]
        // ss[verify verify.process.proptest]
        fn proptest_builder_clone_immutability(capacity in 1usize..4096) {
            let channel_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let oneshot_shutdown_vec = std::sync::Arc::new(futures_util::lock::Mutex::new(Vec::new()));
            let builder = ChannelBuilder::new(channel_count, oneshot_shutdown_vec, 40);
            let original_cap = builder.capacity;
            let _derived = builder.with_capacity(capacity).with_avg_filled().with_no_totals();
            prop_assert_eq!(builder.capacity, original_cap);
            prop_assert!(!builder.avg_filled);
        }

        /// Property: `to_meta_data` reflects configured capacity and telemetry flags.
        #[test]
        // ss[verify channel.lazy.defer-allocation]
        // ss[verify verify.process.proptest]
        fn proptest_to_meta_data_preserves_flags(
            capacity in 1usize..4096,
            enable_avg_filled in any::<bool>(),
            enable_avg_rate in any::<bool>(),
        ) {
            let channel_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let oneshot_shutdown_vec = std::sync::Arc::new(futures_util::lock::Mutex::new(Vec::new()));
            let mut builder = ChannelBuilder::new(channel_count, oneshot_shutdown_vec, 40)
                .with_capacity(capacity);
            if enable_avg_filled {
                builder = builder.with_avg_filled();
            }
            if enable_avg_rate {
                builder = builder.with_avg_rate();
            }
            let meta = builder.to_meta_data("u64", 8);
            prop_assert_eq!(meta.capacity, capacity);
            prop_assert_eq!(meta.avg_filled, enable_avg_filled);
            prop_assert_eq!(meta.avg_rate, enable_avg_rate);
        }

        /// Property: trigger configuration accumulates without mutating the original builder.
        #[test]
        // ss[verify channel.backpressure-never-drop]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_accumulation_immutable(
            trigger_count in 1usize..4,
        ) {
            let channel_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let oneshot_shutdown_vec = std::sync::Arc::new(futures_util::lock::Mutex::new(Vec::new()));
            let builder = ChannelBuilder::new(channel_count, oneshot_shutdown_vec, 40);
            let mut configured = builder.clone();
            for i in 0..trigger_count {
                configured = configured.with_filled_trigger(
                    Trigger::AvgAbove(Filled::p10()),
                    if i % 2 == 0 { AlertColor::Yellow } else { AlertColor::Orange },
                );
            }
            prop_assert_eq!(configured.trigger_filled.len(), trigger_count);
            prop_assert!(builder.trigger_filled.is_empty());
        }
    }
}

#[cfg(test)]
// ss[related philosophy.structural-hierarchy]
pub(crate) mod test_builder {
    // ss[related channel.lazy.defer-allocation]
    use super::*;

    // ss[related philosophy.structural-hierarchy]
    use crate::actor_builder_units::Percentile;
    // ss[related channel.lazy.defer-allocation]
    use crate::steady_rx::RxMetaDataProvider;
    // ss[related philosophy.structural-hierarchy]
    use crate::distributed::aqueduct_stream::StreamIngress;

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_new() {
        let channel_count = Arc::new(AtomicUsize::new(0));
        let oneshot_shutdown_vec = Arc::new(Mutex::new(Vec::new()));
        let frame_rate_ms = 1000;

        let builder = ChannelBuilder::new(channel_count.clone(), oneshot_shutdown_vec.clone(), frame_rate_ms);

        assert_eq!(builder.capacity, DEFAULT_CAPACITY);
        //  assert_eq!(builder.labels, &[]);
        assert_eq!(builder.frame_rate_ms, frame_rate_ms);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_capacity() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_capacity(128);

        assert_eq!(new_builder.capacity, 128);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_type() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_type();

        assert!(new_builder.show_type);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_avg_filled() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_filled();

        assert!(new_builder.avg_filled);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_filled_max() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_filled_max();

        assert!(new_builder.max_filled);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_filled_min() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_filled_min();

        assert!(new_builder.min_filled);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_avg_rate() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_rate();

        assert!(new_builder.avg_rate);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_avg_latency() {
        let builder = create_test_channel_builder();
        let new_builder = builder.with_avg_latency();

        assert!(new_builder.avg_latency);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_labels() {
        let builder = create_test_channel_builder();
        let labels = &["label1", "label2"];
        let new_builder = builder.with_labels(labels, true);

        assert_eq!(new_builder.labels, labels);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_filled_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_filled_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_filled.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_rate_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_rate_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_rate.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_latency_standard_deviation() {
        let builder = create_test_channel_builder();
        let std_dev = StdDev::one();
        let new_builder = builder.with_latency_standard_deviation(std_dev);

        assert_eq!(new_builder.std_dev_latency.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_filled_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_filled_percentile(percentile);

        assert_eq!(new_builder.percentiles_filled.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_rate_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_rate_percentile(percentile);

        assert_eq!(new_builder.percentiles_rate.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_latency_percentile() {
        let builder = create_test_channel_builder();
        let percentile = Percentile::p50();
        let new_builder = builder.with_latency_percentile(percentile);

        assert_eq!(new_builder.percentiles_latency.len(), 1);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_to_meta_data() {
        let builder = create_test_channel_builder();
        let type_name = "TestType";
        let type_byte_count = 4;
        let meta_data = builder.to_meta_data(type_name, type_byte_count);

        assert_eq!(meta_data.capacity, builder.capacity);
        assert_eq!(meta_data.labels, builder.labels);
        assert_eq!(meta_data.bundle_index, builder.bundle_index);
    }

    #[test]
    #[should_panic(expected = "capacity")]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_to_meta_data_zero_capacity_panics() {
        let builder = create_test_channel_builder().with_capacity(0);
        let _ = builder.to_meta_data("T", 4);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_no_totals() {
        let builder = create_test_channel_builder();
        let configured = builder.with_no_totals();
        assert!(!configured.show_total);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_no_refresh_window() {
        let builder = create_test_channel_builder();
        let configured = builder.with_no_refresh_window();
        assert_eq!(configured.refresh_rate_in_bits, 0);
        assert_eq!(configured.window_bucket_in_bits, 0);
    }

    #[test]
    // ss[verify channel.lazy.defer-allocation]
    pub(crate) fn test_channel_builder_with_latency_trigger() {
        let builder = create_test_channel_builder();
        let configured = builder.with_latency_trigger(
            Trigger::AvgAbove(Duration::from_millis(50)),
            AlertColor::Orange,
        );
        assert_eq!(configured.trigger_latency.len(), 1);
    }

    // ss[verify channel.memory-usage-telemetry]
    #[test]
    // ss[related philosophy.structural-hierarchy]
    pub(crate) fn test_channel_builder_with_memory_usage_sets_flag() {
        let builder = create_test_channel_builder();
        let with_mem = builder.with_memory_usage();
        assert!(with_mem.show_memory);
    }

    #[test]
    // ss[verify channel.dynamic-payload-estimate]
    pub(crate) fn test_with_dynamic_payload_estimate_footprint() {
        const DEPTH: usize = 16384;
        const PER_SLOT: usize = 256024;
        const MSG_SIZE: usize = 24;
        let builder = create_test_channel_builder()
            .with_capacity(DEPTH)
            .with_dynamic_payload_estimate(PER_SLOT)
            .with_memory_usage();
        let (_, rx) = builder.eager_build_internal::<[u8; MSG_SIZE]>();
        let meta = crate::steady_rx::RxMetaDataProvider::meta_data(&rx);
        assert_eq!(meta.ring_slot_byte_count, MSG_SIZE);
        assert_eq!(meta.dynamic_per_slot_estimate, Some(PER_SLOT));

        let mut computer = crate::channel_stats::ChannelStatsComputer::default();
        computer.init(
            &meta,
            crate::ActorName::new("a", None),
            crate::ActorName::new("b", None),
            1000,
        );
        assert_eq!(computer.ring_memory_footprint, DEPTH * MSG_SIZE);
        assert_eq!(computer.dynamic_memory_footprint, DEPTH * PER_SLOT);
        assert_eq!(crate::monitor::channel_ring_memory_footprint(&meta), DEPTH * MSG_SIZE);
        assert_eq!(
            crate::monitor::channel_dynamic_memory_footprint(&meta),
            DEPTH * PER_SLOT
        );
    }

    #[test]
    // ss[verify channel.dynamic-payload-estimate]
    pub(crate) fn test_dynamic_payload_estimate_looks_capacity_scaled_heuristic() {
        assert!(dynamic_payload_estimate_looks_capacity_scaled(16384, 256024 * 16384));
        assert!(!dynamic_payload_estimate_looks_capacity_scaled(16384, 256024));
    }

    #[test]
    // ss[verify bundle.girth-const-generic]
    pub(crate) fn test_channel_builder_bundle_index() {
        let builder = create_test_channel_builder();
        let (_, rx_bundle) = builder.build_channel_bundle::<i32, 3>();
        
        for (i, rx) in rx_bundle.iter().enumerate() {
            let meta = rx.clone().meta_data();
            assert_eq!(meta.bundle_index, Some(i));
        }
    }

    #[test]
    // ss[verify channel.stream-dual-buffer]
    pub(crate) fn test_stream_builder_bundle_index() {
        let builder = create_test_channel_builder();
        let (_, rx_bundle) = builder.build_stream_bundle::<StreamIngress, 3>(8);
        
        for (i, rx) in rx_bundle.iter().enumerate() {
            let meta = rx.clone().meta_data();
            assert_eq!(meta.bundle_index, Some(i));
        }
    }

    // ss[related channel.lazy.defer-allocation]
    fn create_test_channel_builder() -> ChannelBuilder {
        ChannelBuilder::test_channel_builder()
    }
}
