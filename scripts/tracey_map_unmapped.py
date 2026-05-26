#!/usr/bin/env python3
"""Insert ss[impl|related|verify] on Rust items missing Tracey annotations."""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None  # type: ignore

ROOT = Path(__file__).resolve().parents[1]
CORE = ROOT / "core" / "src"
CARGO_CLI = ROOT / "cargo-steady-state" / "src"
YAML_PATH = Path(__file__).resolve().parent / "tracey-file-requirements.yaml"

# Items Tracey typically counts as code units.
ITEM_RE = re.compile(
    r"^(\s*)("
    r"(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?struct\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?enum\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?trait\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?type\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?const\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?static\s+\w+"
    r"|(?:pub(?:\([^)]*\))?\s+)?use\s+"
    r"|impl(?:<[^>]+>)?\s+"
    r"|mod\s+\w+"
    r"|macro_rules!\s+\w+"
    r")\b"
)

SS_RE = re.compile(r"^\s*//\s*ss\[(?:impl|verify|depends|related)\s+")
TEST_ATTR_RE = re.compile(r"^\s*#\[(?:test|async_std::test)\]")


def load_config() -> dict:
    if yaml is None:
        print("PyYAML required: pip install pyyaml", file=sys.stderr)
        sys.exit(1)
    data = yaml.safe_load(YAML_PATH.read_text())
    return data or {}


def has_ss_nearby(lines: list[str], idx: int, lookback: int = 3) -> bool:
    for j in range(max(0, idx - lookback), idx):
        if SS_RE.match(lines[j]):
            return True
    return False


def pick_requirement(
    name: str,
    file_cfg: dict,
    is_test: bool,
) -> tuple[str, str]:
    """Return (verb, requirement_id)."""
    if is_test:
        rules = file_cfg.get("test_rules") or []
        for rule in rules:
            substr = rule.get("substr", "")
            if substr == "" or substr in name:
                return "verify", rule["id"]
        if file_cfg.get("test_default_verify"):
            return "verify", file_cfg["test_default_verify"]
        primary = file_cfg.get("primary")
        if primary:
            return "verify", primary
        return "verify", "philosophy.structural-hierarchy"

    patterns = file_cfg.get("impl_patterns") or []
    for pat in patterns:
        substr = pat.get("substr", "")
        if substr and substr in name:
            return pat.get("verb", "impl"), pat["id"]

    verb = file_cfg.get("verb", "related")
    primary = file_cfg.get("primary")
    if primary:
        return verb, primary
    return "related", "philosophy.structural-hierarchy"


def extract_name(line: str) -> str:
    m = re.search(r"\bfn\s+(\w+)", line)
    if m:
        return m.group(1)
    for kw in ("struct", "enum", "trait", "type", "mod"):
        m = re.search(rf"\b{kw}\s+(\w+)", line)
        if m:
            return m.group(1)
    if "impl" in line:
        return "impl"
    return ""


def annotate_file(path: Path, file_cfg: dict, dry_run: bool) -> int:
    text = path.read_text()
    lines = text.splitlines(keepends=True)
    new_lines: list[str] = []
    added = 0
    i = 0
    in_test_mod = False

    while i < len(lines):
        line = lines[i]
        stripped = line.rstrip("\n")

        if re.match(r"^\s*#\[cfg\(test\)\]", stripped):
            in_test_mod = True
        if re.match(r"^\s*mod\s+tests\s*\{", stripped):
            in_test_mod = True

        m = ITEM_RE.match(stripped)
        if m and not has_ss_nearby(new_lines, len(new_lines)):
            indent, _ = m.group(1), m.group(2)
            name = extract_name(stripped)
            # Detect test: next lines may have #[test] above fn — check lookback in source
            is_test_fn = False
            if "fn " in stripped:
                for j in range(max(0, i - 4), i):
                    if TEST_ATTR_RE.match(lines[j].rstrip("\n")):
                        is_test_fn = True
                        break
                if in_test_mod and not name.startswith("test_"):
                    # Still annotate tests in mod tests
                    pass
            verb, req_id = pick_requirement(name, file_cfg, is_test_fn)
            ann = f"{indent}// ss[{verb} {req_id}]\n"
            new_lines.append(ann)
            added += 1

        new_lines.append(line)
        i += 1

    if added and not dry_run:
        path.write_text("".join(new_lines))
    return added


def resolve_files(files_cfg: dict, only: str | None) -> list[Path]:
    paths: list[Path] = []
    if only:
        p = CORE / only
        if p.exists():
            return [p]
        p = CORE.parent / only
        if p.exists():
            return [p]
        print(f"Not found: {only}", file=sys.stderr)
        return []

    if files_cfg:
        for rel in sorted(files_cfg):
            p = CORE / rel
            if p.exists():
                paths.append(p)
    else:
        paths = sorted(CORE.rglob("*.rs"))
    return paths


def main() -> None:
    parser = argparse.ArgumentParser(description="Map unmapped Rust items to Tracey requirements")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--file", help="Single file relative to core/src (e.g. monitor.rs)")
    parser.add_argument("--all-core", action="store_true", help="Process every core/src/**/*.rs")
    parser.add_argument("--cargo-cli", action="store_true", help="Process cargo-steady-state/src")
    args = parser.parse_args()

    cfg = load_config()
    files_cfg = cfg.get("files") or {}
    cargo_cfg = cfg.get("cargo_steady_state") or {}
    defaults = cfg.get("defaults") or {}

    if args.cargo_cli:
        targets = sorted(CARGO_CLI.rglob("*.rs"))
        for path in targets:
            rel = path.relative_to(CARGO_CLI).as_posix()
            file_cfg = {**defaults, **cargo_cfg.get(rel, {"primary": "tooling.cargo-driver-strings"})}
            if not file_cfg.get("primary"):
                file_cfg["primary"] = "tooling.cargo-driver-strings"
            n = annotate_file(path, file_cfg, args.dry_run)
            if n:
                print(f"cargo-steady-state/src/{rel}: +{n}")
        print("Done cargo-cli")
        return

    if args.all_core:
        targets = sorted(CORE.rglob("*.rs"))
        build_rs = CORE.parent / "build.rs"
        if build_rs.exists():
            targets.append(build_rs)
        file_map = files_cfg
    else:
        targets = resolve_files(files_cfg, args.file)

    total = 0
    for path in targets:
        if path.name == "build.rs":
            rel = "build.rs"
            file_cfg = {
                **defaults,
                "primary": "platform.ringbuf-pin",
                "verb": "impl",
            }
        else:
            rel = path.relative_to(CORE).as_posix()
            file_cfg = {**defaults, **files_cfg.get(rel, {})}
        if not file_cfg.get("primary") and not args.all_core and rel not in files_cfg:
            if args.file:
                file_cfg = {**defaults, "primary": "philosophy.structural-hierarchy"}
            else:
                continue
        if not file_cfg.get("primary"):
            file_cfg["primary"] = "philosophy.structural-hierarchy"
        n = annotate_file(path, file_cfg, args.dry_run)
        if n:
            print(f"{rel}: +{n}")
            total += n
    print(f"Total annotations: {total}")


if __name__ == "__main__":
    main()
