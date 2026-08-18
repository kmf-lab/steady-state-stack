#[cfg(test)]
// ss[related telemetry.channel-labels]
mod channel_stats_tests {
    use std::cmp::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    use proptest::prelude::*;

    // ss[related telemetry.channel-labels]
    use crate::actor_stats::ChannelBlock;
    use crate::actor_stats::avg_rational;
    use crate::channel_stats::{ChannelStatsComputer, DOT_GREY, DOT_RED, FilledVisualMode, PLACES_TENS};
    use crate::monitor::ChannelMetaData;
    use crate::{ActorName, AlertColor, Filled, Rate, Trigger};

    // ss[related telemetry.channel-labels]
    fn mock_meta() -> Arc<ChannelMetaData> {
        Arc::new(ChannelMetaData {
            capacity: 100,
            show_total: true,
            type_byte_count: 8,
            show_type: Some("u64"),
            refresh_rate_in_bits: 1,
            window_bucket_in_bits: 1,
            ..Default::default()
        })
    }

    fn fresh_computer() -> ChannelStatsComputer {
        let mut computer = ChannelStatsComputer::default();
        computer.init(
            &mock_meta(),
            ActorName::new("src", None),
            ActorName::new("dst", None),
            1000,
        );
        computer
    }

    fn compute_frame(computer: &mut ChannelStatsComputer, send: i64, take: i64) {
        let mut label = String::new();
        let mut metric = String::new();
        computer.compute(&mut label, &mut metric, None, send, take);
    }

    // --- smoke tests (histogram / edge paths not expressible as properties) ---

    // ss[verify telemetry.channel-labels]
    #[test]
    fn test_avg_filled_percentage_none() {
        let computer = ChannelStatsComputer {
            capacity: 100,
            ..Default::default()
        };
        assert_eq!(computer.avg_filled_percentage(&50, &100), Ordering::Equal);
    }

    // ss[verify telemetry.channel-labels]
    #[test]
    fn test_avg_latency_none() {
        let computer = ChannelStatsComputer::default();
        assert_eq!(
            computer.avg_latency(&Duration::from_millis(100)),
            Ordering::Equal
        );
    }

    // ss[verify telemetry.channel-labels]
    #[test]
    fn test_zero_capacity_safety() {
        let mut computer = ChannelStatsComputer::default();
        computer.capacity = 0;
        let mut label = String::new();
        let (color, _) = computer.compute(&mut label, &mut String::new(), None, 0, 0);
        assert_eq!(color, DOT_GREY);
    }

    // ss[verify telemetry.channel-labels]
    #[test]
    fn test_histogram_creation_failure_handling() {
        use crate::actor_builder_units::Percentile;
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.percentiles_filled.push(Percentile::p50());
        computer.init(
            &Arc::new(meta),
            ActorName::new("a", None),
            ActorName::new("b", None),
            1000,
        );
        assert!(computer.build_filled_histogram);
    }

    ss_proptest! {

        /// Property: monotonic take counters yield non-decreasing total_consumed.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_total_consumed_monotonic(
            deltas in prop::collection::vec(0i64..1000, 1..20),
        ) {
            let mut computer = fresh_computer();
            let mut take = 0i64;
            let mut prev_total = 0u128;
            for delta in deltas {
                take = take.saturating_add(delta);
                let send = take.saturating_add(50);
                compute_frame(&mut computer, send, take);
                prop_assert!(
                    computer.total_consumed >= prev_total,
                    "total_consumed regressed: {} < {}",
                    computer.total_consumed,
                    prev_total
                );
                prev_total = computer.total_consumed;
            }
        }

