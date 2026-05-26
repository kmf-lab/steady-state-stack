#!/usr/bin/env python3
"""Insert ss[verify ...] before #[test] when missing."""
from __future__ import annotations

import re
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None  # type: ignore

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "core" / "src"
CARGO_CLI = ROOT / "cargo-steady-state" / "src"
YAML_PATH = Path(__file__).resolve().parent / "tracey-file-requirements.yaml"

# Legacy inline rules (merged with YAML test_rules)
RULES: list[tuple[str, list[tuple[str, str]]]] = [
    ("graph_liveliness.rs", [
        ("test_unclean_shutdown_veto", "graph.shutdown.veto"),
        ("test_clean_shutdown", "graph.shutdown.accept"),
        ("is_running_accept_shutdown", "graph.shutdown.accept"),
        ("internal_request_shutdown", "graph.request-shutdown"),
        ("register_voter", "graph.liveliness-voters"),
        ("remove_voter", "graph.liveliness-voters"),
        ("vote_for_the_dead", "graph.liveliness-voters"),
        ("check_is_stopped_clean", "graph.block-until-stopped"),
        ("check_is_stopped_unclean", "graph.shutdown.veto"),
        ("actor_identity", "graph.actor-identity"),
        ("start_with_timeout", "graph.for-testing"),
        ("for_testing", "graph.for-testing"),
        ("aeron_init_timeouts", "distributed.media-driver-testing"),
        ("wait_for_registrations", "graph.liveliness-voters"),
        ("actor_by_id", "graph.actor-identity"),
        ("is_shutdown_telemetry", "telemetry.shutdown-complete"),
    ]),
    ("channel_builder.rs", [
        ("memory_usage", "channel.memory-usage-telemetry"),
        ("bundle_index", "bundle.girth-const-generic"),
        ("stream_builder_bundle", "channel.stream-dual-buffer"),
        ("filled_", "channel.backpressure-never-drop"),
        ("rate_per_", "channel.default-capacity"),
        ("to_meta_data", "channel.lazy.defer-allocation"),
        ("channel_builder", "channel.lazy.defer-allocation"),
    ]),
    ("channel_builder_lazy.rs", [
        ("lazy_flow", "channel.lazy.establish-on-clone"),
        ("lazy_channel_initialization", "channel.lazy.defer-allocation"),
        ("testing_send_all", "channel.testing-send-all"),
        ("testing_take_all", "channel.testing-take-all"),
        ("lazy_tx_bundle", "bundle.clone-establishes"),
        ("lazy_rx_bundle", "bundle.clone-establishes"),
    ]),
    ("steady_rx.rs", [
        ("peek", "philosophy.zero-copy-discipline"),
        ("take", "philosophy.zero-copy-discipline"),
        ("try_take", "philosophy.zero-copy-discipline"),
        ("wait_avail", "actor.wait-avail-vacant"),
        ("bundle", "bundle.index-wait-readiness"),
        ("closed", "channel.backpressure-never-drop"),
        ("empty", "channel.backpressure-never-drop"),
        ("test_bundle", "bundle.girth-const-generic"),
    ]),
    ("steady_tx.rs", [
        ("lazy_flow", "channel.backpressure-never-drop"),
        ("send_slice_until_full", "channel.backpressure-never-drop"),
        ("vacant", "channel.backpressure-never-drop"),
        ("wait_vacant", "actor.wait-avail-vacant"),
        ("bundle", "bundle.index-wait-readiness"),
        ("mark_closed", "actor.shutdown-veto"),
        ("is_full", "channel.backpressure-never-drop"),
    ]),
    ("core_rx.rs", [
        ("peek", "philosophy.zero-copy-discipline"),
        ("take", "philosophy.zero-copy-discipline"),
        ("avail", "actor.wait-avail-vacant"),
        ("closed", "channel.backpressure-never-drop"),
    ]),
    ("core_tx.rs", [
        ("vacant", "channel.backpressure-never-drop"),
        ("send", "channel.backpressure-never-drop"),
        ("full", "channel.backpressure-never-drop"),
    ]),
    ("steady_actor_shadow.rs", [
        ("wait_avail_index", "actor.index-wait-truthful"),
        ("wait_vacant_index", "actor.index-wait-truthful"),
        ("wait_avail_vacant", "actor.index-wait-paired"),
        ("index_wait", "actor.index-wait-round-robin"),
        ("repeat", "actor.index-wait-repeat-bypass"),
        ("shutdown", "bundle.index-wait-shutdown-none"),
        ("next_index_wait", "actor.index-wait-round-robin"),
        ("avoid_repeat", "actor.index-wait-repeat-bypass"),
        ("paired", "actor.index-wait-paired"),
        ("uniform", "bundle.uniform-counts-helper"),
        ("is_full_and_vacant", "actor.wait-avail-vacant"),
    ]),
    ("actor_builder.rs", [
        ("regeneration", "actor.regeneration-survives"),
        ("panic", "actor.regeneration-survives"),
        ("restart", "actor.regeneration-survives"),
        ("explicit_core", "actor.regeneration-survives"),
        ("troupe", "graph.troupes"),
        ("never_simulate", "testing.never-run-in-unit"),
    ]),
    ("graph_testing.rs", [
        ("stage", "testing.stage-manager-integration"),
        ("simulated", "testing.stage-manager-integration"),
        ("pipeline", "testing.pipeline-worker-allowlist"),
        ("assert", "testing.assert-steady-rx"),
        ("greeter", "testing.internal-behavior-direct"),
        ("worker", "testing.pipeline-worker-allowlist"),
        ("graph_test", "testing.graph-for-testing"),
        ("stack_guarded", "testing.stage-manager-integration"),
        ("avail", "testing.deterministic-no-sleep"),
    ]),
    ("macros.rs", [
        ("split_bundle", "bundle.split-macro"),
        ("wait_for_index", "bundle.wait-for-index-macro"),
    ]),
    ("dot_unify.rs", [("", "telemetry.dot-export")]),
    ("channel_stats_tests.rs", [("", "telemetry.channel-labels")]),
    ("loop_driver.rs", [
        ("await_for_all", "philosophy.single-wake-up"),
        ("await_for_any", "philosophy.single-wake-up"),
        ("steady_await", "philosophy.single-wake-up"),
        ("steady_select", "philosophy.single-wake-up"),
        ("wait_for_all", "philosophy.single-wake-up"),
    ]),
    ("state_management.rs", [
        ("basic_state", "state.lock-init-once"),
        ("cloning_shared", "state.clone-shared"),
        ("persistent_state_load", "state.persistent-load"),
        ("persistent_state_save", "state.save-on-drop"),
        ("persistent_state_no_file", "state.persistent-load"),
        ("persistent_state_invalid", "state.persistent-load"),
    ]),
    ("simulate_edge.rs", [
        ("close_outputs", "testing.sim-producer-close"),
        ("simulate_single", "testing.stage-manager-integration"),
    ]),
    ("steady_actor.rs", [
        ("next_index_wait", "actor.index-wait-round-robin"),
        ("index_wait_avoid", "actor.index-wait-repeat-bypass"),
        ("index_wait_counts", "bundle.uniform-counts-helper"),
        ("wait_avail_bundle", "bundle.deprecated-bundle-waits"),
    ]),
    ("monitor.rs", [
        ("wait_for_index", "bundle.wait-for-index-macro"),
        ("wait_avail_bundle", "bundle.deprecated-bundle-waits"),
        ("wait_avail_index", "actor.index-wait-round-robin"),
        ("wait_vacant", "actor.wait-avail-vacant"),
        ("index_wait", "bundle.index-wait-readiness"),
    ]),
    ("main.rs", [
        ("extract_percent", "tooling.cargo-percent-parse"),
        ("driver_block", "tooling.cargo-driver-strings"),
        ("wait_avail_bundle", "tooling.cargo-driver-strings"),
        ("wait_vacant_bundle", "tooling.cargo-driver-strings"),
        ("capacity_driven", "tooling.cargo-capacity-driven"),
        ("bundle_codegen", "tooling.cargo-bundle-codegen"),
    ]),
]


