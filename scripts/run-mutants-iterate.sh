#!/usr/bin/env bash
# ss[impl verify.process.mutants]
# Resumable cargo-mutants campaign: one file at a time, fail-fast on first survivor.
#
# Usage (from repository root):
#   bash scripts/run-mutants-iterate.sh -f dot_unify.rs
#   bash scripts/run-mutants-iterate.sh -f graph/liveliness.rs
#
# Optional environment:
#   SS_SKIP_MUTANTS=1              Exit 0 without running (pre-publish skip).
#   CARGO_MUTANTS_JOBS=1           Parallel mutant jobs (default 1).
#   NEXTEST_TEST_THREADS=1         Test threads per mutant (default 1).
#   CARGO_MUTANTS_TIMEOUT_SECS=600 Per-mutant cargo command cap (default 600; baseline nextest needs this).
#   CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT=30
#   SS_MUTANTS_POLL_SECS=2         How often to check missed.txt (default 2).
#   EXTRA_CARGO_MUTANTS_ARGS       Extra CLI tokens (quote carefully).
#
# Resume: copies docs/mutants/latest/{caught,missed,timeout,unviable}.txt into
# mutants.out/ when that dir has no caught list, then passes --iterate so
# previously killed mutants are skipped. mutants.out/ stays gitignored; the
# ledger under docs/mutants/latest/ is the committed copy (run snapshot after).
#
# Exit codes:
#   0  all examined mutants caught or timed out (missed.txt empty)
#   1  tool / baseline failure — do not iterate; fix tests first
#   2  fail-fast: a mutant survived (missed.txt non-empty)
#   3  SS_SKIP_MUTANTS is not 1 but -f was omitted and config has no globs

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Do not share CARGO_TARGET_DIR with other checkouts; cargo-mutants holds the
# artifact lock for a long time.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"

if [[ "${SS_SKIP_MUTANTS:-0}" == "1" ]]; then
  echo "SS_SKIP_MUTANTS=1 — skipping cargo-mutants campaign."
  exit 0
fi

FILE_GLOB=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    -f|--file)
      FILE_GLOB="${2:-}"
      shift 2
      ;;
    -h|--help)
      sed -n '2,24p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1 (try -f <glob>)" >&2
      exit 1
      ;;
  esac
done

if ! command -v cargo-mutants &>/dev/null; then
  echo "cargo-mutants not found, installing..."
  cargo install cargo-mutants --locked
fi

MUTANTS_CONFIG="${REPO_ROOT}/core/mutants.toml"
LOG="${REPO_ROOT}/cargo_mutants.txt"
LEDGER="${REPO_ROOT}/docs/mutants/latest"
OUT_DIR="${REPO_ROOT}/mutants.out"

: "${CARGO_MUTANTS_JOBS:=1}"
: "${NEXTEST_TEST_THREADS:=1}"
: "${CARGO_MUTANTS_TIMEOUT_SECS:=240}"
: "${CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT:=30}"
: "${SS_MUTANTS_POLL_SECS:=2}"
export NEXTEST_TEST_THREADS
export CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT

# Stale June 2026 run (0.2.13 baseline failure, 0 mutants scored) must not drive --iterate.
if [[ -f "${OUT_DIR}/outcomes.json" ]]; then
  if python3 - "${OUT_DIR}/outcomes.json" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
text = open(sys.argv[1], errors="replace").read()
stale = d.get("total_mutants", 0) == 0 or "steady_state@0.2.13" in text
sys.exit(0 if stale else 1)
PY
  then
    stale_dest="${REPO_ROOT}/mutants.out.stale-$(date +%Y%m%d%H%M%S)"
    echo "Archiving stale mutants.out (baseline-failure / 0.2.13) to ${stale_dest}"
    mv "${OUT_DIR}" "${stale_dest}"
  fi
fi

mkdir -p "${OUT_DIR}"

