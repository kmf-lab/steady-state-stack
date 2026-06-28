#!/usr/bin/env bash
# Gate B: merged llvm-cov for release (lib + uri_contract; no live aeron_integration_suite).
# Optional Gate C merge when SS_AERON_GATE_C=1 (live driver + aeron_integration_suite).
# See CHANGELOG.md "Coverage (pre-release scope)".
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export CARGO_LLVM_COV=1

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov not installed. Install: cargo install cargo-llvm-cov" >&2
  exit 1
fi

FEATURES_A="exec_async_std,telemetry_server_builtin,core_affinity,core_display,prometheus_metrics"
FEATURES_B="proactor_nuclei,telemetry_server_cdn"

echo "llvm-cov pass A (${FEATURES_A}): lib + aeron_integration_uri_contract..."
echo "  Do not use 'cargo llvm-cov test --tests' — that runs live aeron_integration_suite (Gate C)."
cargo llvm-cov --lcov --output-path cov_a.lcov --no-default-features -F "${FEATURES_A}" \
  -p steady_state --lib --test aeron_integration_uri_contract

echo "llvm-cov pass B (${FEATURES_B}): lib..."
cargo llvm-cov --lcov --output-path cov_b.lcov --no-default-features -F "${FEATURES_B}" \
  -p steady_state --lib

MERGE_INPUTS=(cov_a.lcov cov_b.lcov)

if [[ "${SS_AERON_GATE_C:-0}" == "1" ]]; then
  echo "llvm-cov pass C (Gate C): lib + aeron_integration_suite (live media driver required)..."
  export SS_AERON_GATE_C=1
  export SS_AERON_LLVM_COV_ALLOW_TESTS=1
  cargo llvm-cov --lcov --output-path cov_c.lcov --no-default-features -F "${FEATURES_A}" \
    -p steady_state --lib --test aeron_integration_suite
  MERGE_INPUTS+=(cov_c.lcov)
else
  echo "Gate C llvm-cov pass skipped (set SS_AERON_GATE_C=1 to merge live Aeron suite coverage)."
fi

if command -v lcov >/dev/null 2>&1; then
  echo "Merging LCOV (${#MERGE_INPUTS[@]} inputs)..."
  lcov_args=()
  for f in "${MERGE_INPUTS[@]}"; do
    lcov_args+=(--add-tracefile "$f")
  done
  lcov "${lcov_args[@]}" -o merged.lcov
  if command -v genhtml >/dev/null 2>&1; then
    genhtml merged.lcov --output-directory coverage_html
    echo "HTML report: coverage_html/index.html"
  fi
else
  echo "lcov not installed; ${MERGE_INPUTS[*]} written (install lcov to merge)."
fi

if [[ "${SS_AERON_GATE_C:-0}" == "1" ]]; then
  echo "Gate B+C merge complete (live Aeron suite included via SS_AERON_GATE_C=1)."
else
  echo "Gate B complete (live Aeron suite excluded; use SS_AERON_GATE_C=1 or ./scripts/run-aeron-integration.sh for Gate C)."
fi
