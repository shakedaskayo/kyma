#!/usr/bin/env bash
# perf-capture-baseline.sh — capture a new perf baseline against a running engine
# and save to scripts/fixtures/perf-baseline.json.
#
# Designed for invocation from .github/workflows/perf-baseline-capture.yml.
# Sets GAUNTLET_HARDWARE_LABEL automatically when run in GHA.
#
# Usage: perf-capture-baseline.sh [--engine-url=URL] [--target-rows=N]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ENGINE_URL="${KYMA_HTTP_ADDR:-http://localhost:8080}"
TARGET_ROWS=100000
OUT="$ROOT/scripts/fixtures/perf-baseline.json"

for arg in "$@"; do
  case "$arg" in
    --engine-url=*) ENGINE_URL="${arg#--engine-url=}" ;;
    --target-rows=*) TARGET_ROWS="${arg#--target-rows=}" ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Hardware label: use GHA runner image when available.
if [ -n "${ImageOS:-}" ]; then
  export GAUNTLET_HARDWARE_LABEL="$ImageOS-gha"
elif [ -n "${RUNNER_OS:-}" ]; then
  export GAUNTLET_HARDWARE_LABEL="$RUNNER_OS-gha"
else
  export GAUNTLET_HARDWARE_LABEL="${GAUNTLET_HARDWARE_LABEL:-unknown-local}"
fi

echo "Capturing baseline against $ENGINE_URL (target rows: $TARGET_ROWS, label: $GAUNTLET_HARDWARE_LABEL)..." >&2

# Run baseline; output to file.
./scripts/perf-baseline.sh --engine-url="$ENGINE_URL" --target-rows="$TARGET_ROWS" > "$OUT"

echo "" >&2
echo "Baseline written to $OUT:" >&2
python3 -m json.tool "$OUT" >&2

echo "" >&2
echo "Next steps:" >&2
echo "  1. Commit this file." >&2
echo "  2. Open a PR with title: 'perf: re-baseline against $GAUNTLET_HARDWARE_LABEL ($(date -u +%Y-%m-%d))'." >&2
