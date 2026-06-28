#!/usr/bin/env bash
# Fail when any core/src Rust source file exceeds hard cap; warn above soft cap.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOFT="${FILE_SIZE_SOFT:-1200}"
HARD="${FILE_SIZE_HARD:-1800}"
cd "$ROOT"

fail=0
warn=0
while IFS= read -r -d '' file; do
  lines=$(wc -l <"$file")
  if (( lines > HARD )); then
    echo "ERROR: $file has $lines lines (hard cap $HARD)" >&2
    fail=1
  elif (( lines > SOFT )); then
    echo "WARN: $file has $lines lines (soft target $SOFT)" >&2
    warn=1
  fi
done < <(find core/src -name '*.rs' -print0)

if (( fail )); then
  exit 1
fi
if (( warn )); then
  echo "file-size check: soft warnings only ($SOFT lines)" >&2
fi
echo "file-size check: ok (soft=$SOFT hard=$HARD)"
