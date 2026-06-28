// ss[related telemetry.dot-export]
use std::collections::BTreeMap;
use std::fmt::Write;

use crate::channel_stats::ChannelStatsComputer;
use crate::dot_edge::Edge;

use super::colors::rgb_to_hex_into;
use super::MAX_INLINE_AVG_FILL_LANES;

/// Single hex color: arithmetic mean of lane RGBs (DOT multi-lane / bundle rollup).
pub(crate) fn hex_color_average_into(out: &mut String, lane_rgbs: &[(u32, u32, u32)]) {
    if lane_rgbs.is_empty() {
        rgb_to_hex_into(out, 128, 128, 128);
        return;
    }
    let n = lane_rgbs.len() as u32;
    let r = lane_rgbs.iter().map(|(r, _, _)| *r).sum::<u32>() / n;
    let g = lane_rgbs.iter().map(|(_, g, _)| *g).sum::<u32>() / n;
    let b = lane_rgbs.iter().map(|(_, _, b)| *b).sum::<u32>() / n;
    rgb_to_hex_into(out, r, g, b);
}

/// Per-resolved-edge color name counts for tooltips (e.g. `Lane colors: 3 red, 120 grey`).
/// Reuses `counts` across calls to avoid allocating a new `BTreeMap` per line.
pub(crate) fn format_lane_color_histogram_into(
    counts: &mut BTreeMap<&'static str, usize>,
    out: &mut String,
    lane_colors: &[&'static str],
) {
    counts.clear();
    for c in lane_colors {
        *counts.entry(*c).or_insert(0) += 1;
    }
    out.clear();
    out.push_str("Lane colors: ");
    let mut first = true;
    for (name, n) in counts.iter() {
        if !first {
            out.push_str(", ");
        }
        first = false;
        let _ = write!(out, "{} {}", n, name);
    }
}

/// Mean whole-percent avg fill from channel edges; ignores lanes with no `Some` sample
/// or with a zero percent (idle/cold channels).
pub(crate) fn mean_avg_fill_from_edge_slice(edges: &[&Edge]) -> Option<u8> {
    let mut sum = 0u32;
    let mut count = 0u32;
    for e in edges {
        if let Some(n) = e.stats_computer.avg_filled_whole_percent() {
            if n > 0 {
                sum += u32::from(n);
                count += 1;
            }
        }
    }
    (count > 0).then(|| (sum / count) as u8)
}

/// Multi-lane `Avg fill` for DOT: comma list when `edges.len() <=` [`MAX_INLINE_AVG_FILL_LANES`], else
/// a single `mean, N ch` line (see module constant). Omits the line entirely when no lane has a sample
/// (`None`) or all samples are zero (idle/cold channels).
pub(crate) fn format_avg_fill_rollup_line_into(out: &mut String, edges: &[&Edge]) {
    out.clear();
    if edges.is_empty() {
        return;
    }
    if edges.len() > MAX_INLINE_AVG_FILL_LANES {
        let n = edges.len();
        if let Some(m) = mean_avg_fill_from_edge_slice(edges) {
            let _ = write!(out, "Avg fill: {}% (mean, {} ch)\n", m, n);
        }
    } else {
        let mut started = false;
        for e in edges {
            if let Some(n) = e.stats_computer.avg_filled_whole_percent() {
                if n == 0 {
                    continue; // skip idle/cold channels (0% fill)
                }
                if !started {
                    out.push_str("Avg fill: ");
                    started = true;
                } else {
                    out.push_str(", ");
                }
                let _ = write!(out, "{}%", n);
            }
        }
        if started {
            out.push('\n');
        }
    }
}

/// Integer mean of `Some` percent values; skips zero values (idle/cold channels). `None` if there are no samples.
pub(crate) fn mean_avg_fill_percent<'a, I: Iterator<Item = &'a Option<u8>>>(iter: I) -> Option<u8> {
    let mut sum = 0u32;
    let mut count = 0u32;
    for o in iter {
        if let Some(n) = o {
            if *n > 0 {
                sum += u32::from(*n);
                count += 1;
            }
        }
    }
    (count > 0).then(|| (sum / count) as u8)
}

/// Per-channel hover line: rolling-window avg fill when enabled, else snapshot inflight/capacity.
pub(crate) fn append_channel_fill_tooltip(
    tooltip: &mut String,
    stats: &ChannelStatsComputer,
    saturation_score: f64,
) {
    if stats.show_avg_filled {
        if let Some(n) = stats.avg_filled_whole_percent() {
            if n > 0 {
                tooltip.push_str("\n ");
                let _ = write!(tooltip, "Avg fill: {}%\n", n);
            }
        }
    } else if saturation_score > 0.0 {
        let _ = write!(
            tooltip,
            "\n Instant fill: {}%\n",
            (saturation_score * 100.0) as usize
        );
    }
}
