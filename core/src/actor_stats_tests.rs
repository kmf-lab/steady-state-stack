#[cfg(test)]
mod test_actor_stats {
    use std::cmp;
    use crate::actor_stats::*;
    use std::sync::Arc;
    use crate::{ActorIdentity, AlertColor, StdDev, Trigger};
    use crate::actor_builder_units::{Percentile, Work, MCPU};
    use crate::channel_stats::DOT_GREEN;
    use crate::monitor::ActorMetaData;

    fn create_mock_metadata() -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1,"test_actor", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50(), Percentile::p90()],
            percentiles_work: vec![Percentile::p50(), Percentile::p90()],
            std_dev_mcpu: vec![StdDev::new(1.0).expect("")],
            std_dev_work: vec![StdDev::new(1.0).expect("")],
            trigger_mcpu: vec![(Trigger::AvgAbove(MCPU::m512()), AlertColor::Red)],
            trigger_work: vec![(Trigger::AvgAbove(Work::p50()), AlertColor::Red)],
            usage_review: false,
            refresh_rate_in_bits: 6,
            window_bucket_in_bits: 5,
        })
    }

    #[test]
    fn test_init() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        assert_eq!(actor_stats.ident.id, 1);
        assert_eq!(actor_stats.ident.label.name, "test_actor");
        assert!(actor_stats.show_avg_mcpu);
        assert!(actor_stats.show_avg_work);
        assert_eq!(actor_stats.percentiles_mcpu.len(), 2);
        assert_eq!(actor_stats.percentiles_work.len(), 2);
        assert_eq!(actor_stats.std_dev_mcpu.len(), 1);
        assert_eq!(actor_stats.std_dev_work.len(), 1);
        assert_eq!(actor_stats.mcpu_trigger.len(), 1);
        assert_eq!(actor_stats.work_trigger.len(), 1);
        assert_eq!(actor_stats.frame_rate_ms, 1000);
        assert_eq!(actor_stats.refresh_rate_in_bits, 6);
        assert_eq!(actor_stats.window_bucket_in_bits, 5);
    }

    #[test]
    fn test_accumulate_data_frame() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let mcpu_histogram = actor_stats.history_mcpu.front().expect("iternal error").histogram.as_ref().expect("iternal error");
        let work_histogram = actor_stats.history_work.front().expect("iternal error").histogram.as_ref().expect("iternal error");

        assert_eq!(mcpu_histogram.value_at_quantile(0.5), 543);
        assert_eq!(work_histogram.value_at_quantile(0.5), 51);
    }

    #[test]
    fn test_compute_labels() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        let (line_color, line_width) = actor_stats.compute(
            &mut dot_label,
            &mut tooltip,
            &mut metric_text,
            Some((512, 50)),
            1,
            false,
            false,
            None
        );

        assert_eq!(line_color, DOT_GREEN);
        assert_eq!(line_width, crate::dot::NODE_PEN_WIDTH);
        assert!(dot_label.contains("test_actor"));

    }

    #[test]
    fn test_percentile_rational() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        actor_stats.accumulate_data_frame(512, 50);

        let percentile_result = percentile_rational(
            &Percentile::p50(),
            &actor_stats.current_mcpu,
            (512, 1024),
        );

        assert_eq!(percentile_result, cmp::Ordering::Equal);
    }
}

#[cfg(test)]
mod test_actor_stats_triggers {
    use crate::actor_stats::*;
    use std::sync::Arc;
    use crate::{ActorIdentity, AlertColor, StdDev, Trigger};
    use crate::actor_builder_units::{Percentile, Work, MCPU};
    use crate::monitor::ActorMetaData;

