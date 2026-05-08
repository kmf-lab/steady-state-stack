#!/usr/bin/env bash
# Run scoped mutation testing for the `steady_state` crate (see core/mutants.toml).
#
# Usage (from repository root):
#   bash scripts/run-cargo-mutants.sh
#
# Optional environment:
#   EXTRA_CARGO_MUTANTS_ARGS  Extra CLI tokens for `cargo mutants` (quote carefully), e.g.
#                             EXTRA_CARGO_MUTANTS_ARGS='--shard 1/4'
#   CARGO_MUTANTS_TIMEOUT_SECS  Wall-clock cap for all cargo commands in this run (default: 3600).
#   CARGO_MUTANTS_JOBS         If set, passed as `cargo mutants -j ...`.
#
# Widen scope by editing examine_globs in core/mutants.toml or passing --file / --exclude via
# EXTRA_CARGO_MUTANTS_ARGS. Expect long runtimes for full-crate mutation.
#
# Dot telemetry edge merge (`core/src/dot_unify.rs`) is included in `mutants.toml` `examine_globs`.
# For a quicker loop on that file only, use the dedicated config (does not apply other examine_globs):
#   cargo mutants --manifest-path core/Cargo.toml --config core/mutants.dot_unify.toml --cap-lints true -j 4
# Or combine glob + no config (see mutants.rs docs for `-f`):
#   cargo mutants --manifest-path core/Cargo.toml --no-config --test-tool nextest -f "**/dot_unify.rs" --cap-lints true
# For sharded CI: EXTRA_CARGO_MUTANTS_ARGS='--shard 1/4'
#
# Edge telemetry troubleshooting: STEADY_TELEMETRY_EDGE_DIAG / RUST_LOG — see docs/telemetry-edge-conflict.md
#
# Requires a Rust toolchain compatible with cargo-mutants (see https://mutants.rs/).
# Exit status is non-zero if the tool fails, tests fail for a mutant, or mutants survive
# (see upstream docs for exact exit codes).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if ! command -v cargo-mutants &>/dev/null; then
  echo "cargo-mutants not found, installing..."
  cargo install cargo-mutants --locked
fi

MUTANTS_CONFIG="${REPO_ROOT}/core/mutants.toml"
LOG="${REPO_ROOT}/cargo_mutants.txt"
: "${CARGO_MUTANTS_TIMEOUT_SECS:=3600}"

mut_args=(
  --manifest-path core/Cargo.toml
  --config "${MUTANTS_CONFIG}"
  --test-tool nextest
  --cap-lints true
  -t "${CARGO_MUTANTS_TIMEOUT_SECS}"
)

if [[ -n "${CARGO_MUTANTS_JOBS:-}" ]]; then
  mut_args+=( -j "${CARGO_MUTANTS_JOBS}" )
fi

# shellcheck disable=SC2086
cargo mutants "${mut_args[@]}" ${EXTRA_CARGO_MUTANTS_ARGS:-} 2>&1 | tee "${LOG}"
exit "${PIPESTATUS[0]}"
