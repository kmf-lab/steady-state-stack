#!/usr/bin/env python3
"""Add ss[verify] before #[test] in cargo-steady-state sources."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "cargo-steady-state" / "src"

RULES: list[tuple[str, list[tuple[str, str]]]] = [
    ("main.rs", [
        ("test_extract_percent", "tooling.cargo-percent-parse"),
        ("build_driver_block_with_at_least", "tooling.cargo-capacity-driven"),
        ("build_driver_block_with_event_driven", "tooling.cargo-driver-strings"),
        ("event_driven_rx_bundle", "tooling.cargo-driver-strings"),
        ("capacity_driven_tx_bundle", "tooling.cargo-bundle-codegen"),
        ("at_most_every", "tooling.cargo-capacity-driven"),
        ("other_driver", "tooling.cargo-capacity-driven"),
        ("at_least_every_plus_event", "tooling.cargo-capacity-driven"),
        ("event_driven_percent", "tooling.cargo-percent-parse"),
        ("capacity_driven_percent", "tooling.cargo-percent-parse"),
        ("fizz_buzz", "tooling.cargo-driver-strings"),
        ("unnamed1", "tooling.cargo-driver-strings"),
        ("pbft", "tooling.cargo-driver-strings"),
        ("circle", "tooling.cargo-driver-strings"),
        ("monitor_defs", "tooling.cargo-driver-strings"),
        ("derive_block", "tooling.cargo-driver-strings"),
    ]),
    ("extract_details.rs", [
        ("parse_parts", "tooling.cargo-driver-strings"),
        ("extract_actor_driver", "tooling.cargo-driver-strings"),
        ("extract_consume_pattern", "tooling.cargo-driver-strings"),
        ("build_pm", "tooling.cargo-driver-strings"),
        ("roll_up_bundle", "tooling.cargo-bundle-codegen"),
        ("parse_parts", "tooling.cargo-percent-parse"),
        ("extract_type_name", "tooling.cargo-driver-strings"),
        ("extract_capacity", "tooling.cargo-driver-strings"),
        ("extract_module", "tooling.cargo-driver-strings"),
        ("extract_channel_name", "tooling.cargo-driver-strings"),
        ("to_snake_case", "tooling.cargo-driver-strings"),
        ("find_start_position", "tooling.cargo-driver-strings"),
        ("correct_format", "tooling.cargo-driver-strings"),
        ("example_", "tooling.cargo-driver-strings"),
    ]),
    ("templates.rs", [
        ("channel_needs_tx", "tooling.cargo-driver-strings"),
        ("channel_needs_rx", "tooling.cargo-driver-strings"),
        ("channel_has_bundle", "tooling.cargo-driver-strings"),
        ("channel_bundle_index", "tooling.cargo-bundle-codegen"),
        ("channel_tx_prefix", "tooling.cargo-driver-strings"),
        ("channel_rx_prefix", "tooling.cargo-driver-strings"),
        ("channel_restructured", "tooling.cargo-bundle-codegen"),
        ("channel_should_build", "tooling.cargo-driver-strings"),
        ("actor_is_on_graph", "tooling.cargo-driver-strings"),
        ("actor_formal_name", "tooling.cargo-driver-strings"),
    ]),
]


def match_verify(name: str, rules: list[tuple[str, str]]) -> list[str]:
    out: list[str] = []
    for substr, vid in rules:
        if substr in name:
            if vid not in out:
                out.append(vid)
    return out


def annotate_file(path: Path, rules: list[tuple[str, str]]) -> int:
    text = path.read_text()
    pat = re.compile(
        r"(?m)(^[\t ]*// ss\[verify [^\]]+\]\n)*"
        r"(^[\t ]*)(#\[(?:test)\])\s*\n"
        r"([\t ]*(?:async\s+)?fn\s+(\w+))"
    )
    added = 0

    def repl(m: re.Match[str]) -> str:
        nonlocal added
        existing = m.group(1) or ""
        fn_name = m.group(5)
        vids = match_verify(fn_name, rules)
        if not vids:
            return m.group(0)
        indent = m.group(2)
        lines = []
        for vid in vids:
            tag = f"{indent}// ss[verify {vid}]\n"
            if tag not in existing and tag not in m.group(0):
                lines.append(tag)
        if not lines:
            return m.group(0)
        added += 1
        return existing + "".join(lines) + f"{indent}{m.group(3)}\n{m.group(4)}"

    new_text = pat.sub(repl, text)
    # Fix verify after #[test]
    new_text = re.sub(
        r"(#\[test\]\s*\n\s*)// ss\[verify ([^\]]+)\]\n(\s*fn )",
        r"// ss[verify \2]\n\1\3",
        new_text,
    )
    if new_text != text:
        path.write_text(new_text)
    return added


def main() -> None:
    total = 0
    for suffix, rules in RULES:
        path = CLI / suffix
        if path.exists():
            n = annotate_file(path, rules)
            if n:
                print(f"{path.relative_to(ROOT)}: +{n}")
                total += n
    print(f"Total: {total}")


if __name__ == "__main__":
    main()
