#!/usr/bin/env bash
# End-to-end compaction test.
#
# Ingests 10 tiny extents, waits for the scheduler to submit a task, for
# the worker to execute it, and verifies:
#   - row count preserved (no data loss)
#   - live extent count dropped
#   - compaction metrics fired
#   - a compaction snapshot appears in the chain

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
# Aggressive for a fast test:
export KYMA_COMPACTION_POLL_SECS="2"
export KYMA_COMPACTION_IDLE_SLEEP_MS="200"
export KYMA_COMPACTION_MIN_EXTENTS="3"
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-compaction.log"
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
assert_eq() {
    if [[ "$2" == "$3" ]]; then ok "$1"
    else
        f "$1"
        printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" "$2" "$3"
    fi
}
cleanup() {
    [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

# --- bring up stack if needed ---
if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
printf "  ${DIM}catalog dropped; bucket cleared${NC}\n"

section "Start kyma with fast-poll compaction scheduler"
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi
    sleep 1
done
printf "  ${DIM}server PID $SERVER_PID${NC}\n"

section "Create table, ingest 10 extents"
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name compact_me \
    --schema 'timestamp:timestamp,n:int,label:string' >/dev/null
for i in $(seq 1 10); do
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: compact_me' \
        -H 'Content-Type: application/x-ndjson' \
        --data-binary "{\"timestamp\":\"2026-04-19T10:$(printf %02d $i):00Z\",\"n\":$i,\"label\":\"batch-$i\"}" \
        >/dev/null
done

before_rows=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' \
    -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM compact_me' | jq -r .n)
before_extents=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='compact_me')")
assert_eq "ingested 10 rows" "10" "$before_rows"
assert_eq "created 10 live extents" "10" "$before_extents"

section "Wait for scheduler to submit + worker to compact"
# Poll until compaction happens (live extent count drops below starting count).
deadline=$((SECONDS + 30))
compacted=0
while (( SECONDS < deadline )); do
    live=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
        "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='compact_me')")
    if (( live < before_extents )); then
        compacted=1
        after_extents=$live
        break
    fi
    sleep 1
done

if (( compacted == 0 )); then
    f "compaction never ran (timed out after 30s)"
    printf "\n${RED}Server log tail:${NC}\n"; tail -40 "$LOG_FILE"
    exit 1
fi
printf "  ${DIM}live extents went $before_extents → $after_extents${NC}\n"
ok "live extent count dropped (${before_extents} → ${after_extents})"

section "Verify row count preserved"
after_rows=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' \
    -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM compact_me' | jq -r .n)
assert_eq "row count unchanged after compaction" "10" "$after_rows"

section "Verify compaction snapshot in chain"
compaction_snaps=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM snapshots WHERE table_id = (SELECT id FROM tables WHERE name='compact_me') AND summary->>'operation' = 'compaction'")
if (( compaction_snaps >= 1 )); then
    ok "at least one compaction snapshot committed ($compaction_snaps)"
else
    f "no compaction snapshots in chain"
fi

section "Verify metrics"
metrics=$(curl -s "$HTTP_BASE/metrics")
if echo "$metrics" | grep -q 'kyma_compaction_tasks_total{result="ok"}'; then ok "compaction_tasks_total counter fired"; else f "no compaction_tasks_total metric"; fi
if echo "$metrics" | grep -q 'kyma_compaction_tasks_submitted_total'; then ok "compaction_tasks_submitted_total counter fired"; else f "no compaction_tasks_submitted_total metric"; fi
if echo "$metrics" | grep -q 'kyma_compaction_bytes_out'; then ok "compaction_bytes_out histogram fired"; else f "no compaction_bytes_out metric"; fi

section "Verify MinIO has the new merged extent + source extents GC-pending"
live_objects=$(docker exec kyma-minio mc ls -r local/kyma/ | grep -c '\.kyma$')
# Source extents still in MinIO (soft-deleted in catalog, physical delete = later GC).
printf "  ${DIM}MinIO object count: $live_objects (source extents await physical GC)${NC}\n"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    printf "\n${RED}Server log tail:${NC}\n"
    tail -40 "$LOG_FILE"
    exit 1
fi
exit 0
