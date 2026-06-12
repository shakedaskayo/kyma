#!/usr/bin/env bash
# gate.sh — per-stage merge gate for the scale program (S-series).
#
# Usage: GATE=s0 scripts/tests/gate.sh
#    or: scripts/tests/gate.sh s0
#
# A stage's gate is the single command that must be green before the next
# stage of the program starts. It runs (1) the workspace test suite,
# (2) the stage's named integration scripts, and (3) the stage's benchmark
# checks against committed baselines. Stages are cumulative: s1 runs s0's
# scripts too, so a later stage can never silently regress an earlier gate.
#
# Add scripts to a stage by appending to the STAGE_SCRIPTS_<n> arrays below.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

STAGE="${GATE:-${1:-}}"
if [ -z "$STAGE" ]; then
  echo "Usage: GATE=s0|s1|s2|s3|s4 $0   (or: $0 s0)" >&2
  exit 2
fi

# --- Stage manifests (cumulative) -------------------------------------------

# S0: foundations — the pre-existing engine suite must stay green, plus the
# new eval/testkit determinism checks as they land.
STAGE_SCRIPTS_0=(
  scripts/e2e-test.sh
  scripts/test-pushdown.sh
  scripts/test-compaction.sh
  scripts/test-retention.sh
  scripts/test-a-two-nodes.sh
  scripts/test-scale-out.sh
  scripts/chaos-test.sh
)

# S1: retrieval intelligence — populated as S1 phases land.
STAGE_SCRIPTS_1=(
  scripts/retrieval-bench.sh
)

# S2: engine core — populated as S2 phases land.
STAGE_SCRIPTS_2=(
  scripts/backcompat-replay.sh
)

# S3: graph + memory — populated as S3 phases land.
STAGE_SCRIPTS_3=(
)

# S4: final gauntlet (everything + soak tiers, run before merge to main).
STAGE_SCRIPTS_4=(
)

case "$STAGE" in
  s0) LEVELS=(0) ;;
  s1) LEVELS=(0 1) ;;
  s2) LEVELS=(0 1 2) ;;
  s3) LEVELS=(0 1 2 3) ;;
  s4) LEVELS=(0 1 2 3 4) ;;
  *) echo "invalid stage: $STAGE (must be s0..s4)" >&2; exit 2 ;;
esac

# --- Run ---------------------------------------------------------------------

FAILED=()
PASSED=()
SKIPPED=()

run_step() {
  local name="$1"; shift
  echo ""
  echo "=== gate[$STAGE] :: $name ==="
  local start=$SECONDS
  if "$@"; then
    PASSED+=("$name ($((SECONDS - start))s)")
  else
    FAILED+=("$name")
  fi
}

run_step "cargo test --workspace" cargo test --workspace --quiet

for lvl in "${LEVELS[@]}"; do
  arr="STAGE_SCRIPTS_${lvl}[@]"
  for script in "${!arr+"${!arr}"}"; do
    [ -z "$script" ] && continue
    if [ ! -x "$script" ] && [ ! -f "$script" ]; then
      SKIPPED+=("$script (not present yet)")
      continue
    fi
    run_step "$script" bash "$script"
  done
done

# S4 additionally runs the full gauntlet nightly tier.
if [ "$STAGE" = "s4" ]; then
  run_step "gauntlet --tier=nightly" bash scripts/gauntlet.sh --tier=nightly
fi

# --- Summary -----------------------------------------------------------------

echo ""
echo "================ gate[$STAGE] summary ================"
for p in "${PASSED[@]:-}";  do [ -n "$p" ] && echo "  PASS  $p"; done
for s in "${SKIPPED[@]:-}"; do [ -n "$s" ] && echo "  SKIP  $s"; done
for f in "${FAILED[@]:-}";  do [ -n "$f" ] && echo "  FAIL  $f"; done

if [ "${#FAILED[@]}" -gt 0 ]; then
  echo "gate[$STAGE]: FAILED (${#FAILED[@]} step(s))"
  exit 1
fi
echo "gate[$STAGE]: GREEN"
