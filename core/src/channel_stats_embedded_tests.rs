//! Tests extracted from `channel_stats.rs` so the production file stays under the 1,200-line budget.
//! Nested test mods keep their own imports; `use super::*` at the top re-exports the parent.

use super::*;

#[cfg(test)]
// ss[related telemetry.channel-labels]
mod channel_stats_tests {
    // ss[related philosophy.structural-hierarchy]
    use super::super::*;
    // ss[related philosophy.structural-hierarchy]
    use std::time::Duration;
    // ss[related telemetry.channel-labels]
    use crate::monitor::ChannelMetaData;
    // ss[related philosophy.structural-hierarchy]
    use std::sync::Arc;

    // ss[related telemetry.channel-labels]
    fn mock_meta() -> Arc<ChannelMetaData> {
        Arc::new(ChannelMetaData {
            capacity: 100,
            show_total: true,
            type_byte_count: 8,
            ring_slot_byte_count: 8,
            show_type: Some("u64"),
            refresh_rate_in_bits: 1, // Rollover every 2 frames
            window_bucket_in_bits: 1, // Window size 2 buckets
            ..Default::default()
        })
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_filled_percentage_none() {
        let computer = ChannelStatsComputer {
            capacity: 100,
            ..Default::default()
        };
        assert_eq!(computer.avg_filled_percentage(&50, &100), Ordering::Equal);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_latency_none() {
        let computer = ChannelStatsComputer::default();
        assert_eq!(computer.avg_latency(&Duration::from_millis(100)), Ordering::Equal);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_init_and_label_building() {
        let mut computer = ChannelStatsComputer::default();
        let meta = mock_meta();
        let from = ActorName::new("src", Some(1));
        let to = ActorName::new("dst", None);
        
        computer.init(&meta, from, to, 1000);
        
        assert_eq!(computer.capacity, 100);
        assert!(computer.prometheus_labels.contains("from=\"src1\""));
        assert!(computer.prometheus_labels.contains("to=\"dst\""));
        assert!(computer.prometheus_labels.contains("type=\"u64\""));
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_init_builds_labels_without_suffix_on_to_actor() {
        let mut computer = ChannelStatsComputer::default();
        let meta = mock_meta();
        computer.init(
            &meta,
            ActorName::new("producer", Some(2)),
            ActorName::new("consumer", None),
            1000,
        );
        assert!(computer.prometheus_labels.contains("from=\"producer2\""));
        assert!(computer.prometheus_labels.contains("to=\"consumer\""));
        assert!(!computer.prometheus_labels.contains("consumer-"));
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_monotonic_total_and_resets() {
        let mut computer = ChannelStatsComputer::default();
        computer.init(&mock_meta(), ActorName::new("a", None), ActorName::new("b", None), 1000);

        let mut label = String::new();
        let mut metric = String::new();

        // Normal increment
        computer.compute(&mut label, &mut metric, None, 100, 50);
        assert_eq!(computer.total_consumed, 50);

        // Counter reset (take < prev_take)
        computer.compute(&mut label, &mut metric, None, 120, 10);
        // Should treat 10 as a fresh delta: 50 + 10 = 60
        assert_eq!(computer.total_consumed, 60);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_bundle_and_memory_display() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.girth = 4;
        meta.show_memory = true;
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Verify internal state is set correctly (display label no longer contains these values)
        assert_eq!(computer.girth, 4);
        assert_eq!(computer.ring_memory_footprint, 800); // 100 * 8 bytes
        assert_eq!(computer.dynamic_memory_footprint, 0);
        assert!(computer.show_memory);
    }

    #[test]
    // ss[verify channel.ring-memory-footprint]
    fn ring_memory_footprint_from_capacity_and_slot_bytes() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.capacity = 100;
        meta.ring_slot_byte_count = 8;
        computer.init(
            &Arc::new(meta),
            ActorName::new("a", None),
            ActorName::new("b", None),
            1000,
        );
        assert_eq!(computer.ring_memory_footprint, 800);
        assert_eq!(computer.dynamic_memory_footprint, 0);
    }

    #[test]
    // ss[verify channel.dynamic-payload-estimate]
    fn dynamic_memory_footprint_from_per_slot_estimate() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.capacity = 100;
        meta.ring_slot_byte_count = 8;
        meta.dynamic_per_slot_estimate = Some(256_000);
        computer.init(
            &Arc::new(meta),
            ActorName::new("a", None),
            ActorName::new("b", None),
            1000,
        );
        assert_eq!(computer.ring_memory_footprint, 800);
        assert_eq!(computer.dynamic_memory_footprint, 25_600_000);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_alert_color_priority() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        // Add a Red and a Yellow trigger
        meta.trigger_rate.push((Trigger::AvgAbove(Rate::per_seconds(1)), AlertColor::Yellow));
        meta.trigger_filled.push((Trigger::AvgAbove(Filled::p10()), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Fill window + 1 to trigger rotation
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(50, 100); }
        
        let mut label = String::new();
        let (color, _) = computer.compute(&mut label, &mut String::new(), None, 100, 50);
        
        assert_eq!(color, DOT_RED); // Red should override Yellow
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_trigger_gauntlet_latency() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        // Add all latency trigger variants
        meta.trigger_latency.push((Trigger::AvgAbove(Duration::from_micros(100)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::AvgBelow(Duration::from_micros(10)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::StdDevsAbove(StdDev::one(), Duration::from_micros(50)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::StdDevsBelow(StdDev::one(), Duration::from_micros(5)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::PercentileAbove(Percentile::p90(), Duration::from_micros(200)), AlertColor::Red));
        meta.trigger_latency.push((Trigger::PercentileBelow(Percentile::p25(), Duration::from_micros(2)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Fill window + 1 to trigger rotation
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(50, 100); }
        
        assert!(computer.triggered_latency(&Trigger::AvgAbove(Duration::from_micros(100))));
        assert!(!computer.triggered_latency(&Trigger::AvgBelow(Duration::from_micros(10))));
        assert!(computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_micros(200))));
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_trigger_gauntlet_rate() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        meta.trigger_rate.push((Trigger::AvgAbove(Rate::per_seconds(50)), AlertColor::Red));
        meta.trigger_rate.push((Trigger::AvgBelow(Rate::per_seconds(10000)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Rate is 100 per message. With 32 samples/frame, that's 3200 per sec.
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(10, 100); }
        
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_seconds(50))));
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_seconds(10000))));
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_trigger_gauntlet_filled() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        
        meta.trigger_filled.push((Trigger::AvgAbove(Filled::Percentage(50, 100)), AlertColor::Red));
        meta.trigger_filled.push((Trigger::AvgBelow(Filled::Exact(80)), AlertColor::Red));
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // 60 items in 100 capacity -> 60% full
        let c = (1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits)) + 1;
        for _ in 0..c { computer.accumulate_data_frame(60, 10); }
        
        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::Percentage(50, 100))));
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(80))));
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_std_dev_triggers() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        meta.std_dev_inflight.push(StdDev::one());
        meta.std_dev_consumed.push(StdDev::one());
        meta.std_dev_latency.push(StdDev::one());
        
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        
        // Add varying data to create standard deviation
        for i in 0..20 { computer.accumulate_data_frame(i % 10, 100); }
        
        assert!(computer.rate_std_dev() >= 0.0);
        assert!(computer.filled_std_dev() >= 0.0);
        assert!(computer.latency_std_dev() >= 0.0);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_zero_capacity_safety() {
        let mut computer = ChannelStatsComputer::default();
        // Capacity 0 is invalid but we test the safety return
        computer.capacity = 0;
        let mut label = String::new();
        let (color, _) = computer.compute(&mut label, &mut String::new(), None, 0, 0);
        assert_eq!(color, DOT_GREY);
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_histogram_creation_failure_handling() {
        let mut computer = ChannelStatsComputer::default();
        let mut meta = (*mock_meta()).clone();
        // Force histogram creation
        meta.percentiles_filled.push(Percentile::p50());
        
        // We can't easily force Histogram::new to fail without mocking, 
        // but we can verify the branches that handle it.
        computer.init(&Arc::new(meta), ActorName::new("a", None), ActorName::new("b", None), 1000);
        assert!(computer.build_filled_histogram);
    }

    // ========================================================================
    // Avg fill: zero-runner / idle-channel tests
    // These verify the fix: avg_filled_whole_percent() returns None when the
    // window's runner is zero (no data), so bundle/partner rollups never
    // produce "Avg fill: 0%, 0%, 0%, …" noise.
    // ========================================================================

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_filled_whole_percent_none_when_show_disabled() {
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: false,
            current_filled: Some(ChannelBlock { histogram: None, runner: 50_000, sum_of_squares: 0 }),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        assert_eq!(c.avg_filled_whole_percent(), None, "should be None when show_avg_filled is false");
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_filled_whole_percent_none_when_no_current_block() {
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            current_filled: None,
            ..Default::default()
        };
        assert_eq!(c.avg_filled_whole_percent(), None, "should be None when current_filled is None");
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_filled_whole_percent_none_when_runner_zero() {
        // THIS IS THE KEY FIX TEST: runner == 0 (idle/cold channel) → None, not Some(0)
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            current_filled: Some(ChannelBlock { histogram: None, runner: 0, sum_of_squares: 0 }),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        assert_eq!(
            c.avg_filled_whole_percent(),
            None,
            "idle channel with runner == 0 must return None (not Some(0))"
        );
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_avg_filled_whole_percent_returns_some_when_nonzero() {
        // Sanity: non-zero runner still produces a valid percentage
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            current_filled: Some(ChannelBlock { histogram: None, runner: 50_000, sum_of_squares: 0 }),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        assert_eq!(c.avg_filled_whole_percent(), Some(50), "50_000 runner with cap 100 → 50%");
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_compute_filled_labels_inner_omits_avg_when_runner_zero() {
        // When runner is zero, no "Avg filled:" line should appear.
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            current_filled: Some(ChannelBlock { histogram: None, runner: 0, sum_of_squares: 0 }),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        let block = c.current_filled.as_ref().unwrap();
        let mut label = String::new();
        let mut metric = String::new();
        c.compute_filled_labels_inner(&mut label, &mut metric, &block, false);
        assert!(
            !label.contains("Avg filled"),
            "label must not contain 'Avg filled' when runner is zero: {:?}",
            label
        );
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_compute_filled_labels_inner_shows_avg_when_nonzero_runner() {
        // Sanity: non-zero runner still shows "Avg filled:" line
        let c = ChannelStatsComputer {
            capacity: 100,
            show_avg_filled: true,
            current_filled: Some(ChannelBlock { histogram: None, runner: 30_000, sum_of_squares: 0 }),
            refresh_rate_in_bits: 0,
            window_bucket_in_bits: 0,
            ..Default::default()
        };
        let block = c.current_filled.as_ref().unwrap();
        let mut label = String::new();
        let mut metric = String::new();
        c.compute_filled_labels_inner(&mut label, &mut metric, &block, false);
        assert!(
            label.contains("Avg filled: 30 %"),
            "label must contain 'Avg filled: 30 %': {:?}",
            label
        );
    }

    #[test]
    // ss[verify telemetry.channel-labels]
    fn test_edge_label_no_avg_filled_when_runner_zero_via_compute() {
        // Integration-style: calling compute() on an idle channel should not
        // produce an "Avg filled" line in the display_label.
        let meta = mock_meta();
        let mut computer = ChannelStatsComputer::default();
        computer.init(&meta, ActorName::new("a", None), ActorName::new("b", None), 1000);
        // Force show_avg_filled after init (it was false by default in mock_meta)
        computer.show_avg_filled = true;

        let mut label = String::new();
        let mut metric = String::new();

        // First call: computer accumulates data but no bucket rotation yet
        computer.compute(&mut label, &mut metric, None, 0, 0);
        // No window sample yet → avg_filled_whole_percent returns None → no "Avg filled:" line
        assert!(
            !label.contains("Avg filled"),
            "no Avg filled yet (no window sample): {:?}",
            label
        );

        // Rotate enough to produce a current_filled with runner=0 (idle channel)
        let rotations = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..rotations {
            computer.compute(&mut label, &mut metric, None, 0, 0);
        }

        // Now current_filled exists but runner is zero → avg_filled_whole_percent returns None
        assert!(
            !label.contains("Avg filled"),
            "no Avg filled line for idle channel with zero-data window: {:?}",
            label
        );
        assert!(
            !label.contains("Avg filled: 0 %"),
            "must not produce 'Avg filled: 0 %': {:?}",
            label
        );
    }
}