# Restore committed ledger so --iterate survives a wiped mutants.out.
# Do not restore missed.txt into the live watch file — those mutants must be retried.
if [[ ! -s "${OUT_DIR}/caught.txt" && -s "${LEDGER}/caught.txt" ]]; then
  echo "Restoring mutants.out from ${LEDGER} (caught/timeout/unviable; missed will be retried)"
  for f in caught.txt timeout.txt unviable.txt outcomes.json; do
    if [[ -f "${LEDGER}/${f}" ]]; then
      cp "${LEDGER}/${f}" "${OUT_DIR}/${f}"
    fi
  done
  if [[ -s "${LEDGER}/missed.txt" ]]; then
    cp "${LEDGER}/missed.txt" "${OUT_DIR}/missed.prev.txt"
    echo "Previous survivors (will retest):"
    cat "${OUT_DIR}/missed.prev.txt"
  fi
fi

mut_args=(
  --manifest-path core/Cargo.toml
  --config "${MUTANTS_CONFIG}"
  --test-tool nextest
  --cap-lints true
  --iterate
  --output "${REPO_ROOT}"
  --minimum-test-timeout "${CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT}"
  -t "${CARGO_MUTANTS_TIMEOUT_SECS}"
  -j "${CARGO_MUTANTS_JOBS}"
)

if [[ -n "${FILE_GLOB}" ]]; then
  # Config examine_globs UNIONS with --file. Isolate one file via --no-config.
  if [[ "${FILE_GLOB}" != *'*'* ]]; then
    FILE_GLOB="**/${FILE_GLOB#/}"
  fi
  mut_args=(
    --manifest-path core/Cargo.toml
    --no-config
    --test-tool nextest
    --test-workspace false
    --cap-lints true
    --iterate
    --output "${REPO_ROOT}"
    --minimum-test-timeout "${CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT}"
    -t "${CARGO_MUTANTS_TIMEOUT_SECS}"
    -j "${CARGO_MUTANTS_JOBS}"
    --file "${FILE_GLOB}"
  )
  echo "cargo-mutants iterate: file=${FILE_GLOB} (--no-config so examine_globs cannot union)"
else
  echo "cargo-mutants iterate: examine_globs from core/mutants.toml"
fi

echo "jobs=${CARGO_MUTANTS_JOBS}  min_timeout=${CARGO_MUTANTS_MINIMUM_TEST_TIMEOUT}s  cap=${CARGO_MUTANTS_TIMEOUT_SECS}s"
echo "nextest profile=mutants  --iterate (skip previously caught)"

# Snapshot list files BEFORE cargo-mutants truncates them.
HISTORY_CAUGHT="${OUT_DIR}/caught.history.txt"
HISTORY_UNVIABLE="${OUT_DIR}/unviable.history.txt"
HISTORY_TIMEOUT="${OUT_DIR}/timeout.history.txt"
if [[ -s "${OUT_DIR}/caught.txt" ]]; then
  cp "${OUT_DIR}/caught.txt" "${HISTORY_CAUGHT}"
fi
if [[ -s "${OUT_DIR}/unviable.txt" ]]; then
  cp "${OUT_DIR}/unviable.txt" "${HISTORY_UNVIABLE}"
fi
if [[ -s "${OUT_DIR}/timeout.txt" ]]; then
  cp "${OUT_DIR}/timeout.txt" "${HISTORY_TIMEOUT}"
fi
if [[ -s "${LEDGER}/caught.txt" && ! -s "${HISTORY_CAUGHT}" ]]; then
  cp "${LEDGER}/caught.txt" "${HISTORY_CAUGHT}"
fi
if [[ -s "${LEDGER}/unviable.txt" && ! -s "${HISTORY_UNVIABLE}" ]]; then
  cp "${LEDGER}/unviable.txt" "${HISTORY_UNVIABLE}"
fi

merge_history() {
  local hist="$1"
  local live="$2"
  if [[ -s "${hist}" ]]; then
    touch "${live}"
    sort -u "${hist}" "${live}" | grep -v '^[[:space:]]*$' > "${live}.merged" || true
    mv "${live}.merged" "${live}"
  fi
}

