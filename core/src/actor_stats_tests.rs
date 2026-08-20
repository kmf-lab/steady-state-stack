#[cfg(test)]
// ss[related telemetry.prometheus-metrics]
mod test_actor_stats {
    // ss[related philosophy.structural-hierarchy]
    use std::sync::Arc;

    // ss[related telemetry.prometheus-metrics]
    use proptest::prelude::*;

    // ss[related philosophy.structural-hierarchy]
    use crate::actor_builder_units::{MCPU, Percentile, Work};
    // ss[related telemetry.prometheus-metrics]
    use crate::actor_stats::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::channel_stats::DOT_GREEN;
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor::ActorMetaData;
    // ss[related telemetry.prometheus-metrics]
    use crate::{ActorIdentity, AlertColor, StdDev, Trigger};

    // ss[related philosophy.structural-hierarchy]
    fn fill_window(actor_stats: &mut ActorStatsComputer, mcpu: u16, work: u16) {
        let total_frames = 1 << (1 + actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits);
        for _ in 0..total_frames {
            actor_stats.accumulate_data_frame(mcpu, work);
        }
    }

    // --- smoke tests ---

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_time_label_thresholds() {
        // Discrete duration buckets; not property-friendly.
        assert_eq!(time_label(500), "sec");
        assert_eq!(time_label(1500), "1.5 secs");
        assert_eq!(time_label(60_000), "min");
        assert_eq!(time_label(90_000), "1.5 mins");
        assert_eq!(time_label(3_600_000), "hr");
        assert_eq!(time_label(7_200_000), "2.0 hrs");
        assert_eq!(time_label(86_400_000), "day");
        assert_eq!(time_label(172_800_000), "2.0 days");
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_compute_std_dev_branches() {
        let val = compute_std_dev(1, 2, 1, 2);
        assert!(val >= 0.0);
        let val = compute_std_dev(0, 1, SQUARE_LIMIT, 0);
        assert_eq!(val, 0.0);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_std_dev_functions_with_none_current() {
        let actor_stats = ActorStatsComputer::default();
        assert_eq!(actor_stats.mcpu_std_dev(), 0f32);
        assert_eq!(actor_stats.work_std_dev(), 0f32);
    }

    ss_proptest! {

        /// Property: AvgAbove/AvgBelow mCPU triggers fire iff avg_rational predicate holds.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_mcpu_iff_predicate(
            mcpu_value in 100u16..900u16,
            threshold in 256u16..768u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: true,
                avg_work: false,
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
            fill_window(&mut actor_stats, mcpu_value, 0);

            let run_divisor = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits);
            let above = Trigger::AvgAbove(MCPU::new(threshold).expect("valid"));
            let below = Trigger::AvgBelow(MCPU::new(threshold).expect("valid"));
            let threshold_pair = (threshold as u64, 1);

            let above_ordering = avg_rational(run_divisor, 1, &actor_stats.current_mcpu, threshold_pair);
            let below_ordering = avg_rational(run_divisor, 1, &actor_stats.current_mcpu, threshold_pair);

            prop_assert_eq!(
                actor_stats.triggered_mcpu(&above),
                above_ordering.is_gt()
            );
            prop_assert_eq!(
                actor_stats.triggered_mcpu(&below),
                below_ordering.is_lt()
            );
        }

        /// Property: AvgAbove/AvgBelow work triggers fire iff avg_rational predicate holds.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_work_iff_predicate(
            work_value in 10u16..90u16,
            threshold_pct in 20u16..80u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: false,
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
            fill_window(&mut actor_stats, 0, work_value);

            let run_divisor = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits);
            let work = Work::new(threshold_pct as f32).expect("valid percent");
            let rational = work.rational();

            let above_ordering = avg_rational(run_divisor, 100, &actor_stats.current_work, rational);
            actor_stats.work_trigger.push((Trigger::AvgAbove(work), AlertColor::Red));
            prop_assert_eq!(
                actor_stats.trigger_alert_level(&AlertColor::Red),
                above_ordering.is_gt()
            );

