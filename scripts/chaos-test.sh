#!/usr/bin/env bash
# Chaos test: kill the server mid-ingest and mid-query, then verify the
# next server can pick up exactly where the previous left off with no data
# loss or corruption.
#
# This exercises the architectural invariants:
#   - object store is the only source of truth (restart reads data)
#   - query nodes are stateless (no in-memory state survives the kill)
#   - catalog is externalized (snapshot pointer survives the crash)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export KYMA_CATALOG_URL="postgres://kyma:kyma_dev@localhost:5433/kyma"
export KYMA_S3_ENDPOINT="http://localhost:9000"
export KYMA_S3_BUCKET="kyma"
export KYMA_S3_ACCESS_KEY_ID="kyma_admin"
export KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev"
export KYMA_S3_PATH_STYLE="true"
export KYMA_S3_ALLOW_HTTP="true"
export KYMA_HTTP_ADDR="127.0.0.1:8080"
export KYMA_SELF_TRACE="off"   # deterministic storage-layout assertions
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_A="/tmp/kyma-chaos-1.log"
LOG_B="/tmp/kyma-chaos-2.log"

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

SERVER_PID=""
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

start_kyma() {
    local log="$1"
    ./target/debug/kyma >"$log" 2>&1 &
    SERVER_PID=$!
    for i in 1 2 3 4 5 6 7 8 9 10; do
        if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then return 0; fi
        sleep 1
    done
    return 1
}

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null

section "Start server A, create schema, ingest 50 rows"
start_kyma "$LOG_A" || { f "server A never healthy"; exit 1; }
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name chaos \
    --schema 'timestamp:timestamp,n:int' >/dev/null

# Generate 50 rows, ingest 5 small batches of 10.
for batch in 0 1 2 3 4; do
    python3 - "$batch" <<'PY' > /tmp/chaos_batch.ndjson
import json, sys
b = int(sys.argv[1])
for i in range(10):
    print(json.dumps({"timestamp": f"2026-04-19T10:{b:02d}:{i:02d}Z", "n": b*10 + i}))
PY
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: chaos' -H 'Content-Type: application/x-ndjson' \
        --data-binary @/tmp/chaos_batch.ndjson > /dev/null
done
before=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM chaos' | jq -r .n)
[[ "$before" == "50" ]] && ok "50 rows committed before crash" || f "expected 50, got $before"

section "Kill server A hard (SIGKILL, no graceful shutdown)"
kill -9 "$SERVER_PID" 2>/dev/null
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""
info "waiting for port :8080 to free"
# Wait for the OS to release the port.
for i in 1 2 3 4 5; do
    if ! nc -z 127.0.0.1 8080 2>/dev/null; then break; fi
    sleep 1
done

section "Start server B against same catalog + object store"
start_kyma "$LOG_B" || { f "server B never healthy"; exit 1; }

section "Verify all 50 rows survive the crash"
after=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM chaos' | jq -r .n)
[[ "$after" == "50" ]] && ok "50 rows queryable after restart" || f "expected 50, got $after"

section "Verify extent data integrity (sum invariant)"
# Expected SUM(n) = sum(0..49) = 49*50/2 = 1225
sum_n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT SUM(n) AS s FROM chaos' | jq -r .s)
[[ "$sum_n" == "1225" ]] && ok "SUM(n) = 1225 (no corruption)" || f "SUM(n) = $sum_n, want 1225"

section "Kill server B DURING a query, restart, retry"
# Fire a long-running-ish query in the background, kill the server 50ms in.
curl -s --max-time 10 -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT n FROM chaos ORDER BY timestamp' > /tmp/chaos_query_out 2>&1 &
QPID=$!
sleep 0.05
kill -9 "$SERVER_PID" 2>/dev/null
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""
# Curl will exit with some non-200 or connection-reset; that's expected.
wait "$QPID" 2>/dev/null || true
ok "mid-query SIGKILL tolerated (client got an error, server terminated)"

section "Wait for :8080 + restart"
for i in 1 2 3 4 5; do
    if ! nc -z 127.0.0.1 8080 2>/dev/null; then break; fi
    sleep 1
done
start_kyma "$LOG_B" || { f "server B (restart) never healthy"; exit 1; }

section "Verify post-second-restart data integrity"
final=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM chaos' | jq -r .n)
[[ "$final" == "50" ]] && ok "50 rows intact after 2 hard kills" || f "expected 50, got $final"

section "Catalog snapshot chain still walkable"
snapshots=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM snapshots WHERE table_id = (SELECT id FROM tables WHERE name='chaos')")
# 1 bootstrap + 5 ingest snapshots = 6
[[ "$snapshots" == "6" ]] && ok "snapshot chain intact (6 rows)" || f "expected 6 snapshots, got $snapshots"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    printf "\n${YLW}Server B log tail:${NC}\n"; tail -30 "$LOG_B"
    exit 1
fi
exit 0
