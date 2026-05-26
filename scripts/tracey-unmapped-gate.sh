#!/usr/bin/env bash
# ss[verify verify.process.tracey-gate]
# Fail if steady-state/rust-core mapped code-unit percentage is below threshold.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
THRESHOLD="${TRACEY_MAPPED_PERCENT:-80}"
cd "$ROOT"

if ! command -v tracey >/dev/null 2>&1; then
  echo "tracey CLI not installed; skipping unmapped gate"
  exit 0
fi

OUT="$(tracey query unmapped 2>/dev/null | head -1 || true)"
if [[ -z "$OUT" ]]; then
  echo "tracey query unmapped: no output"
  exit 1
fi

# Parse: "steady-state/rust-core: 2368 unmapped code units out of 2579 total"
if [[ "$OUT" =~ unmapped\ code\ units\ out\ of\ ([0-9]+)\ total ]]; then
  TOTAL="${BASH_REMATCH[1]}"
else
  echo "Could not parse tracey output: $OUT"
  exit 1
fi

if [[ "$OUT" =~ ^[^:]+:\ ([0-9]+)\ unmapped ]]; then
  UNMAPPED="${BASH_REMATCH[1]}"
else
  echo "Could not parse unmapped count: $OUT"
  exit 1
fi

MAPPED=$((TOTAL - UNMAPPED))
PERCENT=$((MAPPED * 100 / TOTAL))
echo "Tracey rust-core mapped: ${MAPPED}/${TOTAL} (${PERCENT}%), threshold ${THRESHOLD}%"

if [[ "$PERCENT" -lt "$THRESHOLD" ]]; then
  echo "FAIL: mapped ${PERCENT}% < ${THRESHOLD}%"
  exit 1
fi

echo "PASS: mapped code units meet threshold"
(cd "$ROOT" && tracey query validate) 2>/dev/null || true