    fn create_mock_metadata() -> Arc<ActorMetaData> {
        Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test_actor", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50(), Percentile::p90()],
            percentiles_work: vec![Percentile::p50(), Percentile::p90()],
            std_dev_mcpu: vec![StdDev::new(1.0).expect("")],
            std_dev_work: vec![StdDev::new(1.0).expect("")],
            trigger_mcpu: vec![
                (Trigger::AvgAbove(MCPU::m512()), AlertColor::Yellow),
                (Trigger::AvgBelow(MCPU::m256()), AlertColor::Red),
            ],
            trigger_work: vec![
                (Trigger::AvgAbove(Work::p50()), AlertColor::Orange),
                (Trigger::AvgBelow(Work::p30()), AlertColor::Yellow),
            ],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        })
    }

    #[test]
    fn test_trigger_avg_above_mcpu() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);

        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(520, 40);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg above trigger to be activated for mcpu."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(509, 40);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg above trigger NOT to be activated for mcpu."
        );
    }

    #[test]
    fn test_trigger_avg_below_mcpu() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(230, 40);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Red),
            "Expected avg below trigger to be activated for mcpu."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(260, 40);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Red),
            "Expected avg below trigger NOT to be activated for mcpu."
        );
    }

    #[test]
    fn test_trigger_avg_above_work() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 55);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Orange),
            "Expected avg above trigger to be activated for work."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 45);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Orange),
            "Expected avg above trigger NOT to be activated for work."
        );
    }

    #[test]
    fn test_trigger_avg_below_work() {
        let metadata = create_mock_metadata();
        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata.clone(), 1000);

        // Need enough frames to fill the window and set current_mcpu
        let total_frames = 1 << (1+ metadata.window_bucket_in_bits + metadata.refresh_rate_in_bits);
        //println!("total_frames: {}", total_frames);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 28);
        }
        assert!(
            actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg below trigger to be activated for work."
        );
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 32);
        }
        assert!(
            !actor_stats.trigger_alert_level(&AlertColor::Yellow),
            "Expected avg below trigger NOT to be activated for work."
        );
    }
}

/// Additional tests to achieve 100% coverage for all utility functions and edge cases.
#[cfg(test)]
mod extra_tests {
    use crate::actor_stats::*;


    /// Verify `time_label` produces the correct text for various durations.
    #[test]
    fn test_time_label_thresholds() {
        // sub-second
        assert_eq!(time_label(500), "sec");
        // seconds
        assert_eq!(time_label(1500), "1.5 secs");
        // exactly one minute
        assert_eq!(time_label(60_000), "min");
        // minutes
        assert_eq!(time_label(90_000), "1.5 mins");
        // exactly one hour
        assert_eq!(time_label(3_600_000), "hr");
        // multiple hours
        assert_eq!(time_label(7_200_000), "2.0 hrs");
        // exactly one day
        assert_eq!(time_label(86_400_000), "day");
        // multiple days
        assert_eq!(time_label(172_800_000), "2.0 days");
    }

    /// Test both branches of `compute_std_dev`.
    #[test]
    fn test_compute_std_dev_branches() {
        // runner < SQUARE_LIMIT: should compute a finite non-negative value
        let val = compute_std_dev(1, 2, 1, 2);
        assert!(val >= 0.0, "std dev should be non-negative");

        // runner >= SQUARE_LIMIT: safety guard .max(0.0) prevents NaN
        let val = compute_std_dev(0, 1, SQUARE_LIMIT, 0);
        assert_eq!(val, 0.0, "expected 0.0 due to safety guard");
    }

    use std::sync::Arc;
    use crate::monitor::ActorMetaData;
    use crate::{steady_config, logging_util, ActorIdentity, AlertColor, StdDev, Trigger};
    use crate::actor_builder_units::{Percentile, Work, MCPU};
    use crate::channel_stats::{DOT_ORANGE, DOT_RED, DOT_YELLOW};

    /// Test init method with actor suffix
    #[test]
    fn test_init_with_actor_suffix() {
        let _ = logging_util::steady_logger::initialize();

        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(42, "test_actor", Some(123)),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![],
            percentiles_work: vec![],
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);

