#!/usr/bin/env bash
# Run the canonical Aeron integration suite with logging enabled.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export RUST_LOG="${RUST_LOG:-info}"
# Live Aeron test binaries only run when this is set (Gate C). Gate A nextest must skip instantly.
export SS_AERON_GATE_C="${SS_AERON_GATE_C:-1}"

AERON_DIR="${AERON_DIR:-/dev/shm/aeron-default}"
SYSTEMD_UNIT="${SS_AERON_SYSTEMD_UNIT:-aeronmd}"
LOG_FILE="${SS_AERON_LOG_FILE:-/tmp/aeron-integration.log}"
POST_RESTART_SETTLE_SEC="${SS_AERON_POST_RESTART_SETTLE_SEC:-15}"

fresh_driver_disabled() {
  [[ "${SS_AERON_FRESH_DRIVER:-1}" == "0" ]]
}

retry_disabled() {
  [[ "${SS_AERON_RETRY:-1}" == "0" ]]
}

soft_skip_allowed() {
  [[ "${SS_AERON_ALLOW_SOFT_SKIP:-0}" == "1" ]]
}

aeron_required() {
  [[ "${SS_AERON_REQUIRED:-0}" == "1" ]]
}

# Release sign-off profile: strict first-run semantics (see docs/spec/08-streams-and-distributed.md).
release_min_pass_default() {
  case "${SS_AERON_MATRIX:-ipc}" in
    full|all) echo "17" ;;  # 14 IPC + udp_p2p_roundtrip + udp_p2p_many_small + multicast_roundtrip
    ipc) echo "14" ;;
    udp) echo "16" ;;       # 14 IPC + 2 UDP (no multicast unless SS_AERON_MULTICAST=1)
    *) echo "14" ;;
  esac
}

if [[ "${SS_AERON_RELEASE:-0}" == "1" ]]; then
  export SS_AERON_GATE_C=1
  export SS_AERON_REQUIRED=1
  export SS_AERON_ALLOW_SOFT_SKIP=0
  export SS_AERON_RETRY=0
  export SS_AERON_DOUBLE_PASS=0
  export SS_AERON_SCENARIO_RETRY=0
  if [[ -z "${SS_AERON_MIN_PASS:-}" ]]; then
    export SS_AERON_MIN_PASS="$(release_min_pass_default)"
  fi
fi

min_pass_required() {
  local explicit="${SS_AERON_MIN_PASS:-}"
  if [[ -n "${explicit}" ]]; then
    echo "${explicit}"
    return 0
  fi
  if aeron_required; then
    release_min_pass_default
    return 0
  fi
  echo "1"
}

apply_matrix_profile() {
  case "${SS_AERON_MATRIX:-ipc}" in
    ipc)
      ;;
    full|all)
      export SS_AERON_UDP=1
      export SS_AERON_MULTICAST=1
      ;;
    udp)
      export SS_AERON_UDP=1
      ;;
    *)
      echo "ERROR: unknown SS_AERON_MATRIX=${SS_AERON_MATRIX:-} (use ipc, full, udp, or all)" >&2
      exit 1
      ;;
  esac
}

docker_container_exists() {
  command -v docker >/dev/null 2>&1 \
    && docker ps -a --format '{{.Names}}' 2>/dev/null | grep -qx "${SYSTEMD_UNIT}"
}

systemd_unit_installed() {
  systemctl list-unit-files "${SYSTEMD_UNIT}.service" --no-pager --no-legend 2>/dev/null \
    | grep -q "^${SYSTEMD_UNIT}.service"
}

systemd_driver_active() {
  systemctl is-active --quiet "${SYSTEMD_UNIT}" 2>/dev/null
}

driver_process_running() {
  pgrep -f '/aeronmd' >/dev/null 2>&1
}

