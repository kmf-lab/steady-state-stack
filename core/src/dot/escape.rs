// ss[related telemetry.dot-export]

#[inline]
// ss[impl telemetry.dot-export]
pub(crate) fn escape_dot_quotes(out: &mut String, src: &str) {
    out.clear();
    out.reserve(src.len());
    for ch in src.chars() {
        if ch == '"' {
            out.push('\'');
        } else {
            out.push(ch);
        }
    }
}

#[inline]
// ss[related telemetry.dot-export]
pub(crate) fn escape_node_tooltip_text(out: &mut String, src: &str) {
    out.clear();
    out.reserve(src.len().saturating_mul(2));
    for ch in src.chars() {
        match ch {
            '"' => out.push('\''),
            '\n' => {
                out.push('\\');
                out.push('n');
            }
            _ => out.push(ch),
        }
    }
}
