#!/usr/bin/env bash
# Fail when cargo-llvm-cov would run live Aeron integration tests (Gate C), not Gate B coverage.
set -euo pipefail

if [[ "${SS_AERON_LLVM_COV_ALLOW_TESTS:-0}" == "1" ]]; then
  exit 0
fi

args=("$@")
for arg in "${args[@]}"; do
  case "${arg}" in
    --tests|--test=aeron_integration_suite|--test=aeron_preflight_smoke)
      echo "ERROR: cargo llvm-cov must not run live Aeron test binaries (Gate C)." >&2
      echo "  Use: bash scripts/run-llvm-cov-release.sh  (lib + aeron_integration_uri_contract only)" >&2
      echo "  Or:  ./scripts/run-aeron-integration.sh         (live driver required)" >&2
      echo "  To override: SS_AERON_LLVM_COV_ALLOW_TESTS=1" >&2
      exit 1
      ;;
    --test)
      echo "ERROR: cargo llvm-cov --test may include aeron_integration_suite; use --test aeron_integration_uri_contract or run-llvm-cov-release.sh" >&2
      exit 1
      ;;
  esac
done

exit 0
