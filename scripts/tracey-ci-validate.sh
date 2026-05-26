#!/usr/bin/env bash
# ss[verify verify.process.nextest]
# ss[verify verify.process.tracey-gate]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
if command -v tracey >/dev/null 2>&1; then
  (cd "$ROOT" && tracey query validate)
else
  echo "tracey CLI not installed; skipping validate"
  exit 0
fi
