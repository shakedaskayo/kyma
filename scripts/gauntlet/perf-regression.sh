#!/usr/bin/env bash
# perf-regression.sh — gauntlet family: runs perf-baseline.sh + perf-check.sh
# and emits per-family JSON.
set -euo pipefail

TIER="${1#--tier=}"
[ -n "$TIER" ] && [ "$TIER" != "--tier=" ] || { echo "Usage: $0 --tier=pr|nightly|weekly" >&2; exit 2; }

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENGINE_URL="${KYMA_HTTP_ADDR:-http://localhost:8080}"
BASELINE="$ROOT/scripts/fixtures/perf-baseline.json"
TMPCUR="$(mktemp)"
TMPCMP="$(mktemp)"
trap 'rm -f "$TMPCUR" "$TMPCMP"' EXIT

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# 1. Run baseline measurement.
if ! "$ROOT/scripts/perf-baseline.sh" --engine-url="$ENGINE_URL" --target-rows=100000 > "$TMPCUR" 2> /tmp/perf-baseline.err; then
  FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  ERRTAIL="$(tail -5 /tmp/perf-baseline.err 2>/dev/null | tr '\n' ' ' | tr '"' "'")"
  cat <<EOF
{"family":"perf-regression","tier":"$TIER","started_at":"$START","finished_at":"$FINISH","pass":false,"observations":["perf-baseline.sh failed: $ERRTAIL"]}
EOF
  exit 0
fi

# 2. Compare to checked-in baseline (may not exist yet → check skips gracefully).
"$ROOT/scripts/perf-check.sh" "$TMPCUR" "$BASELINE" > "$TMPCMP" || true

FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# 3. Emit per-family JSON, embedding the comparison result.
python3 - <<PY
import json
cmp = json.load(open("$TMPCMP"))
overall_pass = cmp.get("overall_pass", False)
observations = []

if cmp.get("baseline") is None:
    observations.append("no baseline.json yet; skipping comparison")
else:
    for c in cmp.get("comparisons", []):
        if not c.get("pass"):
            observations.append(f"{c['metric']}: current={c['current']} baseline={c['baseline']} ratio={c.get('ratio')} (tolerance {c['direction']}={c['tolerance']})")
    if not observations:
        observations.append("all metrics within tolerance")

print(json.dumps({
    "family": "perf-regression",
    "tier": "$TIER",
    "started_at": "$START",
    "finished_at": "$FINISH",
    "pass": overall_pass,
    "observations": observations,
    "comparison": cmp,
}))
PY
