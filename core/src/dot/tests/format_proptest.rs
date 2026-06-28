// ss[related telemetry.dot-export]
use crate::ss_proptest;
use proptest::prelude::*;
use super::super::{
    actor_fillcolor_hex_into, color_to_rgb, escape_dot_quotes, escape_node_tooltip_text,
    mean_avg_fill_percent, rgb_to_hex_into,
};

ss_proptest! {

    /// Property: escape_dot_quotes replaces double quotes with single quotes.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_escape_dot_quotes_replaces_double_quotes(s in r#".{0,64}"#) {
        let mut out = String::new();
        escape_dot_quotes(&mut out, &s);
        prop_assert!(!out.contains('"'));
        let expected_len = s.chars().count();
        prop_assert_eq!(out.chars().count(), expected_len);
    }

    /// Property: rgb_to_hex_into emits #RRGGBB uppercase hex.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_rgb_to_hex_format(r in 0u32..300, g in 0u32..300, b in 0u32..300) {
        let mut buf = String::new();
        rgb_to_hex_into(&mut buf, r, g, b);
        prop_assert_eq!(buf.len(), 7);
        prop_assert!(buf.starts_with('#'));
        prop_assert!(buf.chars().skip(1).all(|c| c.is_ascii_hexdigit()));
    }

    /// Property: actor_fillcolor_hex_into always produces #RRGGBB.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_actor_fillcolor_hex_prefix(color in prop::sample::select(vec!["red", "green", "blue", "grey", ""])) {
        let mut buf = String::new();
        actor_fillcolor_hex_into(&mut buf, color);
        prop_assert!(buf.starts_with('#'));
        prop_assert_eq!(buf.len(), 7);
    }

    /// Property: mean_avg_fill_percent stays in 1..=100 when nonzero samples exist.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_mean_avg_fill_percent_bounded(samples in prop::collection::vec(0u8..=100, 0..16)) {
        let opts: Vec<Option<u8>> = samples.into_iter().map(Some).collect();
        if let Some(mean) = mean_avg_fill_percent(opts.iter()) {
            prop_assert!(mean <= 100);
        }
    }

    /// Property: named colors map to bounded RGB components.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_color_to_rgb_bounded(name in prop::sample::select(vec!["red", "green", "blue", "grey", "yellow", "unknown"])) {
        let (r, g, b) = color_to_rgb(name);
        prop_assert!(r <= 255 && g <= 255 && b <= 255);
    }

    /// Property: tooltip escape never leaves bare double quotes.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_escape_node_tooltip_no_bare_quotes(s in r#"[^\n]{0,64}"#) {
        let mut out = String::new();
        escape_node_tooltip_text(&mut out, &s);
        prop_assert!(!out.contains('"'));
    }

    /// Property: mean_avg_fill_percent ignores zero samples when computing mean.
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_mean_avg_fill_ignores_zeros(
        nonzero in prop::collection::vec(1u8..=100, 1..8),
        zeros in 0usize..8,
    ) {
        let mut samples: Vec<Option<u8>> = nonzero.into_iter().map(Some).collect();
        samples.extend(std::iter::repeat(None).take(zeros));
        if let Some(mean) = mean_avg_fill_percent(samples.iter()) {
            let nz: Vec<u8> = samples.into_iter().flatten().filter(|&v| v > 0).collect();
            if !nz.is_empty() {
                let expected = (nz.iter().map(|&v| u32::from(v)).sum::<u32>() / nz.len() as u32) as u8;
                prop_assert_eq!(mean, expected);
            }
        }
    }

    /// Property: all-zero runner samples yield no mean (rollup omits avg fill).
    #[test]
    // ss[verify telemetry.dot-export]
    // ss[verify verify.process.proptest]
    fn proptest_mean_avg_fill_all_zero_or_none(len in 0usize..16) {
        let samples: Vec<Option<u8>> = (0..len).map(|_| Some(0u8)).collect();
        prop_assert_eq!(mean_avg_fill_percent(samples.iter()), None);
    }
}
