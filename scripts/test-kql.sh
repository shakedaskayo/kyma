#!/usr/bin/env bash
# E2E test: KQL end-to-end via Content-Type: application/x-kql.

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
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-kql.log"
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
    else f "$1"
        printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" "$2" "$3"
    fi
}
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create table + ingest diverse data"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name http_logs \
    --schema 'timestamp:timestamp,status:int,path:string,message:string' >/dev/null

python3 - <<'PY' > /tmp/kql_data.ndjson
import json, random
random.seed(7)
paths = ["/", "/api/users", "/api/orders", "/admin", "/healthz", "/api/search"]
for i in range(50):
    st = random.choices([200, 301, 404, 500, 502], weights=[70, 5, 10, 10, 5])[0]
    msg = "ok" if st == 200 else ("redirect" if st == 301 else "oom" if random.random() < 0.3 else "generic err")
    print(json.dumps({
        "timestamp": f"2026-04-19T10:{i//60:02d}:{i%60:02d}Z",
        "status": st,
        "path": random.choice(paths),
        "message": msg,
    }))
PY
curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: http_logs' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/kql_data.ndjson > /dev/null

kql() {
    curl -s -X POST "$HTTP_BASE/v1/query" \
        -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
        --data "$1"
}

section "Basic scan via KQL"
n=$(kql 'http_logs | count' | jq -r .Count)
assert_eq "http_logs | count = 50" "50" "$n"

section "where + project"
out=$(kql 'http_logs | where status >= 500 | project status, message | take 1000' | wc -l | tr -d ' ')
# expected = count of status>=500 from python
expected=$(jq -s '[.[] | select(.status >= 500)] | length' /tmp/kql_data.ndjson)
assert_eq "where status >= 500 row count" "$expected" "$out"

section "summarize ... by ..."
# Compare our KQL result to ground truth derived from the source file.
our=$(kql 'http_logs | summarize count() by status' | jq -c -s 'sort_by(.status)')
truth=$(jq -c -s 'group_by(.status) | map({count: length, status: .[0].status}) | sort_by(.status)' /tmp/kql_data.ndjson)
if [[ "$our" == "$truth" ]]; then
    ok "summarize count() by status matches ground truth"
else
    f "summarize count() by status mismatch"
    printf "    our:   %s\n    truth: %s\n" "$our" "$truth"
fi

section "sort + take (top-N)"
first_n=$(kql 'http_logs | sort by timestamp asc | take 3 | project timestamp' \
    | jq -r .timestamp | tr '\n' '|')
expected_first='2026-04-19T10:00:00|2026-04-19T10:00:01|2026-04-19T10:00:02|'
assert_eq "first 3 timestamps ascending" "$expected_first" "$first_n"

section "string `contains`"
oom_rows=$(kql 'http_logs | where message contains "oom" | count' | jq -r .Count)
# Truth from python: only non-200 status with random<0.3 got "oom"
truth_oom=$(jq -s '[.[] | select(.message == "oom")] | length' /tmp/kql_data.ndjson)
assert_eq "contains 'oom' count" "$truth_oom" "$oom_rows"

section "string `startswith`"
api_rows=$(kql 'http_logs | where path startswith "/api" | count' | jq -r .Count)
truth_api=$(jq -s '[.[] | select(.path | startswith("/api"))] | length' /tmp/kql_data.ndjson)
assert_eq "startswith '/api' count" "$truth_api" "$api_rows"

section "extend + arithmetic"
# Class = status / 100 — should produce 2, 3, 4, 5 buckets
distinct_classes=$(kql 'http_logs | extend class = status / 100 | distinct class | sort by class asc' \
    | jq -r .class | tr '\n' ',' | sed 's/,$//')
assert_eq "class buckets = 2,3,4,5" "2,3,4,5" "$distinct_classes"

section "ago() function"
# ago(1h) — since our data is at 2026-04-19T10:00-ish and real `now()` is
# well after that, every row matches `where timestamp < ago(1h)` (in the past).
past_count=$(kql 'http_logs | where timestamp < ago(1h) | count' | jq -r .Count)
assert_eq "ago(1h) predicate captures all 50 past rows" "50" "$past_count"

section "KQL parse error returns 400"
status=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'http_logs | blat blat blat')
assert_eq "garbled KQL returns 400" "400" "$status"

section "Metrics: query_frontend counter"
if curl -s "$HTTP_BASE/metrics" | grep -q 'pensieve_query_frontend_total{lang="kql"}'; then
    ok "pensieve_query_frontend_total{lang=kql} exported"
else
    f "no kql query_frontend metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -40 "$LOG_FILE"; exit 1; fi
exit 0
