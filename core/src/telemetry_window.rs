//! Shared telemetry rolling-window bit sizing for actors and channels.
//!
//! The metrics collector advances DOT statistics once per telemetry frame (`frame_rate_ms`).
//! Both channel edge rollups and actor [`crate::actor_stats::ActorStatsComputer`] accumulation
//! receive one merged sample per frame, so refresh/window bit depths must use the same
//! frame-based math.

use std::time::Duration;

/// Computes `(refresh_rate_in_bits, window_bucket_in_bits)` for rolling telemetry windows.
///
/// One **sample** = one telemetry frame (one collector tick). This matches
/// [`crate::channel_stats::ChannelStatsComputer`] and [`crate::actor_stats::ActorStatsComputer`]
/// behavior after each `accumulate_data_frame` / channel equivalent.
pub(crate) fn compute_refresh_window_frames(
    frame_rate_ms: u128,
    refresh: Duration,
    window: Duration,
) -> (u8, u8) {
    if frame_rate_ms == 0 {
        return (0, 0);
    }

    let frame_micros = 1000u128 * frame_rate_ms;

    // How many frames should a "refresh bucket" contain? Clamp to at least 1 frame.
    let mut frames_per_refresh = refresh.as_micros() / frame_micros;
    if frames_per_refresh == 0 {
        frames_per_refresh = 1;
    }

    let refresh_in_bits = (frames_per_refresh as f32).log2().ceil() as u8;

    // The refresh bucket duration in microseconds: (2^bits) frames * frame duration.
    let refresh_in_micros = (1000u128 << refresh_in_bits) * frame_rate_ms;

    // How many refresh buckets fit into the desired window? Clamp to at least 1 bucket.
    let mut buckets_per_window = window.as_micros() as f32 / refresh_in_micros as f32;
    if !buckets_per_window.is_finite() || buckets_per_window < 1.0 {
        buckets_per_window = 1.0;
    }
    let window_in_bits = buckets_per_window.log2().ceil() as u8;

    (refresh_in_bits, window_in_bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_frame_rate_returns_zeros() {
        assert_eq!(
            compute_refresh_window_frames(0, Duration::from_secs(1), Duration::from_secs(10)),
            (0, 0)
        );
    }

    #[test]
    fn hundred_ms_frame_one_ten_seconds_sized() {
        let (r, w) = compute_refresh_window_frames(
            100,
            Duration::from_secs(1),
            Duration::from_secs(10),
        );
        assert!(r > 0 && w > 0);
        // 10 frames per 1s refresh => ceil(log2(10)) == 4; 10s window / ~1.6s bucket => ceil(log2(6.25)) == 3.
        assert_eq!((r, w), (4, 3));
    }
}
