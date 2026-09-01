#!/usr/bin/env bash
# Auth stub E2E test.
#
# Starts pensieve with PENSIEVE_AUTH_TOKENS configured, then verifies:
#   - no token → 401
#   - unknown token → 401
#   - read-token can query but cannot ingest (403)
#   - write-token can both ingest and query
#   - /health and /metrics are never blocked (no auth)

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
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export PENSIEVE_AUTH_TOKENS="reader-tok:read,writer-tok:write,admin-tok:admin"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-auth.log"
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
assert_status() {
    local name="$1" want="$2" got="$3"
    if [[ "$want" == "$got" ]]; then ok "$name (HTTP $got)"
    else f "$name — want $want got $got"
    fi
}
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state + start pensieve with auth enabled"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Bootstrap (CLI talks directly to Postgres — no auth path)"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name authtest \
    --schema 'timestamp:timestamp,n:int' >/dev/null

PAYLOAD='{"timestamp":"2026-04-19T10:00:00Z","n":1}'

section "Unauthenticated"
code=$(curl -s -o /dev/null -w '%{http_code}' "$HTTP_BASE/health")
assert_status "GET /health w/o token"                    "200" "$code"
code=$(curl -s -o /dev/null -w '%{http_code}' "$HTTP_BASE/metrics")
assert_status "GET /metrics w/o token"                   "200" "$code"

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: authtest' -H 'Content-Type: application/x-ndjson' --data-binary "$PAYLOAD")
assert_status "POST /v1/ingest w/o token"                "401" "$code"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT 1')
assert_status "POST /v1/query w/o token"                 "401" "$code"

section "Unknown token"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'Authorization: Bearer nope' \
    -H 'X-Database: default' -H 'X-Table: authtest' -H 'Content-Type: application/x-ndjson' --data-binary "$PAYLOAD")
assert_status "POST /v1/ingest unknown token"            "401" "$code"

section "Reader token — can query, cannot ingest"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/query" \
    -H 'Authorization: Bearer reader-tok' \
    -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM authtest')
assert_status "POST /v1/query with reader-tok"           "200" "$code"

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'Authorization: Bearer reader-tok' \
    -H 'X-Database: default' -H 'X-Table: authtest' -H 'Content-Type: application/x-ndjson' --data-binary "$PAYLOAD")
assert_status "POST /v1/ingest with reader-tok"          "403" "$code"

section "Writer token — can ingest and query"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'Authorization: Bearer writer-tok' \
    -H 'X-Database: default' -H 'X-Table: authtest' -H 'Content-Type: application/x-ndjson' --data-binary "$PAYLOAD")
assert_status "POST /v1/ingest with writer-tok"          "200" "$code"

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/query" \
    -H 'Authorization: Bearer writer-tok' \
    -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM authtest')
assert_status "POST /v1/query with writer-tok"           "200" "$code"

section "Admin token — same as writer"
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'Authorization: Bearer admin-tok' \
    -H 'X-Database: default' -H 'X-Table: authtest' -H 'Content-Type: application/x-ndjson' --data-binary "$PAYLOAD")
assert_status "POST /v1/ingest with admin-tok"           "200" "$code"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    printf "\n${RED}Server log tail:${NC}\n"
    tail -40 "$LOG_FILE"
    exit 1
fi
exit 0