resolve_aeronmd() {
  if [[ -n "${AERONMD:-}" && -x "${AERONMD}" ]]; then
    echo "${AERONMD}"
    return 0
  fi
  if command -v aeronmd >/dev/null 2>&1; then
    local p
    p="$(command -v aeronmd)"
    if [[ -x "${p}" ]]; then
      echo "${p}"
      return 0
    fi
  fi
  if [[ -x /build/binaries/aeronmd ]]; then
    echo /build/binaries/aeronmd
    return 0
  fi
  if [[ -x /usr/local/bin/aeronmd ]]; then
    echo /usr/local/bin/aeronmd
    return 0
  fi
  while read -r pid cmd _rest; do
    if [[ "${cmd}" == */aeronmd ]]; then
      if [[ -x "${cmd}" ]]; then
        echo "${cmd}"
        return 0
      fi
      if [[ -n "${pid}" && -r "/proc/${pid}/exe" ]]; then
        exe="$(readlink -f "/proc/${pid}/exe" 2>/dev/null || true)"
        exe="${exe% (deleted)}"
        if [[ -n "${exe}" && -x "${exe}" ]]; then
          echo "${exe}"
          return 0
        fi
      fi
    fi
  done < <(pgrep -a -f '/aeronmd' 2>/dev/null || true)
  echo aeronmd
}

detect_restart_via() {
  case "${SS_AERON_RESTART_VIA:-auto}" in
    docker) echo docker ;;
    systemctl|systemd) echo systemctl ;;
    binary) echo binary ;;
    auto|*)
      if docker_container_exists; then
        echo docker
      elif systemd_unit_installed || systemd_driver_active; then
        echo systemctl
      elif [[ -x "$(resolve_aeronmd)" ]]; then
        echo binary
      else
        echo systemctl
      fi
      ;;
  esac
}

restart_via_docker() {
  echo "Restarting ${SYSTEMD_UNIT} via docker (AERON_DIR=${AERON_DIR})..."
  docker restart "${SYSTEMD_UNIT}" >/dev/null
  sleep "${POST_RESTART_SETTLE_SEC}"
}

run_systemctl() {
  local action="$1"
  if systemctl "${action}" "${SYSTEMD_UNIT}" 2>/dev/null; then
    return 0
  fi
  if sudo -n systemctl "${action}" "${SYSTEMD_UNIT}" 2>/dev/null; then
    return 0
  fi
  echo "ERROR: systemctl ${action} ${SYSTEMD_UNIT} failed (try: docker restart ${SYSTEMD_UNIT})" >&2
  return 1
}

restart_via_systemctl() {
  echo "Restarting ${SYSTEMD_UNIT} via systemctl (AERON_DIR=${AERON_DIR})..."
  if ! run_systemctl restart; then
    return 1
  fi
  sleep "${POST_RESTART_SETTLE_SEC}"
}

restart_via_binary() {
  if [[ ! -x "${AERONMD}" ]]; then
    echo "ERROR: AERONMD is not executable: ${AERONMD}" >&2
    echo "  For Docker install use SS_AERON_RESTART_VIA=docker (auto when container exists)" >&2
    return 1
  fi
  echo "Restarting aeronmd binary (AERON_DIR=${AERON_DIR})..."
  pkill -f '/aeronmd' 2>/dev/null || true
  sleep 2
  "${AERONMD}" -Daeron.dir="${AERON_DIR}" >>/tmp/aeronmd.log 2>&1 &
  sleep "${POST_RESTART_SETTLE_SEC}"
}

post_restart_wire_settle() {
  echo "Post-restart wire settle (preflight smoke, up to ${POST_RESTART_SETTLE_SEC}s budget in driver sleep)..."
  if ! cargo test -p steady_state \
    --test aeron_preflight_smoke aeron_preflight_wire_settle -- --nocapture 2>&1 | tee -a "${LOG_FILE}"; then
    echo "WARN: post-restart wire preflight failed; suite may soft-skip or fail." >&2
    return 1
  fi
  export SS_AERON_SCRIPT_PREFLIGHT_OK=1
  return 0
}

pre_suite_settle_after_script_preflight() {
  if [[ "${SS_AERON_SCRIPT_PREFLIGHT_OK:-0}" != "1" ]]; then
    return 0
  fi
  local sec="${SS_AERON_PRE_SUITE_SETTLE_SEC:-}"
  if [[ -z "${sec}" ]]; then
    if [[ "${SS_AERON_RELEASE:-0}" == "1" ]]; then
      sec=12
    else
      sec=10
    fi
  fi
  echo "Pre-suite driver settle (${sec}s after script preflight, before serial scenarios)..."
  sleep "${sec}"
}

