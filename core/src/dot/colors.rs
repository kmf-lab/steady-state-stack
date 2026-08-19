// ss[related telemetry.dot-export]
use std::fmt::Write;

// ss[impl telemetry.dot-export]
use super::ACTOR_FILL_TINT_PERCENT;

/// Maps a color name to its RGB components.
// ss[related telemetry.dot-export]
pub(crate) fn color_to_rgb(color: &str) -> (u32, u32, u32) {
    match color {
        "red" => (255, 0, 0),
        "green" => (0, 169, 0),
        "blue" => (0, 0, 255),
        "grey" | "gray" => (128, 128, 128),
        "yellow" => (255, 255, 0),
        "purple" => (128, 0, 128),
        "white" => (255, 255, 255),
        _ => (0, 0, 0), // black/default
    }
}

/// Writes `#RRGGBB` into `out` (reused across DOT builds to avoid per-edge `String` churn).
// ss[related telemetry.dot-export]
pub(crate) fn rgb_to_hex_into(out: &mut String, r: u32, g: u32, b: u32) {
    out.clear();
    let _ = write!(
        out,
        "#{:02X}{:02X}{:02X}",
        r.min(255),
        g.min(255),
        b.min(255)
    );
}

/// Actor node interior: white blended with `border_color` so the fill reads solid on black backgrounds.
// ss[related telemetry.dot-export]
pub(crate) fn actor_fillcolor_hex_into(out: &mut String, border_color: &str) {
    if border_color.is_empty() {
        rgb_to_hex_into(out, 255, 255, 255);
        return;
    }
    let k = ACTOR_FILL_TINT_PERCENT.clamp(1, 99);
    let (r, g, b) = color_to_rgb(border_color);
    let blend = |c: u32| (255u32 * (100 - k) + c * k) / 100;
    rgb_to_hex_into(out, blend(r), blend(g), blend(b));
}
