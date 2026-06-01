#!/usr/bin/env bash
# Smoke-test simple_aeron_subscriber + simple_aeron_publisher (requires aeronmd, same user).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FEATURES="${SS_AERON_EXAMPLE_FEATURES:-exec_async_std}"
TIMEOUT_SEC="${SS_AERON_EXAMPLE_TIMEOUT_SEC:-45}"
SUB_LOG="${SS_AERON_EXAMPLE_SUB_LOG:-/tmp/aeron-example-subscriber.log}"
PUB_LOG="${SS_AERON_EXAMPLE_PUB_LOG:-/tmp/aeron-example-publisher.log}"

if ! command -v timeout >/dev/null 2>&1; then
  echo "ERROR: 'timeout' required" >&2
  exit 1
fi

media_driver_available() {
  if [[ -r "${AERON_DIR:-/dev/shm/aeron-default}/cnc.dat" ]]; then
    return 0
  fi
  if command -v docker >/dev/null 2>&1 \
    && docker ps --format '{{.Names}}' 2>/dev/null | grep -qx "${SS_AERON_SYSTEMD_UNIT:-aeronmd}"; then
    return 0
  fi
  return 1
}

if ! media_driver_available; then
  if [[ "${SS_AERON_REQUIRED:-0}" == "1" ]]; then
    echo "ERROR: media driver required for examples smoke (SS_AERON_REQUIRED=1)" >&2
    exit 1
  fi
  echo "SKIP: no Aeron media driver — examples smoke not run"
  exit 0
fi

echo "Building examples..."
cargo build -q -p steady_state --features "${FEATURES}" \
  --example simple_aeron_subscriber --example simple_aeron_publisher

SUB_BIN="${ROOT}/target/debug/examples/simple_aeron_subscriber"
PUB_BIN="${ROOT}/target/debug/examples/simple_aeron_publisher"

rm -f "${SUB_LOG}" "${PUB_LOG}"

echo "Starting subscriber (timeout ${TIMEOUT_SEC}s)..."
timeout "${TIMEOUT_SEC}" "${SUB_BIN}" >"${SUB_LOG}" 2>&1 &
SUB_PID=$!

sleep 2

echo "Running publisher (timeout ${TIMEOUT_SEC}s)..."
set +e
timeout "${TIMEOUT_SEC}" "${PUB_BIN}" >"${PUB_LOG}" 2>&1
PUB_STATUS=$?
set -e

wait "${SUB_PID}" 2>/dev/null || SUB_STATUS=$?
SUB_STATUS=${SUB_STATUS:-0}

if [[ "${PUB_STATUS}" -ne 0 && "${PUB_STATUS}" -ne 124 ]]; then
  echo "ERROR: publisher exited ${PUB_STATUS}" >&2
  tail -30 "${PUB_LOG}" >&2 || true
  exit 1
fi

if grep -qi 'unclean' "${SUB_LOG}" "${PUB_LOG}" 2>/dev/null; then
  echo "ERROR: examples log contains unclean shutdown" >&2
  exit 1
fi

if grep -qi 'aeron skipped\|aeron test skipped' "${SUB_LOG}" "${PUB_LOG}" 2>/dev/null; then
  echo "ERROR: example skipped Aeron despite driver probe" >&2
  exit 1
fi

echo "PASS: examples smoke (subscriber log: ${SUB_LOG}, publisher log: ${PUB_LOG})"
