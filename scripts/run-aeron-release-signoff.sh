#!/usr/bin/env bash
# Full-matrix Aeron release sign-off (Gate C): IPC + UDP + multicast, strict first-run profile.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export SS_AERON_RELEASE=1
export SS_AERON_MATRIX=full
export SS_AERON_GATE_C=1
export SS_AERON_ALLOW_SOFT_SKIP=0
export SS_AERON_LOG_FILE="${SS_AERON_LOG_FILE:-/tmp/aeron-release-signoff.log}"

exec "${ROOT}/scripts/run-aeron-integration.sh"
