#!/usr/bin/env bash
# Bounded cargo-fuzz campaign for parse/protocol targets (V2V Fuzz ≥ 6).
#
# Usage (from repository root):
#   bash scripts/run-fuzz.sh
#
# Optional environment:
#   SS_SKIP_FUZZ=1          Exit 0 without running (no nightly/clang, or release skip).
#   SS_FUZZ_SECONDS=20      libFuzzer -max_total_time per target (default 20).
#   SS_FUZZ_TARGETS         Space-separated target list (default: all four).
#
# Requires nightly Rust and clang (libFuzzer). Does not run on GitHub PRs.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ "${SS_SKIP_FUZZ:-0}" == "1" ]]; then
  echo "SS_SKIP_FUZZ=1 — skipping cargo-fuzz campaign."
  exit 0
fi

: "${SS_FUZZ_SECONDS:=20}"
: "${SS_FUZZ_TARGETS:=aeron_channel_uri fast_protocol_packed cgroup_quota pack_even_split}"

if ! command -v rustup >/dev/null 2>&1; then
  echo "ERROR: rustup is required to run cargo +nightly fuzz." >&2
  echo "Set SS_SKIP_FUZZ=1 to skip this step." >&2
  exit 1
fi

if ! rustup toolchain list | grep -q '^nightly'; then
  echo "ERROR: nightly Rust toolchain is not installed (needed for cargo-fuzz / libFuzzer)." >&2
  echo "Install: rustup toolchain install nightly" >&2
  echo "Set SS_SKIP_FUZZ=1 to skip this step." >&2
  exit 1
fi

if ! command -v clang >/dev/null 2>&1 && ! command -v clang++ >/dev/null 2>&1; then
  echo "ERROR: clang is required to link libFuzzer." >&2
  echo "Install clang, or set SS_SKIP_FUZZ=1 to skip this step." >&2
  exit 1
fi

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "cargo-fuzz not found, installing..."
  cargo install cargo-fuzz --locked
fi

if [[ ! -f "${REPO_ROOT}/core/fuzz/Cargo.toml" ]]; then
  echo "ERROR: core/fuzz/Cargo.toml missing — cargo-fuzz crate not initialized." >&2
  exit 1
fi

echo "cargo-fuzz campaign: ${SS_FUZZ_SECONDS}s per target"
echo "targets: ${SS_FUZZ_TARGETS}"

cd "${REPO_ROOT}/core"
fail=0
for target in ${SS_FUZZ_TARGETS}; do
  echo "---------------------------------------------------------------------------------"
  echo "fuzz target: ${target}  (-max_total_time=${SS_FUZZ_SECONDS})"
  start_ns="$(date +%s%N)"
  if cargo +nightly fuzz run "${target}" -- -max_total_time="${SS_FUZZ_SECONDS}" -verbosity=0; then
    end_ns="$(date +%s%N)"
    elapsed_s="$(( (end_ns - start_ns) / 1000000000 ))"
    echo "PASS ${target}  (~${elapsed_s}s wall)"
  else
    echo "FAIL ${target}" >&2
    fail=1
  fi
done

if [[ "${fail}" -ne 0 ]]; then
  echo "cargo-fuzz campaign failed (see artifacts under core/fuzz/artifacts/)." >&2
  exit 1
fi

echo "cargo-fuzz campaign complete (0 crashes)."
