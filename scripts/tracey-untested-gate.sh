#!/usr/bin/env bash
# ss[verify verify.process.tracey-gate]
# Fail if Tier-2 distributed requirements have impl but no verify reference.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${TRACEY_UNTESTED_PREFIX:-distributed}"
cd "$ROOT"

if ! command -v tracey >/dev/null 2>&1; then
  echo "tracey CLI not installed; skipping untested gate"
  exit 0
fi

check_prefix() {
  local p="$1"
  local out
  out="$(tracey query untested --prefix "$p" 2>/dev/null || true)"
  echo "$out"
  if echo "$out" | grep -qE '([1-9][0-9]*|[1-9]) untested'; then
    echo "FAIL: untested requirements remain for prefix ${p}" >&2
    return 1
  fi
  if echo "$out" | grep -q '0 untested'; then
    echo "PASS: ${p} — 0 untested"
    return 0
  fi
  echo "WARN: could not parse untested output for ${p}: ${out}" >&2
  return 1
}

failed=0
check_prefix "$PREFIX" || failed=1
check_prefix "stream.control-payload" || failed=1

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi

echo "All configured untested-prefix checks passed."
