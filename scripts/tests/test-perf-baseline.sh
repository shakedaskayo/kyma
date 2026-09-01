#!/usr/bin/env bash
# Test: perf-baseline.sh emits valid metrics JSON against a running engine.
set -euo pipefail

cd "$(dirname "$0")/.."

ENGINE_URL="${ENGINE_URL:-http://localhost:8080}"
TMPDIR="$(mktemp -d)"

# Stub fixtures for the test (real ones land in Task 5)
mkdir -p fixtures/perf-baseline
STUB_SEED=0; STUB_QUERIES=0
if [ ! -f fixtures/perf-baseline/seed.ndjson ]; then
  cat > fixtures/perf-baseline/seed.ndjson <<'EOF'
{"timestamp":"2026-01-01T00:00:00Z","service.name":"svc-a","severity_text":"INFO","message":"m1"}
{"timestamp":"2026-01-01T00:00:01Z","service.name":"svc-b","severity_text":"ERROR","message":"m2"}
EOF
  STUB_SEED=1
fi
if [ ! -f fixtures/perf-baseline/queries.txt ]; then
  printf 'q-count\tPOST /v1/query\tapplication/sql\tobs\tSELECT COUNT(*) FROM otel_logs\n' > fixtures/perf-baseline/queries.txt
  STUB_QUERIES=1
fi

cleanup() {
  rm -rf "$TMPDIR"
  [ "$STUB_SEED" = "1" ] && rm -f fixtures/perf-baseline/seed.ndjson
  [ "$STUB_QUERIES" = "1" ] && rm -f fixtures/perf-baseline/queries.txt
  rmdir fixtures/perf-baseline 2>/dev/null || true
  rmdir fixtures 2>/dev/null || true
  return 0
}
trap cleanup EXIT

# Use a tiny target so the test is fast.
./perf-baseline.sh --engine-url="$ENGINE_URL" --target-rows=200 > "$TMPDIR/baseline.json"

python3 -c "
import json
m = json.load(open('$TMPDIR/baseline.json'))
required = ['ingest_rps', 'query_p50_ms', 'query_p99_ms', 'ingest_total_rows', 'ingest_total_seconds', 'captured_at', 'hardware_label', 'pensieve_git_sha']
for k in required:
    assert k in m, f'missing key: {k}'
assert isinstance(m['ingest_rps'], (int, float)) and m['ingest_rps'] > 0, m['ingest_rps']
assert isinstance(m['query_p50_ms'], (int, float)) and m['query_p50_ms'] >= 0, m['query_p50_ms']
assert isinstance(m['query_p99_ms'], (int, float)) and m['query_p99_ms'] >= m['query_p50_ms'], m
assert m['ingest_total_rows'] >= 200, m['ingest_total_rows']
print('OK: baseline JSON shape valid')
"

echo PASS