            let below_ordering = avg_rational(run_divisor, 100, &actor_stats.current_work, rational);
            actor_stats.work_trigger = vec![(Trigger::AvgBelow(work), AlertColor::Yellow)];
            prop_assert_eq!(
                actor_stats.trigger_alert_level(&AlertColor::Yellow),
                below_ordering.is_lt()
            );
        }

        /// Property: higher constant mCPU input yields higher window average.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_mcpu_work_ordering(
            low_mcpu in 100u16..400u16,
            high_mcpu in 500u16..900u16,
            low_work in 10u16..40u16,
            high_work in 50u16..90u16,
        ) {
            prop_assume!(high_mcpu > low_mcpu);
            prop_assume!(high_work > low_work);

            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
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

            let mut low_stats = ActorStatsComputer::default();
            low_stats.init(metadata.clone(), 1000);
            fill_window(&mut low_stats, low_mcpu, low_work);

            let mut high_stats = ActorStatsComputer::default();
            high_stats.init(metadata, 1000);
            fill_window(&mut high_stats, high_mcpu, high_work);

            let low_mcpu_runner = low_stats.current_mcpu.as_ref().map(|b| b.runner).unwrap_or(0);
            let high_mcpu_runner = high_stats.current_mcpu.as_ref().map(|b| b.runner).unwrap_or(0);
            let low_work_runner = low_stats.current_work.as_ref().map(|b| b.runner).unwrap_or(0);
            let high_work_runner = high_stats.current_work.as_ref().map(|b| b.runner).unwrap_or(0);

            prop_assert!(high_mcpu_runner > low_mcpu_runner);
            prop_assert!(high_work_runner > low_work_runner);
        }

        /// Property: percentile_rational matches triggered_mcpu PercentileAbove/Below.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_percentile_rational_consistent(
            mcpu_value in 200u16..800u16,
            threshold in 256u16..768u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: true,
                avg_work: false,
                show_thread_info: false,
                percentiles_mcpu: vec![Percentile::p50()],
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
            fill_window(&mut actor_stats, mcpu_value, 0);

            let mcpu = MCPU::new(threshold).expect("valid");
            let rational = (mcpu.mcpu() as u64, 1);
            let above = Trigger::PercentileAbove(Percentile::p50(), mcpu);
            let below = Trigger::PercentileBelow(Percentile::p50(), mcpu);

            let above_ordering =
                percentile_rational(&Percentile::p50(), &actor_stats.current_mcpu, rational);
            let below_ordering =
                percentile_rational(&Percentile::p50(), &actor_stats.current_mcpu, rational);

            prop_assert_eq!(actor_stats.triggered_mcpu(&above), above_ordering.is_gt());
            prop_assert_eq!(actor_stats.triggered_mcpu(&below), below_ordering.is_lt());
        }

        /// Property: StdDevsAbove mCPU trigger matches `stddev_rational` predicate.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_mcpu_stddev_iff_predicate(
            mcpu_value in 200u16..800u16,
            threshold in 256u16..768u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: true,
                avg_work: false,
                show_thread_info: false,
                percentiles_mcpu: vec![Percentile::p50()],
                percentiles_work: vec![],
                std_dev_mcpu: vec![StdDev::one()],
                std_dev_work: vec![],
                trigger_mcpu: vec![],
                trigger_work: vec![],
                usage_review: false,
                refresh_rate_in_bits: 2,
                window_bucket_in_bits: 2,
            });
            let mut actor_stats = ActorStatsComputer::default();
            actor_stats.init(metadata, 1000);
            fill_window(&mut actor_stats, mcpu_value, 0);

            let mcpu = MCPU::new(threshold).expect("valid");
            let window_bits = actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits;
            let rule = Trigger::StdDevsAbove(StdDev::one(), mcpu);
            let ordering = stddev_rational(
                actor_stats.mcpu_std_dev(),
                window_bits,
                &StdDev::one(),
                &actor_stats.current_mcpu,
                (mcpu.mcpu() as u64, 1),
            );
            prop_assert_eq!(actor_stats.triggered_mcpu(&rule), ordering.is_gt());
        }

        /// Property: work PercentileAbove trigger matches `percentile_rational`.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_work_percentile_iff_predicate(
            work_value in 20u16..80u16,
            threshold_pct in 30u16..70u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: false,
                avg_work: true,
                show_thread_info: false,
                percentiles_mcpu: vec![],
                percentiles_work: vec![Percentile::p50()],
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
            fill_window(&mut actor_stats, 0, work_value);

            let work = Work::new(threshold_pct as f32).expect("valid");
            let rule = Trigger::PercentileAbove(Percentile::p50(), work);
            let ordering = percentile_rational(
                &Percentile::p50(),
                &actor_stats.current_work,
                work.rational(),
            );
            actor_stats.work_trigger.push((rule, AlertColor::Red));
            prop_assert_eq!(
                actor_stats.trigger_alert_level(&AlertColor::Red),
                ordering.is_gt()
            );
        }

        /// Property: work StdDevsAbove trigger matches `stddev_rational` predicate.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_trigger_work_stddev_iff_predicate(
            work_value in 20u16..80u16,
            threshold_pct in 30u16..70u16,
        ) {
            let metadata = Arc::new(ActorMetaData {
                ident: ActorIdentity::new(1, "test_actor", None),
                remote_details: None,
                avg_mcpu: false,
                avg_work: true,
                show_thread_info: false,
                percentiles_mcpu: vec![],
                percentiles_work: vec![Percentile::p50()],
                std_dev_mcpu: vec![],
                std_dev_work: vec![StdDev::one()],
                trigger_mcpu: vec![],
                trigger_work: vec![],
                usage_review: false,
                refresh_rate_in_bits: 2,
                window_bucket_in_bits: 2,
            });
            let mut actor_stats = ActorStatsComputer::default();
            actor_stats.init(metadata, 1000);
            fill_window(&mut actor_stats, 0, work_value);

            let work = Work::new(threshold_pct as f32).expect("valid");
            let rule = Trigger::StdDevsAbove(StdDev::one(), work);
            let window_bits = actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits;
            let ordering = stddev_rational(
                actor_stats.work_std_dev(),
                window_bits,
                &StdDev::one(),
                &actor_stats.current_work,
                work.rational(),
            );
            actor_stats.work_trigger.push((rule, AlertColor::Red));
            prop_assert_eq!(
                actor_stats.trigger_alert_level(&AlertColor::Red),
                ordering.is_gt()
            );
        }
    }
}