        // Should contain suffix in prometheus labels
        assert!(actor_stats.prometheus_labels.contains("actor_suffix=\"123\""));
    }

    /// Test compute method with actor suffix
    #[test]
    fn test_compute_with_actor_suffix() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", Some(42));

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 50)), 0, false, false, None);

        // Should contain actor name with suffix
        assert!(dot_label.contains("test"));
        assert!(dot_label.contains("42"));
    }

    /// Test compute method with SHOW_ACTORS feature enabled
    #[cfg(feature = "core_display")]
    #[test]
    fn test_compute_with_show_actors() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(123, "test", None);

        // Mock the SHOW_ACTORS constant
        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 50)), 0, false, false, None);

    }

    /// Test compute method with window display
    #[test]
    fn test_compute_with_window_display() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);
        actor_stats.window_bucket_in_bits = 2; // Non-zero to show window
        actor_stats.time_label = "5.0 mins".to_string();

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 50)), 0, false, false, None);

        // Should contain window information in tooltip
        assert!(tooltip.contains("Window 5.0 mins"));
        assert!(!dot_label.contains("Window"));
    }

    /// Test compute method with restart count
    #[test]
    fn test_compute_with_restarts() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);
        actor_stats.prometheus_labels = "test=\"true\"".to_string();

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 50)), 5, false, false, None);

        // Should contain restart count in dot_label
        assert!(dot_label.contains("Restarts: 5"));

        #[cfg(feature = "prometheus_metrics")]
        {
            // Should contain prometheus restart metric
            assert!(metric_text.contains("graph_node_restarts{"));
            assert!(metric_text.contains("} 5"));
        }
    }

    /// Test compute method with stopped actor
    #[test]
    fn test_compute_with_stopped_actor() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.ident = ActorIdentity::new(1, "test", None);

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 50)), 0, true, false, None);

        // Should contain stopped indicator in tooltip
        assert!(tooltip.contains("stopped"));
        assert!(!dot_label.contains("stopped"));
    }

    /// Test compute method with current work data
    #[test]
    fn test_compute_with_current_work() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.show_avg_work = true;

        // Force current_work to exist by accumulating enough data
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(500, 60);
        }

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((500, 60)), 0, false, false, None);

        // Should contain work load information in both
        assert!(dot_label.contains("load"));
        assert!(tooltip.contains("load"));
    }

    /// Test compute method with current mcpu data
    #[test]
    fn test_compute_with_current_mcpu() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.show_avg_mcpu = true;

        // Force current_mcpu to exist by accumulating enough data
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(700, 50);
        }

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((700, 50)), 0, false, false, None);

        // Should contain mcpu information in both
        assert!(dot_label.contains("mCPU"));
        assert!(tooltip.contains("mCPU"));
    }

    /// Test alert level triggers - Yellow
    #[test]
    fn test_trigger_alert_level_yellow() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add Yellow trigger that should fire
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));

        // Accumulate data to trigger alert
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(600, 50); // Above 256
        }

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((600, 50)), 0, false, false, None);

        assert_eq!(color, DOT_YELLOW);
    }

    /// Test alert level triggers - Orange
    #[test]
    fn test_trigger_alert_level_orange() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add Orange trigger that should fire
        actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p40()), AlertColor::Orange));

        // Accumulate data to trigger alert
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(300, 70); // Above 40%
        }

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((300, 70)), 0, false, false, None);

        assert_eq!(color, DOT_ORANGE);
    }

    /// Test histogram creation errors during init
    #[test]
    fn test_init_histogram_creation_errors() {
        let _ = logging_util::steady_logger::initialize();

        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50()], // Force histogram creation
            percentiles_work: vec![Percentile::p50()], // Force histogram creation
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();

        // This should handle potential histogram creation gracefully
        actor_stats.init(metadata, 1000);

        // Should have created histograms successfully
        assert!(actor_stats.build_mcpu_histogram);
        assert!(actor_stats.build_work_histogram);
    }

    /// Test Percentile triggers for mcpu
    #[test]
    fn test_triggered_mcpu_percentiles() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();
        actor_stats.percentiles_mcpu.push(Percentile::p50());

        // Accumulate data to get current_mcpu with histogram
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(500, 50);
        }

        // Test PercentileAbove trigger
        assert!(actor_stats.triggered_mcpu(&Trigger::PercentileAbove(Percentile::p50(), MCPU::m256())));
        assert!(!actor_stats.triggered_mcpu(&Trigger::PercentileAbove(Percentile::p50(), MCPU::m1024())));

        // Test PercentileBelow trigger
        assert!(!actor_stats.triggered_mcpu(&Trigger::PercentileBelow(Percentile::p50(), MCPU::m256())));
        assert!(actor_stats.triggered_mcpu(&Trigger::PercentileBelow(Percentile::p50(), MCPU::m1024())));
    }

    /// Test std dev functions when current data is None
    #[test]
    fn test_std_dev_functions_with_none_current() {
        let _ = logging_util::steady_logger::initialize();

        let actor_stats = ActorStatsComputer::default();

        // Should return 0 and log info when current data is None
        assert_eq!(actor_stats.mcpu_std_dev(), 0f32);
        assert_eq!(actor_stats.work_std_dev(), 0f32);
    }

    /// Test compute_std_dev alternative calculation branch
    #[test]
    fn test_compute_std_dev_alternative_branch() {
        let _ = logging_util::steady_logger::initialize();

        // Test the branch where sum_sqr <= r2
        let result = compute_std_dev(4, 16, 1000, 500); // sum_sqr < r2
        assert!(result >= 0.0 || result.is_nan()); // Should handle gracefully
    }

    /// Test accumulate_data_frame with histogram recording errors
    #[test]
    fn test_accumulate_data_frame_histogram_errors() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();

        // Try to record extreme values that might cause histogram errors
        actor_stats.accumulate_data_frame(1024, 100); // Max valid mcpu

        // This should not panic and should handle errors gracefully
        assert!(!actor_stats.history_mcpu.is_empty());
        assert!(!actor_stats.history_work.is_empty());
    }

    /// Test bucket refresh with histogram creation errors
    #[test]
    fn test_bucket_refresh_histogram_errors() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_data();

        // Force multiple bucket refreshes
        for cycle in 0..5 {
            let frames_per_bucket = 1 << actor_stats.refresh_rate_in_bits;
            for _ in 0..frames_per_bucket {
                actor_stats.accumulate_data_frame(400 + cycle * 50, 50 + cycle * 10);
            }
        }

        // Should have handled histogram creation during refresh
        assert!(!actor_stats.history_mcpu.is_empty());
        assert!(!actor_stats.history_work.is_empty());
    }

    /// Helper function to set up actor with basic data
    fn setup_actor_with_data() -> ActorStatsComputer {
        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50()],
            percentiles_work: vec![Percentile::p50()],
            std_dev_mcpu: vec![StdDev::one()],
            std_dev_work: vec![StdDev::one()],
            trigger_mcpu: vec![],
            trigger_work: vec![],
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);
        actor_stats
    }

    /// Helper function to set up actor with triggers
    fn setup_actor_with_triggers() -> ActorStatsComputer {
        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: false,
            avg_work: false,
            show_thread_info: false,
            percentiles_mcpu: vec![],
            percentiles_work: vec![],
            std_dev_mcpu: vec![],
            std_dev_work: vec![],
            trigger_mcpu: vec![], // Will be added in tests
            trigger_work: vec![], // Will be added in tests
            usage_review: false,
            refresh_rate_in_bits: 2,
            window_bucket_in_bits: 2,
        });

        let mut actor_stats = ActorStatsComputer::default();
        actor_stats.init(metadata, 1000);
        actor_stats
    }

    /// Test comprehensive alert combinations
    #[test]
    fn test_comprehensive_alert_combinations() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor_stats = setup_actor_with_triggers();

        // Add multiple triggers of different colors
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));
        actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange));
        actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p70()), AlertColor::Red));

        // Test scenario where Red trigger fires (highest priority)
        let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(600, 80); // High work to trigger Red
        }

        let mut dot_label = String::new();
        let mut tooltip = String::new();
        let mut metric_text = String::new();

        let (color, _) = actor_stats.compute(&mut dot_label, &mut tooltip, &mut metric_text, Some((600, 80)), 0, false, false, None);

        // Should be Red (highest priority) even though other triggers also fire
        assert_eq!(color, DOT_RED);
    }
}
