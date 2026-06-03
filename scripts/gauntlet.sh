#!/usr/bin/env bash
# gauntlet.sh — top-level gauntlet orchestrator.
#
# Usage: gauntlet.sh --tier=pr|nightly|weekly [--engine-url=URL] [--results=PATH]
#
# Dispatches per-family scripts under scripts/gauntlet/<family>.sh, captures their
# JSON stdout, collects into gauntlet-results.json, prints summary.
# Exits 0 if all non-SKIP families passed, 1 otherwise.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TIER=""
ENGINE_URL="${KYMA_HTTP_ADDR:-http://localhost:8080}"
RESULTS="$ROOT/gauntlet-results.json"

for arg in "$@"; do
  case "$arg" in
    --tier=*) TIER="${arg#--tier=}" ;;
    --engine-url=*) ENGINE_URL="${arg#--engine-url=}" ;;
    --results=*) RESULTS="${arg#--results=}" ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

if [ -z "$TIER" ]; then
  echo "Usage: $0 --tier=pr|nightly|weekly [--engine-url=URL] [--results=PATH]" >&2
  exit 2
fi

case "$TIER" in
  pr|nightly|weekly) ;;
  *) echo "invalid tier: $TIER (must be pr|nightly|weekly)" >&2; exit 2 ;;
esac

export KYMA_HTTP_ADDR="$ENGINE_URL"

# ---------------------------------------------------------------------------
# Tier definitions: which families to run per tier.
# ---------------------------------------------------------------------------

FAMILIES_PR=(property-fuzz sim)
FAMILIES_NIGHTLY=(property-fuzz sim chaos soak perf-regression)
FAMILIES_WEEKLY=(property-fuzz sim chaos soak perf-regression)

# Per-family tier mapping: passes the right --tier= to each family script.
# Weekly only escalates soak; all other families cap at nightly's spec.
family_tier_arg() {
  local family="$1" outer="$2"
  case "$family:$outer" in
    soak:weekly) echo "weekly" ;;
    *:pr) echo "pr" ;;
    *) echo "nightly" ;;
  esac
}

# Choose the family list for this tier.
case "$TIER" in
  pr) FAMILIES=("${FAMILIES_PR[@]}") ;;
  nightly) FAMILIES=("${FAMILIES_NIGHTLY[@]}") ;;
  weekly) FAMILIES=("${FAMILIES_WEEKLY[@]}") ;;
esac

# ---------------------------------------------------------------------------
# Run each family. Collect JSON results into a temp file.
# ---------------------------------------------------------------------------

RESULTS_TMP="$(mktemp)"
trap 'rm -f "$RESULTS_TMP"' EXIT

echo "[]" > "$RESULTS_TMP"
overall_pass=true
all_families=(property-fuzz sim chaos soak perf-regression)

for family in "${all_families[@]}"; do
  # Is this family in the tier's list?
  in_tier=false
  for f in "${FAMILIES[@]}"; do
    if [ "$f" = "$family" ]; then in_tier=true; break; fi
  done

  if ! $in_tier; then
    # Record a SKIP entry.
    python3 - "$RESULTS_TMP" "$family" "$TIER" <<'PY'
import json, sys
results_file, family, tier = sys.argv[1], sys.argv[2], sys.argv[3]
r = json.load(open(results_file))
r.append({"family": family, "tier": tier, "skipped": True})
json.dump(r, open(results_file, "w"))
PY
    continue
  fi

  inner_tier="$(family_tier_arg "$family" "$TIER")"
  family_script="$ROOT/scripts/gauntlet/$family.sh"

  if [ ! -x "$family_script" ]; then
    echo "ERROR: $family_script missing or not executable" >&2
    overall_pass=false
    python3 - "$RESULTS_TMP" "$family" "$inner_tier" <<'PY'
import json, sys
results_file, family, tier = sys.argv[1], sys.argv[2], sys.argv[3]
r = json.load(open(results_file))
r.append({"family": family, "tier": tier, "pass": False, "observations": ["family script missing or not executable"]})
json.dump(r, open(results_file, "w"))
PY
    continue
  fi

  echo "::group::$family ($inner_tier)" >&2
  family_json="$("$family_script" --tier="$inner_tier" 2>&1 | tail -1 || true)"
  echo "::endgroup::" >&2

  if ! echo "$family_json" | python3 -c "import json,sys; json.loads(sys.stdin.read())" 2>/dev/null; then
    # Family script didn't emit valid JSON on its last line; treat as failure.
    echo "ERROR: $family did not emit valid JSON" >&2
    overall_pass=false
    python3 - "$RESULTS_TMP" "$family" "$inner_tier" <<'PY'
import json, sys
results_file, family, tier = sys.argv[1], sys.argv[2], sys.argv[3]
r = json.load(open(results_file))
r.append({"family": family, "tier": tier, "pass": False, "observations": ["malformed JSON output from family script"]})
json.dump(r, open(results_file, "w"))
PY
    continue
  fi

  # Append parsed JSON to results file via env var to avoid quoting issues.
  FAMILY_JSON="$family_json" python3 - "$RESULTS_TMP" <<'PY'
import json, os, sys
results_file = sys.argv[1]
r = json.load(open(results_file))
r.append(json.loads(os.environ["FAMILY_JSON"]))
json.dump(r, open(results_file, "w"))
PY

  # A family counts as OK if it passed OR explicitly skipped itself (e.g. an
  # unimplemented placeholder). A skip is not a failure.
  fam_pass="$(echo "$family_json" | python3 -c "import json,sys; d=json.loads(sys.stdin.read()); print('true' if (d.get('pass') or d.get('skipped')) else 'false')")"
  if [ "$fam_pass" != "true" ]; then
    overall_pass=false
  fi
done

# ---------------------------------------------------------------------------
# Write final results file (results array wrapped with tier metadata).
# ---------------------------------------------------------------------------

if $overall_pass; then
  overall_pass_json="true"
else
  overall_pass_json="false"
fi

ENGINE_URL="$ENGINE_URL" TIER="$TIER" python3 - "$RESULTS_TMP" "$RESULTS" "$overall_pass_json" <<'PY'
import json, os, sys
results_file, output_file, overall_pass_str = sys.argv[1], sys.argv[2], sys.argv[3]
results = json.load(open(results_file))
final = {
  "tier": os.environ["TIER"],
  "engine_url": os.environ["ENGINE_URL"],
  "overall_pass": overall_pass_str == "true",
  "results": results,
}
json.dump(final, open(output_file, "w"), indent=2)
PY

# ---------------------------------------------------------------------------
# Print summary.
# ---------------------------------------------------------------------------

echo ""
echo "=== gauntlet --tier=$TIER summary ==="
TIER="$TIER" python3 - "$RESULTS" <<'PY'
import json, os, sys
results_file = sys.argv[1]
tier = os.environ["TIER"]
r = json.load(open(results_file))
for entry in r["results"]:
    if entry.get("skipped"):
        print(f"  {entry['family']:18s}  SKIP (not in {tier} tier)")
    elif entry.get("pass"):
        print(f"  {entry['family']:18s}  PASS")
    else:
        obs = entry.get("observations", ["(no observations)"])
        print(f"  {entry['family']:18s}  FAIL  -- {obs[0] if obs else ''}")
print("  backcompat         external (see .github/workflows/backcompat.yml)")
overall = "PASS" if r["overall_pass"] else "FAIL"
print(f"=== overall: {overall} ===")
PY

if $overall_pass; then
  exit 0
else
  exit 1
fi