#[cfg(test)]
// ss[related telemetry.prometheus-metrics]
mod extra_tests {
    // ss[related philosophy.structural-hierarchy]
    use std::sync::Arc;

    // ss[related telemetry.prometheus-metrics]
    use proptest::prelude::*;

    // ss[related philosophy.structural-hierarchy]
    use crate::actor_builder_units::{Percentile, Work, MCPU};
    // ss[related telemetry.prometheus-metrics]
    use crate::actor_stats::*;
    // ss[related philosophy.structural-hierarchy]
    use crate::channel_stats::{DOT_ORANGE, DOT_RED, DOT_YELLOW};
    // ss[related philosophy.structural-hierarchy]
    use crate::monitor::ActorMetaData;
    // ss[related telemetry.prometheus-metrics]
    use crate::{logging_util, ActorIdentity, AlertColor, StdDev, Trigger};

    // ss[related philosophy.structural-hierarchy]
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

    // --- histogram smoke tests ---

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_init_histogram_creation_errors() {
        let _ = logging_util::steady_logger::initialize();
        let metadata = Arc::new(ActorMetaData {
            ident: ActorIdentity::new(1, "test", None),
            remote_details: None,
            avg_mcpu: true,
            avg_work: true,
            show_thread_info: false,
            percentiles_mcpu: vec![Percentile::p50()],
            percentiles_work: vec![Percentile::p50()],
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
        assert!(actor_stats.build_mcpu_histogram);
        assert!(actor_stats.build_work_histogram);
    }

    #[test]
    // ss[verify telemetry.prometheus-metrics]
    fn test_accumulate_data_frame_histogram_errors() {
        let _ = logging_util::steady_logger::initialize();
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
        actor_stats.accumulate_data_frame(1024, 100);
        assert!(!actor_stats.history_mcpu.is_empty());
        assert!(!actor_stats.history_work.is_empty());
    }

    ss_proptest! {

        /// Property: alert color priority — Red beats Orange beats Yellow when multiple fire.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_alert_color_priority(
            mcpu_value in 600u16..900u16,
            work_value in 7100u16..9500u16,
        ) {
            let _ = logging_util::steady_logger::initialize();
            let mut actor_stats = setup_actor_with_triggers();
            actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));
            actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m512()), AlertColor::Orange));
            actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p70()), AlertColor::Red));

            let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
            for _ in 0..total_frames {
                actor_stats.accumulate_data_frame(mcpu_value, work_value);
            }

            prop_assume!(actor_stats.trigger_alert_level(&AlertColor::Red));

            let mut dot_label = String::new();
            let mut tooltip = String::new();
            let mut metric_text = String::new();
            let (color, _) = actor_stats.compute(
                &mut dot_label,
                &mut tooltip,
                &mut metric_text,
                Some((mcpu_value, work_value)),
                0,
                false,
                false,
                None,
                None,
                true,
            );

            prop_assert_eq!(color, DOT_RED);
            prop_assert!(actor_stats.trigger_alert_level(&AlertColor::Yellow));
            prop_assert!(actor_stats.trigger_alert_level(&AlertColor::Orange));
        }

        /// Property: compute returns grey when no triggers are configured.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_no_triggers_grey(mcpu_value in 100u16..900u16, work_value in 10u16..90u16) {
            let _ = logging_util::steady_logger::initialize();
            let mut actor_stats = setup_actor_with_triggers();
            let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
            for _ in 0..total_frames {
                actor_stats.accumulate_data_frame(mcpu_value, work_value);
            }

            let mut dot_label = String::new();
            let mut tooltip = String::new();
            let mut metric_text = String::new();
            let (color, _) = actor_stats.compute(
                &mut dot_label,
                &mut tooltip,
                &mut metric_text,
                Some((mcpu_value, work_value)),
                0,
                false,
                false,
                None,
                None,
                true,
            );
            prop_assert_eq!(color, crate::channel_stats::DOT_GREY);
        }

        /// Property: Yellow-only trigger yields yellow dot color, not red/orange.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_single_yellow_alert(mcpu_value in 300u16..500u16) {
            let _ = logging_util::steady_logger::initialize();
            let mut actor_stats = setup_actor_with_triggers();
            actor_stats.mcpu_trigger.push((Trigger::AvgAbove(MCPU::m256()), AlertColor::Yellow));

            let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
            for _ in 0..total_frames {
                actor_stats.accumulate_data_frame(mcpu_value, 30);
            }

            let mut dot_label = String::new();
            let mut tooltip = String::new();
            let mut metric_text = String::new();
            let (color, _) = actor_stats.compute(
                &mut dot_label,
                &mut tooltip,
                &mut metric_text,
                Some((mcpu_value, 30)),
                0,
                false,
                false,
                None,
                None,
                true,
            );
            prop_assert_eq!(color, DOT_YELLOW);
            prop_assert!(!actor_stats.trigger_alert_level(&AlertColor::Red));
            prop_assert!(!actor_stats.trigger_alert_level(&AlertColor::Orange));
        }

        /// Property: Orange-only work trigger yields orange, not red.
        #[test]
        // ss[verify telemetry.prometheus-metrics]
        // ss[verify verify.process.proptest]
        fn proptest_single_orange_alert(work_value in 45u16..65u16) {
            let _ = logging_util::steady_logger::initialize();
            let mut actor_stats = setup_actor_with_triggers();
            actor_stats.work_trigger.push((Trigger::AvgAbove(Work::p40()), AlertColor::Orange));

            let total_frames = 1 << (actor_stats.window_bucket_in_bits + actor_stats.refresh_rate_in_bits + 1);
            for _ in 0..total_frames {
                actor_stats.accumulate_data_frame(200, work_value);
            }

            let mut dot_label = String::new();
            let mut tooltip = String::new();
            let mut metric_text = String::new();
            let (color, _) = actor_stats.compute(
                &mut dot_label,
                &mut tooltip,
                &mut metric_text,
                Some((200, work_value)),
                0,
                false,
                false,
                None,
                None,
                true,
            );
            prop_assert_eq!(color, DOT_ORANGE);
            prop_assert!(!actor_stats.trigger_alert_level(&AlertColor::Red));
        }
    }
}
