#!/usr/bin/env bash
# Test: perf-check.sh compares two JSON files; passes in tolerance, fails out of tolerance,
# warns-only by default but enforces when GAUNTLET_PERF_ENFORCE=1.
set -euo pipefail

cd "$(dirname "$0")/.."

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Baseline (the "known good" numbers)
cat > "$TMPDIR/baseline.json" <<EOF
{"ingest_rps":1000.0,"query_p50_ms":10.0,"query_p99_ms":50.0,"ingest_total_rows":100000,"ingest_total_seconds":100.0,"captured_at":"2026-01-01T00:00:00Z","hardware_label":"ubuntu-22.04-gha","pensieve_git_sha":"abc"}
EOF

# Current run within tolerance (ingest 950 / 0.95*1000 = OK; query_p99 55 / 50*1.1 = 55 OK)
cat > "$TMPDIR/current-ok.json" <<EOF
{"ingest_rps":950.0,"query_p50_ms":10.5,"query_p99_ms":55.0,"ingest_total_rows":100000,"ingest_total_seconds":105.0,"captured_at":"2026-01-02T00:00:00Z","hardware_label":"ubuntu-22.04-gha","pensieve_git_sha":"def"}
EOF

# Current run out of tolerance on ingest_rps (800 < 900)
cat > "$TMPDIR/current-bad.json" <<EOF
{"ingest_rps":800.0,"query_p50_ms":10.0,"query_p99_ms":50.0,"ingest_total_rows":100000,"ingest_total_seconds":125.0,"captured_at":"2026-01-02T00:00:00Z","hardware_label":"ubuntu-22.04-gha","pensieve_git_sha":"def"}
EOF

# --- Test 1: warn-only mode passes regardless of tolerance ---
unset GAUNTLET_PERF_ENFORCE
./perf-check.sh "$TMPDIR/current-bad.json" "$TMPDIR/baseline.json" > "$TMPDIR/out1.json"
RC=$?
[ $RC -eq 0 ] || { echo "FAIL: warn-only mode should exit 0 even on regression"; exit 1; }
python3 -c "
import json
r = json.load(open('$TMPDIR/out1.json'))
assert r['enforce_mode'] is False, r['enforce_mode']
assert r['overall_pass'] is False, 'comparison should show fail'
bad = [c for c in r['comparisons'] if not c['pass']]
assert any(c['metric'] == 'ingest_rps' for c in bad), 'should flag ingest_rps'
print('OK: warn-only mode')
"

# --- Test 2: in-tolerance passes with enforce_mode=1 ---
GAUNTLET_PERF_ENFORCE=1 ./perf-check.sh "$TMPDIR/current-ok.json" "$TMPDIR/baseline.json" > "$TMPDIR/out2.json"
echo "OK: in-tolerance passes under enforce"

# --- Test 3: out-of-tolerance fails with enforce_mode=1 ---
if GAUNTLET_PERF_ENFORCE=1 ./perf-check.sh "$TMPDIR/current-bad.json" "$TMPDIR/baseline.json" > "$TMPDIR/out3.json"; then
  echo "FAIL: enforce mode should exit non-zero on regression"; exit 1
fi
python3 -c "
import json
r = json.load(open('$TMPDIR/out3.json'))
assert r['enforce_mode'] is True
assert r['overall_pass'] is False
print('OK: enforce mode catches regression')
"

# --- Test 4: missing baseline emits skip rather than fail ---
./perf-check.sh "$TMPDIR/current-ok.json" "$TMPDIR/nonexistent-baseline.json" > "$TMPDIR/out4.json"
python3 -c "
import json
r = json.load(open('$TMPDIR/out4.json'))
assert r['overall_pass'] is True, r
assert any('no baseline' in str(x).lower() for x in r.get('observations', [])), r
print('OK: missing baseline gracefully skipped')
"

echo PASS
