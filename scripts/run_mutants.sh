#!/usr/bin/env bash
# Friendly entry point for scoped mutation testing on one CPU core.
#
# Defaults keep the machine responsive:
#   - one mutant job at a time (CARGO_MUTANTS_JOBS=1)
#   - one test thread per baseline/mutant run (NEXTEST_TEST_THREADS=1)
#
# Usage (from repository root):
#   bash scripts/run_mutants.sh
#
# Override when you have spare capacity:
#   CARGO_MUTANTS_JOBS=2 NEXTEST_TEST_THREADS=2 bash scripts/run_mutants.sh
#
# Narrow scope for a quick loop:
#   EXTRA_CARGO_MUTANTS_ARGS='--file dot_unify.rs' bash scripts/run_mutants.sh

set -euo pipefail

: "${CARGO_MUTANTS_JOBS:=1}"
: "${NEXTEST_TEST_THREADS:=1}"
export CARGO_MUTANTS_JOBS NEXTEST_TEST_THREADS

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec bash "${SCRIPT_DIR}/run-cargo-mutants.sh"
