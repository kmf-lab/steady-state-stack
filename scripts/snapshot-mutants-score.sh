#!/usr/bin/env bash
# ss[impl verify.process.mutants]
# Copy cargo-mutants progress into the committed ledger and print kill-rate.
#
# Usage (from repository root):
#   bash scripts/snapshot-mutants-score.sh
#   SS_MUTANTS_FILE=dot_unify.rs bash scripts/snapshot-mutants-score.sh
#
# Reads mutants.out/ (gitignored). Writes docs/mutants/latest/ and refreshes
# the current-score block in docs/mutants.md.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

OUT_DIR="${REPO_ROOT}/mutants.out"
LEDGER="${REPO_ROOT}/docs/mutants/latest"
DOC="${REPO_ROOT}/docs/mutants.md"
mkdir -p "${LEDGER}"

merge_unique_lists() {
  local live="$1"
  local kept="$2"
  touch "${live}" "${kept}"
  sort -u "${live}" "${kept}" | grep -v '^[[:space:]]*$' > "${kept}.merged" || true
  mv "${kept}.merged" "${kept}"
  cp "${kept}" "${live}"
}

# cargo-mutants rewrites caught.txt each run; union with the committed ledger first.
if [[ -f "${OUT_DIR}/caught.txt" || -f "${LEDGER}/caught.txt" ]]; then
  merge_unique_lists "${OUT_DIR}/caught.txt" "${LEDGER}/caught.txt"
fi
if [[ -f "${OUT_DIR}/timeout.txt" || -f "${LEDGER}/timeout.txt" ]]; then
  merge_unique_lists "${OUT_DIR}/timeout.txt" "${LEDGER}/timeout.txt"
fi
if [[ -f "${OUT_DIR}/unviable.txt" || -f "${LEDGER}/unviable.txt" ]]; then
  merge_unique_lists "${OUT_DIR}/unviable.txt" "${LEDGER}/unviable.txt"
fi
if [[ -f "${OUT_DIR}/missed.txt" ]]; then
  cp "${OUT_DIR}/missed.txt" "${LEDGER}/missed.txt"
fi

count_lines() {
  local path="$1"
  if [[ -f "${path}" ]]; then
    grep -cve '^[[:space:]]*$' "${path}" || true
  else
    echo 0
  fi
}

caught="$(count_lines "${OUT_DIR}/caught.txt")"
missed="$(count_lines "${OUT_DIR}/missed.txt")"
timeout="$(count_lines "${OUT_DIR}/timeout.txt")"
unviable="$(count_lines "${OUT_DIR}/unviable.txt")"

# File lists (merged with the ledger) are the campaign source of truth.
# outcomes.json is per-run and would shrink --iterate totals.

scored="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
git_sha="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
crate_version="$(python3 - <<'PY' 2>/dev/null || echo unknown
import re
text = open("core/Cargo.toml", errors="replace").read()
m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
print(m.group(1) if m else "unknown")
PY
)"
file_label="${SS_MUTANTS_FILE:-examine_globs}"

python3 - "${LEDGER}/score.json" "${scored}" "${git_sha}" "${crate_version}" "${file_label}" \
  "${caught}" "${missed}" "${timeout}" "${unviable}" <<'PY'
import json, sys
path, scored, sha, ver, file_label, c, m, t, u = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5], int(sys.argv[6]), int(sys.argv[7]), int(sys.argv[8]), int(sys.argv[9])
denom = c + t + m
kill = round(100.0 * (c + t) / denom, 1) if denom else None
data = {
    "scored_at": scored,
    "git_sha": sha,
    "crate_version": ver,
    "file": file_label,
    "caught": c,
    "missed": m,
    "timeout": t,
    "unviable": u,
    "kill_rate_pct": kill,
    "formula": "(caught + timeout) / (caught + timeout + missed)",
}
open(path, "w").write(json.dumps(data, indent=2) + "\n")
print(json.dumps(data, indent=2))
PY

for f in caught.txt missed.txt timeout.txt unviable.txt outcomes.json; do
  if [[ -f "${OUT_DIR}/${f}" ]]; then
    cp "${OUT_DIR}/${f}" "${LEDGER}/${f}"
  elif [[ ! -f "${LEDGER}/${f}" ]]; then
    : > "${LEDGER}/${f}"
  fi
done

# Slim outcomes.json for git: keep counts + last 20 scenario names, drop logs.
if [[ -f "${OUT_DIR}/outcomes.json" ]] && command -v python3 >/dev/null 2>&1; then
  python3 - "${OUT_DIR}/outcomes.json" "${LEDGER}/outcomes.json" <<'PY'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
d = json.load(open(src))
slim_outcomes = []
for o in d.get("outcomes", [])[-20:]:
    slim_outcomes.append({
        "scenario": o.get("scenario"),
        "summary": o.get("summary"),
    })
slim = {
    "caught": d.get("caught"),
    "missed": d.get("missed"),
    "timeout": d.get("timeout"),
    "unviable": d.get("unviable"),
    "total_mutants": d.get("total_mutants"),
    "success": d.get("success"),
    "start_time": d.get("start_time"),
    "end_time": d.get("end_time"),
    "cargo_mutants_version": d.get("cargo_mutants_version"),
    "recent_outcomes": slim_outcomes,
}
open(dst, "w").write(json.dumps(slim, indent=2) + "\n")
PY
fi

kill_rate="$(python3 -c "import json; d=json.load(open('${LEDGER}/score.json')); print('n/a' if d['kill_rate_pct'] is None else d['kill_rate_pct'])")"

if [[ -f "${DOC}" ]]; then
  python3 - "${DOC}" "${LEDGER}/score.json" <<'PY'
import json, re, sys
doc_path, score_path = sys.argv[1], sys.argv[2]
d = json.load(open(score_path))
kill = "n/a" if d["kill_rate_pct"] is None else f"{d['kill_rate_pct']}%"
row = (
    f"| {d['scored_at'][:10]} | `{d['git_sha']}` | {d['file']} | {d['caught']} | "
    f"{d['missed']} | {d['timeout']} | {d['unviable']} | {kill} |"
)
block = "\n".join([
    f"- **When:** {d['scored_at']}",
    f"- **git:** `{d['git_sha']}`  **crate:** {d['crate_version']}  **file:** `{d['file']}`",
    f"- **caught:** {d['caught']}  **missed:** {d['missed']}  **timeout:** {d['timeout']}  **unviable:** {d['unviable']}",
    f"- **kill-rate:** {kill}  (`(caught + timeout) / (caught + timeout + missed)`)",
]) + "\n"
text = open(doc_path, errors="replace").read()
text, n = re.subn(
    r"<!-- CAMPAIGN_CURRENT_START -->.*?<!-- CAMPAIGN_CURRENT_END -->",
    "<!-- CAMPAIGN_CURRENT_START -->\n" + block + "<!-- CAMPAIGN_CURRENT_END -->",
    text,
    count=1,
    flags=re.S,
)
if n != 1:
    sys.stderr.write("WARNING: CAMPAIGN_CURRENT markers missing in docs/mutants.md\n")
marker = "<!-- CAMPAIGN_LOG_ROWS -->"
if marker in text and row not in text:
    text = text.replace(marker, marker + "\n" + row, 1)
open(doc_path, "w").write(text)
PY
fi

echo "snapshot: kill-rate=${kill_rate}  ledger=${LEDGER}"
