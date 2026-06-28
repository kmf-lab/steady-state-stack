#!/usr/bin/env bash
# ss[verify verify.process.llvm-cov]
# Gate B coverage enforcement: full merged LCOV must meet region threshold (default 98%).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MIN_REGION="${SS_COVERAGE_MIN_REGION:-98.0}"
MIN_LINE="${SS_COVERAGE_MIN_LINE:-98.0}"
LCOV="${SS_COVERAGE_LCOV:-merged.lcov}"

if [[ ! -f "$LCOV" ]]; then
  echo "Running llvm-cov release merge (no $LCOV)..." >&2
  bash scripts/run-llvm-cov-release.sh
fi

if [[ ! -f "$LCOV" ]]; then
  echo "ERROR: $LCOV not found after run-llvm-cov-release.sh" >&2
  exit 1
fi

if ! command -v lcov >/dev/null 2>&1; then
  echo "ERROR: lcov required for coverage gate (install lcov package)" >&2
  exit 1
fi

summary="$(lcov --summary "$LCOV" 2>&1)"
echo "$summary"

region_pct="$(echo "$summary" | awk '/lines\.\.\.:/{print $2}' | tr -d '%')"
line_pct="$(echo "$summary" | awk '/lines\.\.\.:/{print $2}' | tr -d '%')"

# lcov --summary reports lines; use python for robust region parse when available
if command -v python3 >/dev/null 2>&1; then
  read -r region_pct line_pct <<<"$(python3 - "$LCOV" <<'PY'
import re, sys
text = open(sys.argv[1], errors="replace").read()
regions = re.search(r"regions\.\.\.: (\d+) / (\d+)", text)
lines = re.search(r"lines\.\.\.\.: (\d+) / (\d+)", text)
rp = 100.0 * int(regions.group(1)) / int(regions.group(2)) if regions else 0.0
lp = 100.0 * int(lines.group(1)) / int(lines.group(2)) if lines else 0.0
print(f"{rp:.2f} {lp:.2f}")
PY
)"
fi

echo "Region coverage: ${region_pct}% (min ${MIN_REGION}%)"
echo "Line coverage:   ${line_pct}% (min ${MIN_LINE}%)"

fail=0
python3 - "$region_pct" "$MIN_REGION" <<'PY' || fail=1
import sys
r, m = float(sys.argv[1]), float(sys.argv[2])
if r + 1e-9 < m:
    print(f"FAIL: region {r:.2f}% < {m}%", file=sys.stderr)
    sys.exit(1)
print(f"PASS: region {r:.2f}% >= {m}%")
PY

python3 - "$line_pct" "$MIN_LINE" <<'PY' || fail=1
import sys
l, m = float(sys.argv[1]), float(sys.argv[2])
if l + 1e-9 < m:
    print(f"FAIL: line {l:.2f}% < {m}%", file=sys.stderr)
    sys.exit(1)
print(f"PASS: line {l:.2f}% >= {m}%")
PY

if [[ "$fail" -ne 0 ]]; then
  echo "Per-file offenders (lowest line %):" >&2
  lcov --list "$LCOV" 2>/dev/null | tail -n +2 | sort -t'|' -k2 -n | head -20 >&2 || true
  exit 1
fi

echo "Coverage gate passed."
