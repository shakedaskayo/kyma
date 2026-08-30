#!/usr/bin/env bash
# ALTER TABLE ADD COLUMN E2E test.
#
# Ingest rows with schema V1 (3 cols), ALTER TABLE ADD COLUMN, ingest new
# rows with schema V2 (4 cols). Verify:
#   - old rows read back with the new column as NULL
#   - new rows have the new column populated
#   - SELECT on the new column works across both extents

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
export PENSIEVE_SELF_TRACE="off"   # deterministic storage-layout assertions
# Disable compaction + retention so the test is deterministic.
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-alter.log"
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
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null

section "Start pensieve"
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create table with V1 schema (3 cols) and ingest 3 rows"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name evolve \
    --schema 'timestamp:timestamp,status:int,path:string' >/dev/null

V1_PAYLOAD='{"timestamp":"2026-04-19T10:00:00Z","status":200,"path":"/"}
{"timestamp":"2026-04-19T10:00:01Z","status":404,"path":"/missing"}
{"timestamp":"2026-04-19T10:00:02Z","status":500,"path":"/oops"}'
curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: evolve' -H 'Content-Type: application/x-ndjson' \
    --data-binary "$V1_PAYLOAD" >/dev/null

v1_rows=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM evolve' | jq -r .n)
assert_eq "V1 rows ingested" "3" "$v1_rows"

section "ALTER TABLE ADD COLUMN latency:long"
./target/debug/pensieve-cli alter-table --db default --table evolve --add-column 'latency:long'

section "Query V1 data — new column comes back NULL"
v1_latency_non_null=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM evolve WHERE latency IS NOT NULL' | jq -r .n)
assert_eq "V1 rows have NULL latency" "0" "$v1_latency_non_null"

# Confirm the column is visible in projection (schema is promoted).
curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT status, latency FROM evolve LIMIT 1' | head -1 | jq . >/dev/null && \
    ok "promoted schema exposes latency in projections"

section "Ingest V2 data (4 cols including new latency)"
V2_PAYLOAD='{"timestamp":"2026-04-19T11:00:00Z","status":200,"path":"/api","latency":42}
{"timestamp":"2026-04-19T11:00:01Z","status":200,"path":"/api","latency":57}'
curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: evolve' -H 'Content-Type: application/x-ndjson' \
    --data-binary "$V2_PAYLOAD" >/dev/null

section "Cross-schema query"
total=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM evolve' | jq -r .n)
assert_eq "total rows = 5 (3 V1 + 2 V2)" "5" "$total"

with_latency=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM evolve WHERE latency IS NOT NULL' | jq -r .n)
assert_eq "latency populated only on V2 rows" "2" "$with_latency"

avg_latency=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT AVG(latency) AS avg FROM evolve WHERE latency IS NOT NULL' | jq -r .avg)
assert_eq "AVG(latency) across V2 rows = 49.5" "49.5" "$avg_latency"

section "Schema chain visible in catalog"
ss_count=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM schema_snapshots WHERE table_id = (SELECT id FROM tables WHERE name='evolve')")
assert_eq "schema_snapshots has 2 versions (V1 + V2)" "2" "$ss_count"

section "Duplicate ALTER rejected"
dup_output=$(./target/debug/pensieve-cli alter-table --db default --table evolve --add-column 'latency:long' 2>&1 || true)
if echo "$dup_output" | grep -qi "already exists"; then
    ok "duplicate ADD COLUMN rejected with 'already exists'"
else
    f "duplicate ADD COLUMN should have been rejected; saw: $dup_output"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    printf "\n${RED}Server log tail:${NC}\n"
    tail -40 "$LOG_FILE"
    exit 1
fi
exit 0