AERON_RESTART_VIA="$(detect_restart_via)"
AERONMD="$(resolve_aeronmd)"
export AERONMD
export SS_AERON_RESTART_VIA="${SS_AERON_RESTART_VIA:-auto}"

apply_matrix_profile

echo "Using SS_AERON_RESTART_VIA=${AERON_RESTART_VIA}, AERONMD=${AERONMD}, unit=${SYSTEMD_UNIT}, matrix=${SS_AERON_MATRIX:-ipc}"

LAST_RESTART_OK=0

restart_aeronmd() {
  case "${AERON_RESTART_VIA}" in
    docker)
      if restart_via_docker; then
        LAST_RESTART_OK=1
        return 0
      fi
      return 1
      ;;
    systemctl)
      if restart_via_systemctl; then
        LAST_RESTART_OK=1
        return 0
      fi
      if docker_container_exists && restart_via_docker; then
        LAST_RESTART_OK=1
        return 0
      fi
      return 1
      ;;
    binary)
      if restart_via_binary; then
        LAST_RESTART_OK=1
        return 0
      fi
      if docker_container_exists && restart_via_docker; then
        LAST_RESTART_OK=1
        return 0
      fi
      return 1
      ;;
  esac
}

driver_running() {
  driver_process_running || systemd_driver_active
}

if ! fresh_driver_disabled; then
  echo "SS_AERON_FRESH_DRIVER=1 (default): restarting media driver for a clean CNC..."
  if ! restart_aeronmd; then
    exit 1
  fi
  if ! post_restart_wire_settle; then
    if aeron_required; then
      echo "ERROR: post-restart wire preflight failed (SS_AERON_REQUIRED=1)" >&2
      exit 1
    fi
    echo "WARN: post-restart wire preflight failed; suite may soft-skip." >&2
  fi
else
  if ! driver_running; then
    echo "ERROR: Aeron media driver is not running." >&2
    echo "  Start: docker restart ${SYSTEMD_UNIT} or sudo systemctl start ${SYSTEMD_UNIT}" >&2
    exit 1
  fi
fi

if ! driver_running; then
  echo "ERROR: Aeron media driver is not running." >&2
  exit 1
fi

if [[ ! -d "${AERON_DIR}" ]]; then
  echo "ERROR: Aeron directory missing: ${AERON_DIR}" >&2
  exit 1
fi

if [[ -f "${AERON_DIR}/cnc.dat" ]]; then
  cnc_mb="$(du -m "${AERON_DIR}/cnc.dat" 2>/dev/null | awk '{print $1}')"
  if [[ -n "${cnc_mb}" && "${cnc_mb}" -gt 64 ]]; then
    echo "WARN: ${AERON_DIR}/cnc.dat is ${cnc_mb}MB — driver may be stressed from prior runs." >&2
  fi
fi

count_pass_lines() {
  grep -c '^PASS \[' "${LOG_FILE}" 2>/dev/null || true
}

full_suite_soft_skip() {
  grep -q '^SKIP \[aeron_integration_serial_suite\]' "${LOG_FILE}" 2>/dev/null
}

analyze_log_or_fail() {
  local status="$1"
  if full_suite_soft_skip; then
    if aeron_required && [[ "${status}" -eq 0 ]]; then
      echo "ERROR: full suite soft-skipped but SS_AERON_REQUIRED=1" >&2
      return 1
    fi
    if ! soft_skip_allowed; then
      echo "ERROR: full suite soft-skipped (zero scenario coverage). Set SS_AERON_ALLOW_SOFT_SKIP=1 to allow." >&2
      echo "  Your log means zero behavioral coverage — fix driver/preflight or use SS_AERON_REQUIRED=1 to fail fast." >&2
      return 1
    fi
    echo "Skipped (no scenario coverage; SS_AERON_ALLOW_SOFT_SKIP=1)."
    return "${status}"
  fi
  if [[ "${status}" -ne 0 ]]; then
    return "${status}"
  fi
  local passes
  passes="$(count_pass_lines)"
  local min_pass
  min_pass="$(min_pass_required)"
  if [[ "${passes}" -lt 1 ]]; then
    echo "ERROR: suite exited 0 but log has no PASS [scenario] lines (false green)." >&2
    return 1
  fi
  if [[ "${passes}" -lt "${min_pass}" ]]; then
    echo "ERROR: expected at least ${min_pass} PASS [scenario] lines (got ${passes})." >&2
    echo "  IPC matrix incomplete — check driver health or set SS_AERON_MIN_PASS to override." >&2
    return 1
  fi
  if grep 'graph stop timed out or unclean' "${LOG_FILE}" 2>/dev/null | grep -qv 'continuing' \
    && [[ "${SS_AERON_ALLOW_UNCLEAN_SHUTDOWN:-0}" != "1" ]]; then
    echo "ERROR: log contains strict-scenario unclean graph shutdown (no 'continuing' lenient waiver)." >&2
    echo "  Set SS_AERON_ALLOW_UNCLEAN_SHUTDOWN=1 to waive, or fix shutdown on roundtrip scenarios." >&2
    return 1
  fi
  echo "Passed (${passes} scenario PASS lines in log, min=${min_pass})."
  return 0
}

