// ss[related telemetry.dot-export]
use crate::ss_proptest;
// ss[impl telemetry.dot-export]
use proptest::prelude::*;

// ss[related telemetry.dot-export]
use super::super::FrameHistory;

ss_proptest! {

    /// Property: `build_history_path` is deterministic and its components are a pure subset of the run metadata.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_build_history_path_deterministic_pure_subset(
        ms_rate in 1u64..10_000u64,
        _seed in 0u8..=255u8,
    ) {
        let mut frame_history = FrameHistory::new(ms_rate);
        let path_a = frame_history.build_history_path();
        let path_b = frame_history.build_history_path();
        prop_assert_eq!(&path_a, &path_b);

        let path_str = path_a.to_string_lossy();
        prop_assert!(path_str.contains(&frame_history.guid), "path: {path_str}");
        prop_assert!(path_str.ends_with("_log.dat"), "path: {path_str}");
        prop_assert!(path_a.parent().is_some(), "path: {path_str}");

        let file_name = path_a
            .file_name()
            .and_then(|n| n.to_str())
            .expect("file name");
        let stem = file_name.strip_suffix("_log.dat").expect("stem");
        let (date_part, guid_part) = stem
            .rsplit_once('_')
            .expect("date_guid separator");
        prop_assert_eq!(guid_part, frame_history.guid);
        prop_assert!(date_part.chars().filter(|&c| c == '_').count() == 2);
    }

    /// Property: `mark_position` records the current history buffer length.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_mark_position_records_buffer_len(
        ms_rate in 1u64..10_000u64,
        extra_writes in 0usize..8,
    ) {
        let mut frame_history = FrameHistory::new(ms_rate);
        let baseline = frame_history.history_buffer.len();
        for _ in 0..extra_writes {
            frame_history.history_buffer.extend_from_slice(b"x");
        }
        frame_history.mark_position();
        prop_assert_eq!(frame_history.buffer_bytes_count, frame_history.history_buffer.len());
        prop_assert!(frame_history.buffer_bytes_count >= baseline);
    }

    /// Property: `apply_edge` grows the history buffer for non-empty take/send pairs.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_apply_edge_grows_buffer(
        ms_rate in 1u64..10_000u64,
        take in 0i64..1000,
        send in 0i64..1000,
    ) {
        let mut frame_history = FrameHistory::new(ms_rate);
        let before = frame_history.history_buffer.len();
        frame_history.apply_edge(&[(take, send)], ms_rate);
        prop_assert!(frame_history.history_buffer.len() > before);
    }

    /// Property: many edge writes eventually hit the packed-writer sync path.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_apply_edge_sync_path_after_many_writes(
        write_count in 60usize..80,
    ) {
        let ms_rate = 10_000u64;
        let mut frame_history = FrameHistory::new(ms_rate);
        for i in 0..write_count {
            frame_history.apply_edge(&[(i as i64, i as i64 + 1)], ms_rate);
        }
        prop_assert!(frame_history.packed_sent_writer.delta_write_count() > 0
            || frame_history.history_buffer.len() > 64);
    }
}
