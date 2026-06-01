#!/usr/bin/env bash
# Run release sign-off three times; fail if any run fails (flake gate before tagging).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUNS="${SS_AERON_FLAKE_RUNS:-3}"
SIGNOFF="${ROOT}/scripts/run-aeron-release-signoff.sh"

for run in $(seq 1 "${RUNS}"); do
  echo "=== Aeron flake check run ${run}/${RUNS} ==="
  if [[ "${run}" -eq 1 ]]; then
    export SS_AERON_FRESH_DRIVER="${SS_AERON_FRESH_DRIVER:-1}"
  else
    export SS_AERON_FRESH_DRIVER=0
  fi
  export SS_AERON_LOG_FILE="/tmp/aeron-flake-check-${run}.log"
  if ! bash "${SIGNOFF}"; then
    echo "ERROR: flake check failed on run ${run}/${RUNS} (log: ${SS_AERON_LOG_FILE})" >&2
    exit 1
  fi
  passes="$(grep -c '^PASS \[' "${SS_AERON_LOG_FILE}" 2>/dev/null || true)"
  echo "Run ${run}/${RUNS} ok (${passes} PASS [scenario] lines)"
done

echo "PASS: ${RUNS}/${RUNS} release sign-off runs succeeded"
