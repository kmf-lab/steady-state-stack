#!/usr/bin/env bash
# Full Aeron integration matrix: IPC suite + UDP + multicast (fresh driver each run).
set -euo pipefail

export SS_AERON_MATRIX=full
export SS_AERON_FRESH_DRIVER=1
export SS_AERON_RETRY=1
export SS_AERON_REQUIRED="${SS_AERON_REQUIRED:-1}"
export SS_AERON_ALLOW_SOFT_SKIP=0
export SS_AERON_DOUBLE_PASS=0

exec "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/run-aeron-integration.sh"
