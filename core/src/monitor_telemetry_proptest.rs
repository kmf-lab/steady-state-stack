//! Property tests for monitor telemetry counters and DOT subtitle coalescing.

use crate::channel_builder::ChannelBuilder;
use crate::monitor_telemetry::{DotSubtitleMailbox, SteadyTelemetrySend, DOT_SUBTITLE_MAX_CHARS};
use crate::ss_proptest;
use crate::{MONITOR_NOT, MONITOR_UNKNOWN};
use proptest::prelude::*;
use std::time::Instant;

ss_proptest! {
    /// Property: `process_event` never panics and preserves non-negative counts for valid indices.
    #[test]
    // ss[verify telemetry.prometheus-metrics]
    // ss[verify verify.process.proptest]
    fn proptest_process_event_saturating_add(
        index in 0usize..4,
        delta in 0isize..100,
        seed in 0usize..50,
    ) {
        let builder = ChannelBuilder::default().with_capacity(8);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();
        let mut send = SteadyTelemetrySend::new(tx, [seed; 4], [0, 1, 2, 3], Instant::now());
        let before = send.count[index];
        let resolved = send.process_event(index, index, delta);
        prop_assert_eq!(resolved, index);
        prop_assert!(send.count[index] >= before);
    }

    /// Property: unknown index resolution returns sentinel without panicking.
    #[test]
    #[ignore] //too slow for mutants
    // ss[verify telemetry.prometheus-metrics]
    // ss[verify verify.process.proptest]
    fn proptest_process_event_unknown_index(
        id in 4usize..64,
        delta in 0isize..32,
    ) {
        let builder = ChannelBuilder::default().with_capacity(8);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();
        let mut send = SteadyTelemetrySend::new(tx, [0; 4], [0; 4], Instant::now());
        let resolved = send.process_event(MONITOR_UNKNOWN, id, delta);
        prop_assert!(resolved >= MONITOR_NOT);
    }

    /// Property: MONITOR_NOT index is returned unchanged.
    #[test]
    // ss[verify telemetry.prometheus-metrics]
    // ss[verify verify.process.proptest]
    fn proptest_process_event_monitor_not_passthrough(
        delta in 0isize..16,
    ) {
        let builder = ChannelBuilder::default().with_capacity(8);
        let (tx, _rx) = builder.eager_build::<[usize; 4]>();
        let mut send = SteadyTelemetrySend::new(tx, [0; 4], [0; 4], Instant::now());
        prop_assert_eq!(send.process_event(MONITOR_NOT, 0, delta), MONITOR_NOT);
    }

    /// Property: subtitle mailbox collapses newlines and never exceeds max chars.
    #[test]
    // ss[verify telemetry.prometheus-metrics]
    // ss[verify verify.process.proptest]
    fn proptest_dot_subtitle_mailbox_truncates(
        prefix_len in 0usize..400,
        suffix_kind in 0u8..4,
    ) {
        let prefix = "x".repeat(prefix_len);
        let suffix = match suffix_kind {
            0 => String::new(),
            1 => "\ntail".to_string(),
            2 => "\r\nmore".to_string(),
            _ => format!("\n{}", "y".repeat(40)),
        };
        let m = DotSubtitleMailbox::new();
        let text = format!("{}{}", prefix, suffix);
        m.record(Some(&text));
        let taken = m.take_pending().expect("pending").expect("text");
        prop_assert!(!taken.contains('\n'));
        prop_assert!(!taken.contains('\r'));
        prop_assert!(taken.len() <= DOT_SUBTITLE_MAX_CHARS);
    }
}
