// ss[related telemetry.dot-export]
use bytes::{BufMut, BytesMut};

use super::escape::escape_dot_quotes;

pub(crate) fn render_edge_internal(
    dot_graph: &mut BytesMut,
    from_name: &'static str,
    from_suffix: Option<usize>,
    to_name: &'static str,
    to_suffix: Option<usize>,
    label: &str,
    color: &str,
    pen_width: &str,
    style: &str,
    sidecar: bool,
    headlabel: &str,
    taillabel: &str,
    tooltip: &str,
    escape_buf: &mut String,
) {
    dot_graph.put_slice(b"\"");
    dot_graph.put_slice(from_name.as_bytes());
    if let Some(s) = from_suffix {
        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
    }
    dot_graph.put_slice(b"\" -> \"");
    dot_graph.put_slice(to_name.as_bytes());
    if let Some(s) = to_suffix {
        dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
    }
    dot_graph.put_slice(b"\" [label=\"");
    escape_dot_quotes(escape_buf, label);
    dot_graph.put_slice(escape_buf.as_bytes());

    if !headlabel.is_empty() {
        dot_graph.put_slice(b"\", headlabel=\"");
        escape_dot_quotes(escape_buf, headlabel);
        dot_graph.put_slice(escape_buf.as_bytes());
    }
    if !taillabel.is_empty() {
        dot_graph.put_slice(b"\", taillabel=\"");
        escape_dot_quotes(escape_buf, taillabel);
        dot_graph.put_slice(escape_buf.as_bytes());
    }
    if !tooltip.is_empty() {
        dot_graph.put_slice(b"\", tooltip=\"");
        escape_dot_quotes(escape_buf, tooltip);
        dot_graph.put_slice(escape_buf.as_bytes());

        // NOTE: labeltooltip is NOT added here because it is unreliable in Graphviz JS
        // rendering. Instead, the tooltip <title> element is cloned from each edge group
        // onto its label <text> element by dot-viewer.js after the SVG is injected into the DOM.
    }

    dot_graph.put_slice(b"\", color=\"");
    dot_graph.put_slice(color.as_bytes());
    dot_graph.put_slice(b"\", penwidth=");
    dot_graph.put_slice(pen_width.as_bytes());
    dot_graph.put_slice(style.as_bytes());
    dot_graph.put_slice(b"];\n");

    if sidecar {
        dot_graph.put_slice(b"{rank=same; \"");
        dot_graph.put_slice(to_name.as_bytes());
        if let Some(s) = to_suffix {
            dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
        }
        dot_graph.put_slice(b"\" \"");
        dot_graph.put_slice(from_name.as_bytes());
        if let Some(s) = from_suffix {
            dot_graph.put_slice(itoa::Buffer::new().format(s).as_bytes());
        }
        dot_graph.put_slice(b"\"}\n");
    }
}

#[cfg(test)]
// ss[related telemetry.dot-export]
mod render_proptest {
    use super::*;
    use crate::ss_proptest;
    use proptest::prelude::*;

    fn render_sample(
        from: &'static str,
        from_suffix: Option<usize>,
        to: &'static str,
        to_suffix: Option<usize>,
        label: &str,
        sidecar: bool,
        headlabel: &str,
        taillabel: &str,
        tooltip: &str,
    ) -> String {
        let mut dot = BytesMut::new();
        let mut escape_buf = String::new();
        render_edge_internal(
            &mut dot,
            from,
            from_suffix,
            to,
            to_suffix,
            label,
            "green",
            "1",
            "",
            sidecar,
            headlabel,
            taillabel,
            tooltip,
            &mut escape_buf,
        );
        String::from_utf8(dot.to_vec()).expect("utf8")
    }

    ss_proptest! {
        /// Property: rendered edges contain quoted from/to node names.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_render_edge_contains_endpoints(
            from_suffix in prop::option::of(0usize..10),
            to_suffix in prop::option::of(0usize..10),
            label in r#"[A-Za-z0-9 _.-]{0,32}"#,
        ) {
            let dot = render_sample("alpha", from_suffix, "beta", to_suffix, &label, false, "", "", "");
            prop_assert!(dot.contains("\"alpha"));
            prop_assert!(dot.contains("\"beta"));
            prop_assert!(dot.contains(" -> "));
        }

        /// Property: optional head/taillabel/tooltip sections appear when non-empty.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_render_edge_optional_labels(
            head in prop::option::of(r#"[A-Za-z0-9]{1,8}"#),
            tail in prop::option::of(r#"[A-Za-z0-9]{1,8}"#),
            tip in prop::option::of(r#"[A-Za-z0-9]{1,12}"#),
        ) {
            let dot = render_sample(
                "from",
                None,
                "to",
                None,
                "edge",
                false,
                head.as_deref().unwrap_or(""),
                tail.as_deref().unwrap_or(""),
                tip.as_deref().unwrap_or(""),
            );
            if let Some(h) = &head {
                prop_assert!(dot.contains("headlabel="));
                prop_assert!(dot.contains(h));
            }
            if let Some(t) = &tail {
                prop_assert!(dot.contains("taillabel="));
                prop_assert!(dot.contains(t));
            }
            if let Some(t) = &tip {
                prop_assert!(dot.contains("tooltip="));
                prop_assert!(dot.contains(t));
            }
        }

        /// Property: sidecar edges emit a rank=same block for the endpoints.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_render_edge_sidecar_rank_same(_case in 0..1u8) {
            let dot = render_sample("x", Some(1), "y", Some(2), "CH", true, "", "", "");
            const RANK_SAME: &str = "{rank=same;";
            prop_assert!(dot.contains(RANK_SAME));
            prop_assert!(dot.contains("\"x1\""));
            prop_assert!(dot.contains("\"y2\""));
        }

        /// Property: embedded double-quotes in labels are escaped for valid DOT.
        #[test]
        // ss[verify telemetry.dot-export]
        // ss[verify verify.process.proptest]
        fn proptest_render_edge_escapes_embedded_quotes(
            inner in r#"[A-Za-z0-9]{0,8}"#,
        ) {
            let label = format!("say \"{inner}\"");
            let dot = render_sample("a", None, "b", None, &label, false, "", "", "");
            let expected = format!("say '{}'", inner);
            prop_assert!(dot.contains(&expected));
            prop_assert!(dot.contains(" -> "));
        }
    }
}