preserve_lists() {
  # cargo-mutants wipes mutants.out/; keep history in the committed ledger.
  merge_history "${LEDGER}/caught.txt" "${OUT_DIR}/caught.txt"
  merge_history "${LEDGER}/unviable.txt" "${OUT_DIR}/unviable.txt"
  merge_history "${LEDGER}/timeout.txt" "${OUT_DIR}/timeout.txt"
}

trap preserve_lists EXIT

# Fail-fast watches for a *new* write to missed.txt this run. Previous survivors
# are retried via --iterate (they are not in caught.txt).
if [[ -s "${OUT_DIR}/missed.txt" ]]; then
  echo "Retrying previously missed mutants:"
  cat "${OUT_DIR}/missed.txt"
  cp "${OUT_DIR}/missed.txt" "${OUT_DIR}/missed.prev.txt"
  : > "${OUT_DIR}/missed.txt"
fi

: > "${LOG}"
set +e
# shellcheck disable=SC2086
cargo mutants "${mut_args[@]}" ${EXTRA_CARGO_MUTANTS_ARGS:-} -- --profile mutants --test-threads "${NEXTEST_TEST_THREADS}" >>"${LOG}" 2>&1 &
mut_pid=$!
set -e

echo "cargo-mutants pid=${mut_pid}  log=${LOG}"

count_lines() {
  local path="$1"
  if [[ -f "${path}" ]]; then
    grep -cve '^[[:space:]]*$' "${path}" || true
  else
    echo 0
  fi
}

kill_mutants_tree() {
  local pid="$1"
  if [[ -z "${pid}" ]]; then
    return 0
  fi
  pkill -TERM -P "${pid}" 2>/dev/null || true
  kill -TERM "${pid}" 2>/dev/null || true
  sleep 1
  pkill -KILL -P "${pid}" 2>/dev/null || true
  kill -KILL "${pid}" 2>/dev/null || true
}

baseline_failed() {
  grep -qE 'baseline .*fail|Baseline.*Failure|tests failed in an unmutated tree' "${LOG}" 2>/dev/null
}

last_report=0
while kill -0 "${mut_pid}" 2>/dev/null; do
  now="$(date +%s)"
  if (( now - last_report >= 30 )); then
    echo "progress caught=$(count_lines "${OUT_DIR}/caught.txt") missed=$(count_lines "${OUT_DIR}/missed.txt") timeout=$(count_lines "${OUT_DIR}/timeout.txt") unviable=$(count_lines "${OUT_DIR}/unviable.txt")"
    last_report="${now}"
  fi
  if [[ -s "${OUT_DIR}/missed.txt" ]]; then
    echo "FAIL-FAST: mutant survived (missed.txt non-empty). Stopping cargo-mutants." >&2
    echo "---- missed.txt ----" >&2
    cat "${OUT_DIR}/missed.txt" >&2
    kill_mutants_tree "${mut_pid}"
    wait "${mut_pid}" 2>/dev/null || true
    exit 2
  fi
  if baseline_failed; then
    echo "ERROR: unmutated baseline tests failed — kill campaign, fix tests, do not --iterate." >&2
    tail -n 80 "${LOG}" >&2
    kill_mutants_tree "${mut_pid}"
    wait "${mut_pid}" 2>/dev/null || true
    exit 1
  fi
  sleep "${SS_MUTANTS_POLL_SECS}"
done

set +e
wait "${mut_pid}"
exit_code=$?
set -e

if [[ -s "${OUT_DIR}/missed.txt" ]]; then
  echo "Mutant(s) survived:" >&2
  cat "${OUT_DIR}/missed.txt" >&2
  exit 2
fi

# cargo-mutants exits 3 when some mutants TIMED OUT. Timeouts count as killed.
if [[ "${exit_code}" -eq 0 || "${exit_code}" -eq 3 ]]; then
  echo "cargo-mutants iterate complete: missed.txt empty (timeouts count as killed)."
  exit 0
fi

echo "cargo-mutants exited ${exit_code} (see ${LOG})" >&2
tail -n 80 "${LOG}" >&2
exit 1