        /// Property: avg_filled_whole_percent is in [0, 100] when defined.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_avg_filled_whole_percent_bounded(
            capacity in 1usize..10_000,
            runner in 1u128..10_000_000,
            window_bits in 0u8..4,
        ) {
            let c = ChannelStatsComputer {
                capacity,
                show_avg_filled: true,
                refresh_rate_in_bits: window_bits / 2,
                window_bucket_in_bits: window_bits - window_bits / 2,
                current_filled: Some(ChannelBlock {
                    histogram: None,
                    runner,
                    sum_of_squares: 0,
                }),
                ..Default::default()
            };
            if let Some(pct) = c.avg_filled_whole_percent() {
                prop_assert!(pct <= 100, "percent {} out of range for runner {}", pct, runner);
            }
        }

        /// Property: avg fill is absent when the channel is not configured to show it.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_avg_filled_none_when_not_shown(
            capacity in 1usize..10_000,
            runner in 1u128..10_000_000,
        ) {
            let c = ChannelStatsComputer {
                capacity,
                show_avg_filled: false,
                current_filled: Some(ChannelBlock {
                    histogram: None,
                    runner,
                    sum_of_squares: 0,
                }),
                ..Default::default()
            };
            prop_assert!(c.avg_filled_whole_percent().is_none());
        }

        /// Property: init() derives memory_footprint and show_memory from channel meta.
        #[test]
        // ss[verify channel.memory-usage-telemetry]
        // ss[verify verify.process.proptest]
        fn proptest_memory_footprint_from_meta(
            capacity in 1usize..10_000,
            type_byte_count in 1usize..4096,
            show_memory in any::<bool>(),
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.capacity = capacity;
            meta.type_byte_count = type_byte_count;
            meta.show_memory = show_memory;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("src", None),
                ActorName::new("dst", None),
                1000,
            );
            prop_assert_eq!(computer.memory_footprint, capacity * type_byte_count);
            prop_assert_eq!(computer.show_memory, show_memory);
        }

        /// Property: bundle rollup total_consumed equals sum of per-lane deltas.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_bundle_sum_equals_lanes(
            lane_count in 1usize..8,
            deltas in prop::collection::vec(1_i64..500, 1_usize..12),
        ) {
            let meta = mock_meta();
            let mut computers: Vec<ChannelStatsComputer> = (0..lane_count)
                .map(|i| {
                    let mut c = ChannelStatsComputer::default();
                    c.init(
                        &meta,
                        ActorName::new("src", Some(i)),
                        ActorName::new("dst", Some(i)),
                        1000,
                    );
                    c
                })
                .collect();

            let mut takes = vec![0i64; lane_count];
            let mut expected = 0u128;
            for delta in &deltas {
                for i in 0..lane_count {
                    takes[i] = takes[i].saturating_add(*delta);
                    let send = takes[i].saturating_add(10);
                    compute_frame(&mut computers[i], send, takes[i]);
                }
                expected += (*delta as u128) * lane_count as u128;
            }

            let bundle_total: u128 = computers.iter().map(|c| c.total_consumed).sum();
            prop_assert_eq!(bundle_total, expected);
        }

        /// Property: init resets counters; counter reset (take < prev_take) still accumulates.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_reset_clears_state_and_handles_take_rollback(
            pre_reset_deltas in prop::collection::vec(1_i64..500, 1_usize..5),
            post_reset_take in 1i64..500,
        ) {
            let mut computer = fresh_computer();
            prop_assert_eq!(computer.total_consumed, 0);
            prop_assert_eq!(computer.prev_take, 0);

            let mut take = 0i64;
            for delta in pre_reset_deltas {
                take = take.saturating_add(delta);
                compute_frame(&mut computer, take.saturating_add(10), take);
            }
            let before_reset = computer.total_consumed;
            let prev_take = computer.prev_take;
            prop_assume!(post_reset_take < prev_take);

            // Simulate counter rollback: take drops below prev_take.
            compute_frame(
                &mut computer,
                post_reset_take.saturating_add(10),
                post_reset_take,
            );
            prop_assert_eq!(
                computer.total_consumed,
                before_reset + post_reset_take as u128
            );

            // Re-init clears accumulated state.
            computer.init(
                &mock_meta(),
                ActorName::new("a", None),
                ActorName::new("b", None),
                1000,
            );
            prop_assert_eq!(computer.total_consumed, 0);
            prop_assert_eq!(computer.prev_take, 0);
        }

        /// Property: higher alert color wins when multiple channel triggers fire.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_alert_color_priority_red_over_yellow(_case in 0..1u8) {
            let mut computer = ChannelStatsComputer::default();
            let mut meta = (*mock_meta()).clone();
            meta.trigger_rate
                .push((Trigger::AvgAbove(Rate::per_seconds(1)), AlertColor::Yellow));
            meta.trigger_filled
                .push((Trigger::AvgAbove(Filled::p10()), AlertColor::Red));
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                1000,
            );

            let rotations =
                (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
            for _ in 0..rotations {
                computer.accumulate_data_frame(50, 100);
            }

            let mut label = String::new();
            let (color, _) = computer.compute(&mut label, &mut String::new(), None, 100, 50);
            prop_assert_eq!(color, DOT_RED);
        }

        /// Property: distinct lane suffixes produce unique prometheus label strings.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_prometheus_label_suffix_uniqueness(
            suffixes in prop::collection::vec(0usize..100, 2..8),
        ) {
            let meta = mock_meta();
            let labels: Vec<String> = suffixes
                .iter()
                .map(|&suf| {
                    let mut c = ChannelStatsComputer::default();
                    c.init(
                        &meta,
                        ActorName::new("src", Some(suf)),
                        ActorName::new("dst", Some(suf)),
                        1000,
                    );
                    c.prometheus_labels.clone()
                })
                .collect();

            for (i, a) in labels.iter().enumerate() {
                for (j, b) in labels.iter().enumerate() {
                    if suffixes[i] != suffixes[j] {
                        prop_assert_ne!(a, b, "suffixes {:?} vs {:?} collided", suffixes[i], suffixes[j]);
                    }
                }
            }
        }

        /// Property: enough accumulate frames roll the window and set `current_filled`.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_window_rollover_sets_current_filled(
            filled in 1u64..50,
            rate in 1u64..50,
            extra_frames in 0usize..4,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.percentiles_filled.push(crate::actor_builder_units::Percentile::p50());
            meta.refresh_rate_in_bits = 0;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            let rollover_frames = (1 << computer.window_bucket_in_bits) + extra_frames;
            for _ in 0..rollover_frames {
                computer.accumulate_data_frame(filled, rate);
            }
            prop_assert!(computer.current_filled.is_some());
        }

        /// Property: `triggered_rate` AvgAbove iff `avg_rational` is greater.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_triggered_rate_iff_avg_rational(
            rate_value in 10u64..500,
            threshold_units in 5u64..200,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_rate = true;
            meta.refresh_rate_in_bits = 1;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            let frames = (1 << (computer.refresh_rate_in_bits + computer.window_bucket_in_bits)) + 2;
            for _ in 0..frames {
                computer.accumulate_data_frame(10, rate_value);
            }
            let rule = Trigger::AvgAbove(Rate::per_seconds(threshold_units));
            let window_ms = computer.frame_rate_ms
                << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
            let ordering = avg_rational(
                window_ms as u128,
                PLACES_TENS as u128,
                &computer.current_rate,
                Rate::per_seconds(threshold_units).rational_ms(),
            );
            prop_assert_eq!(computer.triggered_rate(&rule), ordering.is_gt());
        }

        /// Property: `triggered_filled` Exact AvgAbove iff `avg_filled_exact` is greater.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_triggered_filled_exact_iff(
            filled in 5u64..80,
            threshold in 1u64..50,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_filled = true;
            meta.percentiles_filled.push(crate::actor_builder_units::Percentile::p50());
            meta.refresh_rate_in_bits = 0;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            for _ in 0..3 {
                computer.accumulate_data_frame(filled, 10);
            }
            let rule = Trigger::AvgAbove(Filled::exact(threshold));
            let ordering = computer.avg_filled_exact(&threshold);
            prop_assert_eq!(computer.triggered_filled(&rule), ordering.is_gt());
        }

        /// Property: `avg_latency` returns Equal when no latency window sample exists.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_avg_latency_none_when_no_sample(_case in 0..1u8) {
            let computer = ChannelStatsComputer::default();
            prop_assert_eq!(
                computer.avg_latency(&Duration::from_millis(100)),
                Ordering::Equal
            );
        }

        /// Property: `triggered_filled` Percentage AvgAbove iff `avg_filled_percentage` is greater.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_triggered_filled_percentage_iff(
            filled in 5u64..80,
            threshold_num in 1u64..50,
            threshold_den in 50u64..100,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_filled = true;
            meta.percentiles_filled.push(crate::actor_builder_units::Percentile::p50());
            meta.refresh_rate_in_bits = 0;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            for _ in 0..3 {
                computer.accumulate_data_frame(filled, 10);
            }
            let rule = Trigger::AvgAbove(Filled::Percentage(threshold_num, threshold_den));
            let ordering = computer.avg_filled_percentage(&threshold_num, &threshold_den);
            prop_assert_eq!(computer.triggered_filled(&rule), ordering.is_gt());
        }

        /// Property: `triggered_latency` AvgAbove iff `avg_latency` is greater.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_triggered_latency_iff_predicate(
            filled in 5u64..80,
            rate in 1u64..100,
            threshold_ms in 1u64..500,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_latency = true;
            meta.refresh_rate_in_bits = 0;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            for _ in 0..3 {
                computer.accumulate_data_frame(filled, rate);
            }
            prop_assume!(computer.current_latency.is_some());
            let threshold = Duration::from_millis(threshold_ms);
            let rule = Trigger::AvgAbove(threshold);
            let ordering = computer.avg_latency(&threshold);
            prop_assert_eq!(computer.triggered_latency(&rule), ordering.is_gt());
        }

        /// Property: `triggered_rate` AvgBelow iff `avg_rational` is less.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_triggered_rate_avg_below_iff(
            rate_value in 10u64..500,
            threshold_units in 5u64..200,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_rate = true;
            meta.refresh_rate_in_bits = 1;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            let frames = (1 << (computer.refresh_rate_in_bits + computer.window_bucket_in_bits)) + 2;
            for _ in 0..frames {
                computer.accumulate_data_frame(10, rate_value);
            }
            let rule = Trigger::AvgBelow(Rate::per_seconds(threshold_units));
            let window_ms = computer.frame_rate_ms
                << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
            let ordering = avg_rational(
                window_ms as u128,
                PLACES_TENS as u128,
                &computer.current_rate,
                Rate::per_seconds(threshold_units).rational_ms(),
            );
            prop_assert_eq!(computer.triggered_rate(&rule), ordering.is_lt());
        }

        /// Property: SuppressAvgOnly omits the avg filled line but keeps other filled labels.
        #[test]
        // ss[verify telemetry.channel-labels]
        // ss[verify verify.process.proptest]
        fn proptest_suppress_avg_only_omits_avg_line(
            filled in 10u64..90,
        ) {
            let mut meta = (*mock_meta()).clone();
            meta.avg_filled = true;
            meta.percentiles_filled.push(crate::actor_builder_units::Percentile::p50());
            meta.refresh_rate_in_bits = 0;
            meta.window_bucket_in_bits = 1;
            let mut computer = ChannelStatsComputer::default();
            computer.init(
                &Arc::new(meta),
                ActorName::new("a", None),
                ActorName::new("b", None),
                40,
            );
            for _ in 0..3 {
                computer.accumulate_data_frame(filled, 10);
            }
            let mut full = String::new();
            let mut metric = String::new();
            computer.append_visual_metric_lines(
                &mut full,
                &mut metric,
                FilledVisualMode::Full,
            );
            let mut suppressed = String::new();
            let mut metric2 = String::new();
            computer.append_visual_metric_lines(
                &mut suppressed,
                &mut metric2,
                FilledVisualMode::SuppressAvgOnly,
            );
            prop_assert!(full.contains("Avg filled:"));
            prop_assert!(!suppressed.contains("Avg filled:"));
        }
    }
}