def load_yaml_rules() -> dict[str, list[tuple[str, str]]]:
    if yaml is None or not YAML_PATH.exists():
        return {}
    data = yaml.safe_load(YAML_PATH.read_text()) or {}
    out: dict[str, list[tuple[str, str]]] = {}
    for rel, cfg in (data.get("files") or {}).items():
        rules: list[tuple[str, str]] = []
        default = cfg.get("test_default_verify")
        if default:
            rules.append(("", default))
        for tr in cfg.get("test_rules") or []:
            rules.append((tr.get("substr", ""), tr["id"]))
        if rules:
            out[rel] = rules
    for rel, cfg in (data.get("cargo_steady_state") or {}).items():
        rules = []
        default = cfg.get("test_default_verify")
        if default:
            rules.append(("", default))
        for tr in cfg.get("test_rules") or []:
            rules.append((tr.get("substr", ""), tr["id"]))
        if rules:
            out[rel] = rules
    return out


def merged_rules(suffix: str) -> list[tuple[str, str]]:
    rules: list[tuple[str, str]] = []
    yaml_rules = load_yaml_rules()
    if suffix in yaml_rules:
        rules.extend(yaml_rules[suffix])
    for s, r in RULES:
        if s == suffix:
            rules.extend(r)
    # dedupe preserving order
    seen: set[tuple[str, str]] = set()
    unique: list[tuple[str, str]] = []
    for item in rules:
        if item not in seen:
            seen.add(item)
            unique.append(item)
    return unique


def match_verify(fn_name: str, rules: list[tuple[str, str]]) -> str | None:
    for substr, vid in rules:
        if substr == "" or substr in fn_name:
            return vid
    return None


def annotate_file(path: Path, rules: list[tuple[str, str]]) -> int:
    if not rules:
        return 0
    text = path.read_text()
    pat = re.compile(
        r"(?m)(^[\t ]*// ss\[verify [^\]]+\]\n)?"
        r"(^[\t ]*)(#\[(?:test|async_std::test)\])\s*\n"
        r"([\t ]*(?:async\s+)?fn\s+(\w+))"
    )
    added = 0

    def repl(m: re.Match[str]) -> str:
        nonlocal added
        if m.group(1):
            return m.group(0)
        fn_name = m.group(5)
        vid = match_verify(fn_name, rules)
        if not vid:
            return m.group(0)
        added += 1
        indent = m.group(2)
        return f"{indent}// ss[verify {vid}]\n{indent}{m.group(3)}\n{m.group(4)}"

    new_text = pat.sub(repl, text)
    if added:
        path.write_text(new_text)
    return added


def main() -> None:
    total = 0
    suffixes: set[str] = {s for s, _ in RULES}
    suffixes.update(load_yaml_rules().keys())
    for suffix in sorted(suffixes):
        rules = merged_rules(suffix)
        for base in (CORE, CARGO_CLI):
            path = base / suffix
            if path.exists():
                n = annotate_file(path, rules)
                if n:
                    print(f"{path.relative_to(ROOT)}: +{n}")
                    total += n
    print(f"Total: {total}")


if __name__ == "__main__":
    main()
