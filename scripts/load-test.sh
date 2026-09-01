#!/usr/bin/env bash
# Sustained-ingest load test. Fires ~1000 rows/sec for 30 seconds, measures
# p50/p99 latency and end-to-end throughput. Verifies no data loss under
# sustained load.
#
# Tune via:
#   LOAD_DURATION_SECS   (default 30)
#   LOAD_PARALLELISM     (default 25)
#   LOAD_ROWS_PER_REQ    (default 40)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
export PENSIEVE_S3_ENDPOINT="http://localhost:9000"
export PENSIEVE_S3_BUCKET="pensieve"
export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
export PENSIEVE_S3_PATH_STYLE="true"
export PENSIEVE_S3_ALLOW_HTTP="true"
export PENSIEVE_HTTP_ADDR="127.0.0.1:8080"
export PENSIEVE_COMPACTION_POLL_SECS="5"
export PENSIEVE_COMPACTION_MIN_EXTENTS="20"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-load.log"
SERVER_PID=""

DURATION="${LOAD_DURATION_SECS:-30}"
PARALLEL="${LOAD_PARALLELISM:-8}"
ROWS_PER_REQ="${LOAD_ROWS_PER_REQ:-500}"

if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; YLW="\033[33m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
    RED=""; GRN=""; YLW=""; BLU=""; DIM=""; NC=""
fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()      { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
f()       { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
info()    { printf "  ${DIM}%s${NC}\n" "$*"; }
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill "$SERVER_PID" 2>/dev/null && wait "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start pensieve"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Bootstrap"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name load \
    --schema 'timestamp:timestamp,n:int,payload:string' >/dev/null

section "Generate one shared payload ($ROWS_PER_REQ rows)"
python3 - "$ROWS_PER_REQ" <<'PY' > /tmp/load_payload.ndjson
import json, sys
n = int(sys.argv[1])
for i in range(n):
    print(json.dumps({
        "timestamp": f"2026-04-19T10:{i//60:02d}:{i%60:02d}Z",
        "n": i,
        "payload": "x" * 64,
    }))
PY

section "Fire sustained load (${DURATION}s, ${PARALLEL} workers, ${ROWS_PER_REQ} rows/req)"
TMPDIR=$(mktemp -d)
: > "$TMPDIR/latencies.ms"
: > "$TMPDIR/errors"

worker() {
    local end_s=$1
    while :; do
        local now_s
        now_s=$(date +%s)
        if (( now_s >= end_s )); then break; fi
        local t0 t1
        # Use python for millisecond-resolution timing (BSD date lacks %N).
        t0=$(python3 -c 'import time;print(int(time.perf_counter()*1000))')
        local code
        code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
            -H 'X-Database: default' -H 'X-Table: load' -H 'Content-Type: application/x-ndjson' \
            --data-binary @/tmp/load_payload.ndjson)
        t1=$(python3 -c 'import time;print(int(time.perf_counter()*1000))')
        if [[ "$code" != "200" ]]; then
            echo "$code" >> "$TMPDIR/errors"
        else
            echo $((t1 - t0)) >> "$TMPDIR/latencies.ms"
        fi
    done
}
export -f worker
export HTTP_BASE TMPDIR

START_S=$(date +%s)
END_S=$((START_S + DURATION))
WORKER_PIDS=()
for _ in $(seq 1 "$PARALLEL"); do
    worker "$END_S" &
    WORKER_PIDS+=($!)
done
# Wait only for the workers — *not* `wait` with no args, which would also
# block on the pensieve server that was backgrounded at the top of the script.
for pid in "${WORKER_PIDS[@]}"; do
    wait "$pid" 2>/dev/null || true
done
END_REAL_S=$(date +%s)
elapsed_s=$((END_REAL_S - START_S))
(( elapsed_s < 1 )) && elapsed_s=1

section "Compute throughput + latency percentiles"
total_requests=$(wc -l < "$TMPDIR/latencies.ms" | tr -d ' ')
errors=$(wc -l 2>/dev/null < "$TMPDIR/errors" | tr -d ' ' || echo 0)
total_rows=$((total_requests * ROWS_PER_REQ))
rows_per_sec=$(echo "scale=1; $total_rows / $elapsed_s" | bc)
reqs_per_sec=$(echo "scale=1; $total_requests / $elapsed_s" | bc)

sort -n "$TMPDIR/latencies.ms" > "$TMPDIR/sorted.ms"
n=$total_requests
p50_ms=$(awk -v n="$n" 'NR==int(n*0.50)' "$TMPDIR/sorted.ms")
p90_ms=$(awk -v n="$n" 'NR==int(n*0.90)' "$TMPDIR/sorted.ms")
p99_ms=$(awk -v n="$n" 'NR==int(n*0.99)' "$TMPDIR/sorted.ms")

printf "  %-22s %s\n" "Duration"          "${elapsed_s}s"
printf "  %-22s %s\n" "Total requests"    "$total_requests"
printf "  %-22s %s\n" "Total rows"        "$total_rows"
printf "  %-22s %s\n" "Requests/sec"      "$reqs_per_sec"
printf "  %-22s ${GRN}%s${NC}\n" "Rows/sec"           "$rows_per_sec"
printf "  %-22s %s\n" "p50 latency"       "${p50_ms}ms"
printf "  %-22s %s\n" "p90 latency"       "${p90_ms}ms"
printf "  %-22s ${GRN}%s${NC}\n" "p99 latency"       "${p99_ms}ms"
printf "  %-22s %s\n" "Errors"            "$errors"

section "Verify no data loss (catalog-level)"
sleep 1
queried=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COALESCE(SUM(row_count),0) FROM extents
     WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='load')" \
    2>/dev/null | tr -d ' ')
queried=${queried:-0}
if [[ "$queried" == "$total_rows" ]]; then
    ok "catalog row count matches sent: $total_rows"
else
    f "row count mismatch: sent $total_rows, catalog $queried"
fi

[[ "$errors" == "0" ]] && ok "no ingest errors" || f "$errors ingest errors"

section "Observability: CAS conflicts + live extent count"
metrics=$(curl -s --max-time 5 "$HTTP_BASE/metrics" 2>/dev/null || echo "")
cas=$(echo "$metrics" | awk '/pensieve_catalog_cas_conflicts_total/{print $2}' | head -1)
extents_live=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL" 2>/dev/null | tr -d ' ')
info "CAS conflicts retried: ${cas:-0}"
info "Live extents: ${extents_live:-?}"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -40 "$LOG_FILE"; exit 1; fi
exit 0
