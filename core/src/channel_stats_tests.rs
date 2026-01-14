#[cfg(test)]
pub(crate) mod channel_stats_tests {
    use crate::channel_stats::*;
    use rand_distr::{Distribution, Normal};
    use rand::{rngs::StdRng, SeedableRng};
    use std::sync::Arc;
    use std::time::Duration;
    #[allow(unused_imports)]
    use log::*;
    use crate::monitor::ChannelMetaData;
    use crate::{logging_util, ActorName, StdDev, Trigger};
    use crate::actor_builder_units::Percentile;
    use crate::actor_stats::{ActorStatsComputer, ChannelBlock};
    use crate::channel_builder_units::{Filled, Rate};
    use crate::channel_stats_labels::ComputeLabelsConfig;
    ////////////////////////////////
    // Each of these tests cover both sides of above and below triggers with the matching label display
    ///////////////////////////////

    #[test]
    pub(crate) fn filled_avg_percent_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c { // 256 * 0.81 = 207
            computer.accumulate_data_frame((computer.capacity as f32 * 0.81f32) as u64, 100);
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\n");
        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::p80())), "Trigger should fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgAbove(Filled::p90())), "Trigger should not fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgBelow(Filled::p80())), "Trigger should not fire when the average filled is below");
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::p90())), "Trigger should fire when the average filled is below");
    }

    #[test]
    pub(crate) fn filled_avg_fixed_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c { // 256 * 0.81 = 207
            let filled = (computer.capacity as

                f32 * 0.81f32) as u64;
            let consumed = 100;
            computer.accumulate_data_frame(filled, consumed);
   
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\n");

        assert!(computer.triggered_filled(&Trigger::AvgAbove(Filled::Exact(16))), "Trigger should fire when the average filled is above");
        assert!(!computer.triggered_filled(&Trigger::AvgAbove(Filled::Exact((computer.capacity - 1) as u64))), "Trigger should not fire when the average filled is above");
        assert!(computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(220))), "Trigger should fire when the average filled is below");
        assert!(!computer.triggered_filled(&Trigger::AvgBelow(Filled::Exact(200))), "Trigger should not fire when the average filled is below");
    }

    #[test]
    pub(crate) fn filled_std_dev_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer.frame_rate_ms = 3;
        computer.show_avg_filled = true;

        let mean = computer.capacity as f64 * 0.81; // Mean value just above test
        let expected_std_dev = 10.0; // Standard deviation
        let normal = Normal::new(mean, expected_std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(value, 100);
  
        }

        computer.std_dev_filled.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\nfilled 2.5StdDev: 30.455 per frame (3ms duration)\n");

        computer.std_dev_filled.clear();
        computer.std_dev_filled.push(StdDev::one());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg filled: 80 %\nfilled StdDev: 12.182 per frame (3ms duration)\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p80())), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsAbove(StdDev::one(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p80())), "Trigger should not fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p90())), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::StdDevsBelow(StdDev::one(), Filled::p100())), "Trigger should fire when standard deviation from the average filled is below the threshold");
    }

    #[test]
    pub(crate) fn filled_percentile_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;

        actor.percentiles_filled.push(Percentile::p25());
        actor.percentiles_filled.push(Percentile::p50());
        actor.percentiles_filled.push(Percentile::p75());
        actor.percentiles_filled.push(Percentile::p90());

        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        assert!(computer.percentiles_filled.is_empty());
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        assert!(!computer.percentiles_filled.is_empty());

        let mean = computer.capacity as f64 * 0.13;
        let expected_std_dev = 10.0; // Standard deviation
        let normal = Normal::new(mean, expected_std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(value, 100);
        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "filled 25%ile 31 %\nfilled 50%ile 31 %\nfilled 75%ile 63 %\nfilled 90%ile 63 %\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p90(), Filled::Exact(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p50(), Filled::p90())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileAbove(Percentile::p50(), Filled::p100())), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p90(), Filled::Exact(80))), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(!computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p50(), Filled::Exact(17))), "Trigger should not fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_filled(&Trigger::PercentileBelow(Percentile::p50(), Filled::p100())), "Trigger should fire when standard deviation from the average filled is below the threshold");
    }

    ////////////////////////////////////////////////////////
    #[test]
    pub(crate) fn rate_avg_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 4;
        actor.refresh_rate_in_bits = 8;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None)
                                    , ActorName::new("2",None)
                                    , 30);
        computer.show_avg_rate = true;
        // We consume 100 messages every computer.frame_rate_ms which is 3 ms
        // So per ms we are consuming about 33.3 messages
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(0, 1000);

        }

        let display_label = compute_display_label(&mut computer);
        assert_eq!(display_label, "Avg rate: 33K per/sec\n");

        // NOTE: our triggers are in fixed units so they do not need to change if we modify
        // the frame rate, refresh rate or window rate.
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(32))), "Trigger should fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(34))), "Trigger should not fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(32))), "Trigger should not fire when the average is below");
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(34))), "Trigger should fire when the average is below");


        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(0, 1_000_000);

        }

        let display_label = compute_display_label(&mut computer);
        assert_eq!(display_label, "Avg rate: 33M per/sec\n");

        // NOTE: our triggers are in fixed units so they do not need to change if we modify
        // the frame rate, refresh rate or window rate.
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(32000))), "Trigger should fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(34000))), "Trigger should not fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(32000))), "Trigger should not fire when the average is below");
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(34000))), "Trigger should fire when the average is below");

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(0, 1_000_000_000);

        }

        let display_label = compute_display_label(&mut computer);
        assert_eq!(display_label, "Avg rate: 33B per/sec\n");

        // NOTE: our triggers are in fixed units so they do not need to change if we modify
        // the frame rate, refresh rate or window rate.
        assert!(computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(32_000_000))), "Trigger should fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgAbove(Rate::per_millis(34_000_000))), "Trigger should not fire when the average is above");
        assert!(!computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(32_000_000))), "Trigger should not fire when the average is below");
        assert!(computer.triggered_rate(&Trigger::AvgBelow(Rate::per_millis(34_000_000))), "Trigger should fire when the average is below");

    }

    #[test]
    pub(crate) fn rate_std_dev_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer.frame_rate_ms = 3;
        computer.show_avg_rate = true;

        let mean = computer.capacity as f64 * 0.81; // Mean value just above test
        let expected_std_dev = 10.0; // Standard deviation
        let normal = Normal::new(mean, expected_std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(100, value);

        }

        computer.std_dev_rate.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg rate: 68K per/sec\nrate 2.5StdDev: 30.455 per frame (3ms duration)\n");

        computer.std_dev_rate.clear();
        computer.std_dev_rate.push(StdDev::one());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg rate: 68K per/sec\nrate StdDev: 12.182 per frame (3ms duration)\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_rate(&Trigger::StdDevsAbove(StdDev::one(), Rate::per_millis(80))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::StdDevsAbove(StdDev::one(), Rate::per_millis(220))), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::StdDevsBelow(StdDev::one(), Rate::per_millis(80))), "Trigger should fire when standard deviation from the average filled is below the threshold");
        assert!(computer.triggered_rate(&Trigger::StdDevsBelow(StdDev::one(), Rate::per_millis(220))), "Trigger should fire when standard deviation from the average filled is below the threshold");
    }

    ///////////////////
    #[test]
    pub(crate) fn rate_percentile_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        actor.percentiles_rate.push(Percentile::p25());
        actor.percentiles_rate.push(Percentile::p50());
        actor.percentiles_rate.push(Percentile::p75());
        actor.percentiles_rate.push(Percentile::p90());
        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);

        let mean = computer.capacity as f64 * 0.13;
        let expected_std_dev = 10.0; // Standard deviation
        let normal = Normal::new(mean, expected_std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(100, value);

        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "rate 25%ile 25 per/sec\nrate 50%ile 28 per/sec\nrate 75%ile 38 per/sec\nrate 90%ile 49 per/sec\n");

        // Define a trigger with a standard deviation condition
        assert!(computer.triggered_rate(&Trigger::PercentileAbove(Percentile::p90(), Rate::per_millis(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::PercentileAbove(Percentile::p90(), Rate::per_millis(52))), "Trigger should not fire when standard deviation from the average filled is above the threshold");
        assert!(!computer.triggered_rate(&Trigger::PercentileBelow(Percentile::p90(), Rate::per_millis(47))), "Trigger should fire when standard deviation from the average filled is above the threshold");
        assert!(computer.triggered_rate(&Trigger::PercentileBelow(Percentile::p90(), Rate::per_millis(52))), "Trigger should not fire when standard deviation from the average filled is above the threshold");
    }

    fn compute_display_label(computer: &mut ChannelStatsComputer) -> String {
        let mut display_label = String::new();
        let mut metrics = String::new();
        if let Some(ref current_rate) = computer.current_rate {
            computer.compute_rate_labels( &mut display_label, &mut metrics, &current_rate);
        }
        if let Some(ref current_filled) = computer.current_filled {
            computer.compute_filled_labels(&mut display_label, &mut metrics, &current_filled);
        }
        if let Some(ref current_latency) = computer.current_latency {
            computer.compute_latency_labels(&mut display_label, &mut metrics, &current_latency);
        }
        display_label
    }

    /////////////////////////

    #[test]
    pub(crate) fn latency_avg_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer.frame_rate_ms = 3;
        computer.show_avg_latency = true;

        let mean = computer.capacity as f64 * 0.81; // Mean value just above test
        let expected_std_dev = 10.0; // Standard deviation
        let normal = Normal::new(mean, expected_std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            // 205 in flight and we consume 33 per 3ms frame 11 per ms, so 205/11 = 18.6ms
            let inflight_value = normal.sample(&mut rng).max(0.0).min(computer.capacity as f64) as u64;
            computer.accumulate_data_frame(inflight_value, 33);

        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg latency: 18K µs\n");

        assert!(computer.triggered_latency(&Trigger::AvgAbove(Duration::from_millis(5))), "Trigger should fire when the average is above");
        assert!(!computer.triggered_latency(&Trigger::AvgAbove(Duration::from_millis(21))), "Trigger should fire when the average is above");

        assert!(!computer.triggered_latency(&Trigger::AvgBelow(Duration::from_millis(5))), "Trigger should fire when the average is above");
        assert!(computer.triggered_latency(&Trigger::AvgBelow(Duration::from_millis(21))), "Trigger should fire when the average is above");
    }




    #[test]
    pub(crate) fn latency_std_dev_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);
        computer

            .frame_rate_ms = 3;
        computer.show_avg_latency = true;
        let mean = 5.0; // Mean rate
        let std_dev = 1.0; // Standard deviation
        let normal = Normal::new(mean, std_dev).expect("iternal error");
        let seed = [42; 32];
        let mut rng = StdRng::from_seed(seed);

        // Simulate rate data with a distribution
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            let consumed_value = normal.sample(&mut rng) as u64;
            computer.accumulate_data_frame(computer.capacity as u64 / 2, consumed_value);

        }

        computer.std_dev_latency.push(StdDev::two_and_a_half());
        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "Avg latency: 95K µs\nlatency 2.5StdDev: 79.329 per frame (3ms duration)\n");

        // Define a trigger for rate deviation above a threshold
        assert!(computer.triggered_latency(&Trigger::StdDevsAbove(StdDev::two_and_a_half(), Duration::from_millis(96 + 70))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(!computer.triggered_latency(&Trigger::StdDevsAbove(StdDev::two_and_a_half(), Duration::from_millis(96 + 90))), "Trigger should not fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(!computer.triggered_latency(&Trigger::StdDevsBelow(StdDev::two_and_a_half(), Duration::from_millis(96 + 70))), "Trigger should not fire when rate deviates above the mean by a std dev, exceeding 6");
        assert!(computer.triggered_latency(&Trigger::StdDevsBelow(StdDev::two_and_a_half(), Duration::from_millis(96 + 90))), "Trigger should fire when rate deviates above the mean by a std dev, exceeding 6");
    }

    #[test]
    pub(crate) fn latency_percentile_trigger() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 256;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        actor.percentiles_latency.push(Percentile::p25());
        actor.percentiles_latency.push(Percentile::p50());
        actor.percentiles_latency.push(Percentile::p75());
        actor.percentiles_latency.push(Percentile::p90());
        let mut computer = ChannelStatsComputer::default();
        computer.frame_rate_ms = 3;
        computer.init(&Arc::new(actor), ActorName::new("1",None), ActorName::new("2",None), 42);

        // Simulate rate data accumulation
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame((computer.capacity - 1) as u64, (5.0 * 1.2) as u64); // Simulating rate being consistently above a certain value

        }

        let display_label = compute_display_label(&mut computer);

        assert_eq!(display_label, "latency 25%ile 1785855 µs\nlatency 50%ile 1785855 µs\nlatency 75%ile 1785855 µs\nlatency 90%ile 1785855 µs\n");

        // Define a trigger for average rate above a threshold
        assert!(computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_millis(100))), "Trigger should fire when the average rate is above");
        assert!(computer.triggered_latency(&Trigger::PercentileAbove(Percentile::p90(), Duration::from_millis(1500))), "Trigger should fire when the average rate is above");
        assert!(!computer.triggered_latency(&Trigger::PercentileBelow(Percentile::p90(), Duration::from_millis(100))), "Trigger should fire when the average rate is below");
        assert!(computer.triggered_latency(&Trigger::PercentileBelow(Percentile::p90(), Duration::from_millis(1900))), "Trigger should fire when the average rate is below");
    }


    /// Helper function to set up a `ChannelStatsComputer` with specified parameters.
    fn setup_computer(capacity: usize, window_bits: u8, refresh_bits: u8) -> ChannelStatsComputer {
        let mut actor = ChannelMetaData::default();
        actor.capacity = capacity;
        actor.window_bucket_in_bits = window_bits;
        actor.refresh_rate_in_bits = refresh_bits;
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 42);
        computer.frame_rate_ms = 3;
        computer
    }


    /// Test histogram creation failure by using an extreme capacity.
    #[test]
    pub(crate) fn histogram_failure() {
        let _ = logging_util::steady_logger::initialize();
        let mut actor = ChannelMetaData::default();
        actor.capacity = usize::MAX; // Extremely large capacity to simulate failure
        actor.percentiles_filled.push(Percentile::p50()); // Force histogram creation
        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 42);
        computer.accumulate_data_frame(0, 100);
        // Expect error logging or graceful handling; no panic should occur
    }


    /// Test when the bucket is full and a new one is added.
    #[test]
    pub(crate) fn full_bucket() {
        let _ = logging_util::steady_logger::initialize();
        let mut computer = setup_computer(256, 2, 2);
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(100, 100);
        }
        assert!(computer.history_filled.len() > 1);
    }




    /// Test Prometheus metrics generation when enabled.
    #[cfg(feature = "prometheus_metrics")]
    #[test]
    pub(crate) fn prometheus_metrics_enabled() {
        let _ = logging_util::steady_logger::initialize();
        let mut computer = setup_computer(256, 2, 2);
        computer.accumulate_data_frame(100, 100);
        let mut display_label = String::new();
        let mut metrics = String::new();
        computer.compute(&mut display_label, &mut metrics, None, 100, 50);
        assert!(metrics.contains("inflight"));
        assert!(metrics.contains("send_total"));
        assert!(metrics.contains("take_total"));
    }

    /// Test behavior when capacity is zero (should return grey line).
    #[test]
    pub(crate) fn zero_capacity() {
        let _ = logging_util::steady_logger::initialize();
        let mut computer = ChannelStatsComputer::default();
        computer.capacity = 0; // Manually set to bypass init assertion
        let mut display_label = String::new();
        let mut metrics = String::new();
        let (color, thickness) = computer.compute(&mut display_label, &mut metrics, None, 100, 50);
        assert_eq!(color, DOT_GREY);
        assert_eq!(thickness, "1");
    }


    /// Test init method with labels and show_type
    #[test]
    fn test_init_with_labels_and_show_type() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 100;
        actor.labels = vec!["test_label1", "test_label2"];
        actor.show_type = Some("test_type");
        actor.display_labels = true;

        let mut computer = ChannelStatsComputer::default();
        computer.init(
            &Arc::new(actor),
            ActorName::new("from_actor", Some(42)),
            ActorName::new("to_actor", Some(99)),
            1000
        );

        // Should contain labels in prometheus_labels
        assert!(computer.prometheus_labels.contains("test_label1"));
        assert!(computer.prometheus_labels.contains("test_label2"));
        assert!(computer.prometheus_labels.contains("type=\"test_type\""));
        assert!(computer.prometheus_labels.contains("from=\"from_actor42\""));
        assert!(computer.prometheus_labels.contains("to=\"to_actor99\""));

        // display_labels should be Some
        assert!(computer.display_labels.is_some());
    }

    /// Test histogram creation errors by mocking extreme conditions
    #[test]
    fn test_histogram_creation_errors() {
        let _ = logging_util::steady_logger::initialize();

        // Test with capacity that might cause histogram errors
        let mut actor = ChannelMetaData::default();
        actor.capacity = u64::MAX as usize; // Extreme capacity
        actor.percentiles_filled.push(Percentile::p50());
        actor.percentiles_rate.push(Percentile::p50());
        actor.percentiles_latency.push(Percentile::p50());

        let mut computer = ChannelStatsComputer::default();
        // This should handle histogram creation gracefully and log errors
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 1000);
    }



    /// Test compute method with full prometheus metrics and all features
    // #[cfg(feature = "prometheus_metrics")]
    // #[test]
    // fn test_compute_full_prometheus_metrics() {
    //     let _ = util::steady_logger::initialize();
    //
    //     let mut actor = ChannelMetaData::default();
    //     actor.capacity = 100;
    //     actor.show_type = Some("test_type");
    //     actor.display_labels = true;
    //     actor.labels = vec!["label1", "label2"];
    //     actor.window_bucket_in_bits = 2;
    //     actor.refresh_rate_in_bits = 2;
    //     actor.show_total = true;
    //     actor.avg_filled = true;
    //     actor.avg_rate = true;
    //     actor.avg_latency = true;
    //     actor.percentiles_filled.push(Percentile::p50());
    //     actor.std_dev_inflight.push(StdDev::one());
    //     actor.trigger_filled.push((Trigger::AvgAbove(Filled::p50()), AlertColor::Yellow));
    //     actor.trigger_rate.push((Trigger::AvgAbove(Rate::per_millis(100)), AlertColor::Orange));
    //     actor.trigger_latency.push((Trigger::AvgAbove(Duration::from_millis(100)), AlertColor::Red));
    //     actor.line_expansion = 1.5;
    //
    //     let mut computer = ChannelStatsComputer::default();
    //     computer.init(&Arc::new(actor), ActorName::new("from", None), ActorName::new("to", None), 1000);
    //
    //     // Accumulate enough data to trigger bucket rotation
    //     let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
    //     for _ in 0..c {
    //         computer.accumulate_data_frame(50, 10);
    //     }
    //
    //     let mut display_label = String::new();
    //     let mut metric_text = String::new();
    //     let (color, thickness) = computer.compute(&mut display_label, &mut metric_text, Some(ActorName::new("test", None)), 1000, 500);
    //
    //     // Should contain prometheus metrics
    //     assert!(metric_text.contains("inflight{"));
    //     assert!(metric_text.contains("send_total{"));
    //     assert!(metric_text.contains("take_total{"));
    //
    //     // Should contain display elements
    //     assert!(display_label.contains("test_type"));
    //     assert!(display_label.contains("Window"));
    //     assert!(display_label.contains("label1"));
    //     assert!(display_label.contains("label2"));
    //     assert!(display_label.contains("Capacity: 100"));
    //     assert!(display_label.contains("Total:"));
    //
    //     // Should have alert colors based on triggers
    //     assert!(color == DOT_YELLOW || color == DOT_ORANGE || color == DOT_RED);
    // }

    /// Test compute method without prometheus feature (lines 407-427 should be skipped)
    #[cfg(not(feature = "prometheus_metrics"))]
    #[test]
    fn test_compute_without_prometheus() {
        let _ = logging_util::steady_logger::initialize();

        let mut computer = setup_basic_computer();
        let mut display_label = String::new();
        let mut metric_text = String::new();
        computer.compute(&mut display_label, &mut metric_text, None, 100, 50);

        // metric_text should be empty without prometheus feature
        assert!(metric_text.is_empty());
    }

   

    /// Test line thickness calculation based on traffic
    #[test]
    fn test_line_thickness_calculation() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 100;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;
        actor.percentiles_rate.push(Percentile::p80());
        actor.line_expansion = 2.0; // Test non-NaN line expansion

        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 1000);

        // Accumulate data to get current_rate
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(50, 1000); // High rate
        }

        let mut display_label = String::new();
        let mut metric_text = String::new();
        let (_, thickness) = computer.compute(&mut display_label, &mut metric_text, None, 1000, 500);

        // Should confirm line thickness remains stable at "1" despite traffic
        assert_eq!(thickness, "1"); 
    }

    /// Test std dev functions when current data is None
    #[test]
    fn test_std_dev_functions_with_none_current() {
        let _ = logging_util::steady_logger::initialize();

        let computer = ChannelStatsComputer::default();

        // These should return 0 or log info when current data is None
        assert_eq!(computer.rate_std_dev(), 0f32);
        assert_eq!(computer.filled_std_dev(), 0f32);
        assert_eq!(computer.latency_std_dev(), 0f32);
    }

    /// Test trigger functions when current data is None
    #[test]
    fn test_trigger_functions_with_none_current() {
        let _ = logging_util::steady_logger::initialize();

        let computer = ChannelStatsComputer::default();

        // All trigger functions should return Equal (false) when no current data
        assert_eq!(computer.avg_filled_percentage(&50, &100), std::cmp::Ordering::Equal);
        assert_eq!(computer.avg_filled_exact(&50), std::cmp::Ordering::Equal);
        assert_eq!(computer.avg_latency(&Duration::from_millis(100)), std::cmp::Ordering::Equal);
        assert_eq!(computer.stddev_filled_exact(&StdDev::one(), &50), std::cmp::Ordering::Equal);
        assert_eq!(computer.stddev_filled_percentage(&StdDev::one(), &50, &100), std::cmp::Ordering::Equal);
        assert_eq!(computer.stddev_latency(&StdDev::one(), &Duration::from_millis(100)), std::cmp::Ordering::Equal);
        assert_eq!(computer.percentile_filled_exact(&Percentile::p50(), &50), std::cmp::Ordering::Equal);
        assert_eq!(computer.percentile_filled_percentage(&Percentile::p50(), &50, &100), std::cmp::Ordering::Equal);
        assert_eq!(computer.percentile_latency(&Percentile::p50(), &Duration::from_millis(100)), std::cmp::Ordering::Equal);
    }

    /// Test percentile functions when histogram is None
    #[test]
    fn test_percentile_functions_with_none_histogram() {
        let _ = logging_util::steady_logger::initialize();

        let mut computer = ChannelStatsComputer::default();
        computer.current_filled = Some(ChannelBlock::default()); // No histogram
        computer.current_rate = Some(ChannelBlock::default());
        computer.current_latency = Some(ChannelBlock::default());

        // Should return Equal when histogram is None
        assert_eq!(computer.percentile_filled_exact(&Percentile::p50(), &50), std::cmp::Ordering::Equal);
        assert_eq!(computer.percentile_filled_percentage(&Percentile::p50(), &50, &100), std::cmp::Ordering::Equal);
        assert_eq!(computer.percentile_latency(&Percentile::p50(), &Duration::from_millis(100)), std::cmp::Ordering::Equal);
    }

    /// Test actor_config method
    #[test]
    fn test_actor_config_method() {
        let _ = logging_util::steady_logger::initialize();

        let actor_stats = ActorStatsComputer::default();
        let config = ComputeLabelsConfig::actor_config(&actor_stats, (1, 1000), (1,1), 100, true, false, false);

        assert_eq!(config.frame_rate_ms, actor_stats.frame_rate_ms);
        assert_eq!(config.runner_adjust, (1, 1000));
        assert_eq!(config.max_value, 100);
        assert!(config.show_avg);
    }





    /// Test number formatting in show_total
    #[test]
    fn test_show_total_number_formatting() {
        let _ = logging_util::steady_logger::initialize();

        let mut actor = ChannelMetaData::default();
        actor.capacity = 100;
        actor.show_total = true;

        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 1000);

        let mut display_label = String::new();
        let mut metric_text = String::new();

        // Test with large number to trigger comma formatting
        computer.compute(&mut display_label, &mut metric_text, None, 1_234_567, 1_234_567);

        // Should contain formatted number with commas
        assert!(display_label.contains("1234K"),"found: {}",&display_label);
    }

    /// Helper function to create a basic computer for testing
    fn setup_basic_computer() -> ChannelStatsComputer {
        let mut actor = ChannelMetaData::default();
        actor.capacity = 100;
        actor.window_bucket_in_bits = 2;
        actor.refresh_rate_in_bits = 2;

        let mut computer = ChannelStatsComputer::default();
        computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 1000);

        // Accumulate some data to get current values
        let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
        for _ in 0..c {
            computer.accumulate_data_frame(50, 100);
        }

        computer
    }

    /// Test comprehensive latency computation with zero rate edge cases
    #[test]
    fn test_latency_computation_zero_rate() {
        let _ = logging_util::steady_logger::initialize();

        let mut computer = setup_basic_computer();

        // Test with zero rate - should result in zero latency
        computer.accumulate_data_frame(100, 0);

        // The latency calculation should handle zero rate gracefully
        assert!(!computer.history_latency.is_empty());
    }

    // Test bucket refresh with histogram creation errors during refresh
    // #[test]
    // fn test_bucket_refresh_histogram_errors() {
    //     let _ = util::steady_logger::initialize();
    //
    //     let mut actor = ChannelMetaData::default();
    //     actor.capacity = 100;
    //     actor.window_bucket_in_bits = 1; // Small window to trigger refresh quickly
    //     actor.refresh_rate_in_bits = 1;
    //     actor.percentiles_filled.push(Percentile::p50());
    //     actor.percentiles_rate.push(Percentile::p50());
    //     actor.percentiles_latency.push(Percentile::p50());
    //
    //     let mut computer = ChannelStatsComputer::default();
    //     computer.init(&Arc::new(actor), ActorName::new("1", None), ActorName::new("2", None), 1000);
    //
    //     // Force bucket refresh multiple times
    //     for _ in 0..10 {
    //         let c = 1 << (computer.window_bucket_in_bits + computer.refresh_rate_in_bits);
    //         for _ in 0..c {
    //             computer.accumulate_data_frame(50, 100);
    //         }
    //     }
    //
    //     // Should have handled histogram creation during refresh
    //     assert!(computer.history_filled.len() > 0);
    //     assert!(computer.history_rate.len() > 0);
    //     assert!(computer.history_latency.len() > 0);
    // }
}