run_suite() {
  : > "${LOG_FILE}"
  cargo test -p steady_state \
    --test aeron_integration_suite -- --nocapture 2>&1 | tee "${LOG_FILE}"
  return "${PIPESTATUS[0]}"
}

pre_suite_settle_after_script_preflight

echo "Running Aeron integration suite (serial, single binary)..."
export SS_AERON_SCRIPT_PREFLIGHT_OK="${SS_AERON_SCRIPT_PREFLIGHT_OK:-0}"
set +e
run_suite
status=$?
set -e

if ! analyze_log_or_fail "${status}"; then
  status=1
fi

if grep -q '^SKIP \[' "${LOG_FILE}" && ! full_suite_soft_skip; then
  echo ""
  echo "Note: one or more optional scenarios were skipped (UDP/multicast off or SS_AERON_SCENARIO filter)."
fi

if [[ "${status}" -eq 0 && "${SS_AERON_DOUBLE_PASS:-1}" != "0" ]]; then
  if full_suite_soft_skip; then
    echo "Skipping second pass (full suite was soft-skipped)."
  else
    echo ""
    echo "Second consecutive suite run (no driver restart)..."
    set +e
    run_suite
    status=$?
    set -e
    if ! analyze_log_or_fail "${status}"; then
      status=1
    fi
  fi
fi

if [[ "${status}" -ne 0 ]]; then
  if grep -qE 'ingress_avail=0|phase=Wire' "${LOG_FILE}"; then
    echo "" >&2
    if grep -qE '\[driver_refresh\]|driver refresh failed' "${LOG_FILE}"; then
      echo "Hint: Mid-suite driver refresh wire probe failed (ingress_avail=0 on stream 80000)." >&2
      echo "  In-suite restart uses SS_AERON_POST_RESTART_SETTLE_SEC (default 15), same as script start." >&2
      echo "  Release sign-off refreshes after ten, bundle, shutdown_bundle, backpressure (2,6,9,10); subprocess preflight smoke." >&2
    else
      echo "Hint: Wire failure (ingress_avail=0) — stale CNC, suite in-process warmup failed, or bundle lane not connected." >&2
    fi
    echo "  Check: same user as aeronmd; du -m /dev/shm/aeron-default/cnc.dat; docker restart aeronmd && sleep 20" >&2
    if [[ "${SS_AERON_RELEASE:-0}" == "1" ]]; then
      echo "  Release sign-off (SS_AERON_RELEASE=1): no auto-retry. Fix driver, then:" >&2
      echo "    bash scripts/run-aeron-release-signoff.sh" >&2
      echo "    bash scripts/run-aeron-flake-check.sh   # 3/3 consecutive passes" >&2
    fi
    if ! retry_disabled; then
      echo "SS_AERON_RETRY=1 (default): restarting media driver and rerunning suite once..." >&2
      if restart_aeronmd; then
        post_restart_wire_settle || true
        set +e
        run_suite
        status=$?
        set -e
        if ! analyze_log_or_fail "${status}"; then
          status=1
        fi
      else
        echo "ERROR: retry skipped because driver restart failed." >&2
      fi
    fi
  fi
fi

if [[ "${status}" -ne 0 ]]; then
  exit "${status}"
fi

if full_suite_soft_skip; then
  echo "Aeron integration suite finished (soft skip — no live scenarios ran)."
else
  passes="$(count_pass_lines)"
  echo "Aeron integration suite passed (${passes} PASS lines)."
fi
