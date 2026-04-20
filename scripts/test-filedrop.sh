#!/usr/bin/env bash
# File-drop ingest E2E test.
#
# 1. Start kyma with filedrop watcher enabled + aggressive poll (2s).
# 2. Create target table.
# 3. Drop an NDJSON file into minio at ingest/{db}/{table}/file.ndjson.
# 4. Wait for the watcher to pick it up and ingest.
# 5. Verify row count + query results.
# 6. Drop the SAME content again → verify it's deduplicated via SHA256
#    idempotency ledger (row count unchanged).
# 7. Drop NEW content → verify it's ingested (different SHA).

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
export KYMA_FILEDROP_ENABLED=1
export KYMA_FILEDROP_PREFIX=ingest
export KYMA_FILEDROP_POLL_SECS=2
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-info,sqlx=warn,hyper=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-filedrop.log"
SERVER_PID=""

if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
    RED=""; GRN=""; BLU=""; DIM=""; NC=""
fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()      { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
f()       { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
info()    { printf "  ${DIM}%s${NC}\n" "$*"; }
cleanup() {
    if [[ -n "${SERVER_PID:-}" ]]; then
        kill -9 "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start kyma (filedrop watcher enabled)"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done
if grep -q "file-drop watcher enabled" "$LOG_FILE"; then
    ok "filedrop watcher started"
else
    f "filedrop not enabled in log"; tail -20 "$LOG_FILE"; exit 1
fi

section "Create target table (events)"
./target/debug/kyma-cli create-database default >/dev/null
./target/debug/kyma-cli create-table --db default --name events \
    --schema 'timestamp:timestamp,level:string,message:string' >/dev/null

section "Drop NDJSON file into ingest/default/events/*.ndjson"
cat > /tmp/filedrop-batch-a.ndjson <<'EOF'
{"timestamp":"2026-04-20T11:00:00Z","level":"INFO","message":"hello from file-drop"}
{"timestamp":"2026-04-20T11:00:01Z","level":"WARN","message":"second line"}
{"timestamp":"2026-04-20T11:00:02Z","level":"ERROR","message":"third line"}
EOF
docker cp /tmp/filedrop-batch-a.ndjson kyma-minio:/tmp/a.ndjson
docker exec kyma-minio mc cp /tmp/a.ndjson local/kyma/ingest/default/events/a.ndjson >/dev/null

info "waiting up to 15s for watcher..."
got=0
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
        --data 'SELECT COUNT(*) AS n FROM events' | jq -r .n 2>/dev/null || echo 0)
    if [[ "$n" == "3" ]]; then got=1; break; fi
    sleep 1
done
if (( got == 1 )); then
    ok "watcher ingested 3 rows from file-drop"
else
    f "watcher never ingested (last COUNT = $n)"
    tail -40 "$LOG_FILE"
    exit 1
fi

section "Verify content via query"
level_counts=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT level, COUNT(*) AS n FROM events GROUP BY level ORDER BY level' | jq -c -s .)
# Expect INFO=1, WARN=1, ERROR=1
if [[ "$level_counts" == *'"level":"INFO"'* ]] \
   && [[ "$level_counts" == *'"level":"WARN"'* ]] \
   && [[ "$level_counts" == *'"level":"ERROR"'* ]]; then
    ok "all three levels ingested"
else
    f "level grouping mismatch: $level_counts"
fi

section "Drop IDENTICAL file a second time → expect SHA dedup"
# Same content, different S3 object name — watcher computes SHA of content.
docker exec kyma-minio mc cp /tmp/a.ndjson local/kyma/ingest/default/events/a-copy.ndjson >/dev/null

# Wait a couple of poll cycles.
sleep 6
after_n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM events' | jq -r .n)
if [[ "$after_n" == "3" ]]; then
    ok "identical content was deduplicated (still 3 rows)"
else
    f "dedup failed: row count is $after_n (want 3)"
fi

section "Idempotency metric fired"
if curl -s "$HTTP_BASE/metrics" | grep -q 'kyma_ingest_idempotency_hits_total'; then
    ok "kyma_ingest_idempotency_hits_total exported"
else
    f "no idempotency-hit metric"
fi

section "Drop DIFFERENT content → ingests as new"
cat > /tmp/filedrop-batch-b.ndjson <<'EOF'
{"timestamp":"2026-04-20T12:00:00Z","level":"INFO","message":"second batch row 1"}
{"timestamp":"2026-04-20T12:00:01Z","level":"INFO","message":"second batch row 2"}
EOF
docker cp /tmp/filedrop-batch-b.ndjson kyma-minio:/tmp/b.ndjson
docker exec kyma-minio mc cp /tmp/b.ndjson local/kyma/ingest/default/events/b.ndjson >/dev/null

got=0
for i in 1 2 3 4 5 6 7 8 9 10; do
    n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
        --data 'SELECT COUNT(*) AS n FROM events' | jq -r .n 2>/dev/null || echo 0)
    if [[ "$n" == "5" ]]; then got=1; break; fi
    sleep 1
done
if (( got == 1 )); then
    ok "new content ingested (count = 5)"
else
    f "new content NOT ingested (count = $n)"
    tail -30 "$LOG_FILE"
fi

section "Filedrop metrics"
if curl -s "$HTTP_BASE/metrics" | grep -q 'kyma_filedrop_objects_processed_total'; then
    ok "kyma_filedrop_objects_processed_total exported"
else
    f "no filedrop-processed metric"
fi
if curl -s "$HTTP_BASE/metrics" | grep -q 'kyma_filedrop_rows_total'; then
    ok "kyma_filedrop_rows_total exported"
else
    f "no filedrop-rows metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
