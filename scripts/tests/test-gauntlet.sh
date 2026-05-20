#!/usr/bin/env bash
# Test: gauntlet.sh dispatches the right families per tier, collects JSON results,
# returns the correct exit code, and writes results to gauntlet-results.json.
set -euo pipefail

cd "$(dirname "$0")/.."

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# --- Test 1: --tier required ---
if ./gauntlet.sh 2>/dev/null; then
  echo "FAIL: gauntlet.sh without --tier should fail"; exit 1
fi
echo "OK: --tier required"

# --- Test 2: PR tier runs property-fuzz + sim only ---
./gauntlet.sh --tier=pr --results="$TMPDIR/r-pr.json" || true  # placeholders fail; exit non-zero expected
python3 -c "
import json, sys
r = json.load(open('$TMPDIR/r-pr.json'))
families_run = [x['family'] for x in r['results'] if x.get('pass') is not None]
assert set(families_run) == {'property-fuzz', 'sim'}, families_run
print('OK: PR tier ran property-fuzz + sim')
"

# --- Test 3: Nightly tier runs all 5 families ---
./gauntlet.sh --tier=nightly --results="$TMPDIR/r-nightly.json" || true
python3 -c "
import json, sys
r = json.load(open('$TMPDIR/r-nightly.json'))
families_run = [x['family'] for x in r['results'] if x.get('pass') is not None]
assert set(families_run) == {'property-fuzz', 'sim', 'chaos', 'soak', 'perf-regression'}, families_run
print('OK: nightly tier ran all 5 families')
"

# --- Test 4: Weekly tier runs all 5 families ---
./gauntlet.sh --tier=weekly --results="$TMPDIR/r-weekly.json" || true
python3 -c "
import json, sys
r = json.load(open('$TMPDIR/r-weekly.json'))
families_run = [x['family'] for x in r['results'] if x.get('pass') is not None]
assert set(families_run) == {'property-fuzz', 'sim', 'chaos', 'soak', 'perf-regression'}, families_run
print('OK: weekly tier ran all 5 families')
"

# --- Test 5: Exit code is non-zero when any family fails ---
if ./gauntlet.sh --tier=pr --results="$TMPDIR/r-pr2.json" 2>/dev/null; then
  echo "FAIL: gauntlet.sh should exit non-zero when families fail"; exit 1
fi
echo "OK: non-zero exit on failure"

# --- Test 6: Invalid tier rejected ---
if ./gauntlet.sh --tier=bogus 2>/dev/null; then
  echo "FAIL: bogus tier should fail"; exit 1
fi
echo "OK: invalid tier rejected"

# --- Test 7: perf-regression.sh emits valid per-family JSON ---
# Requires running engine on $ENGINE_URL.
ENGINE_URL_CHECK="${ENGINE_URL:-http://localhost:8080}"
if curl -fsS "$ENGINE_URL_CHECK/health" >/dev/null 2>&1; then
  ./gauntlet/perf-regression.sh --tier=nightly > "$TMPDIR/perf.json"
  python3 -c "
import json
r = json.load(open('$TMPDIR/perf.json'))
required = ['family', 'tier', 'started_at', 'finished_at', 'pass', 'observations']
for k in required:
    assert k in r, k
assert r['family'] == 'perf-regression'
assert r['tier'] == 'nightly'
print('OK: perf-regression.sh emits valid family JSON')
"
else
  echo "SKIP: perf-regression.sh test (no engine running)"
fi

echo "PASS"
