#!/usr/bin/env bash
# End-to-end smoke test for the kyma phase-A vertical slice.
#
# Runs a sequence of ingest + query + catalog operations against a live
# server backed by the docker-compose Postgres + MinIO stack, and asserts
# each result programmatically. Exits non-zero on first failure.
#
# Prereqs (auto-bootstrapped where possible):
#   - docker + docker-compose
#   - jq, curl
#   - `cargo build -p kyma-bin -p kyma-cli` already run
#   - docker-compose up (the script brings it up if not already)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ------------------------------------------------------------------
# Config
# ------------------------------------------------------------------
export KYMA_CATALOG_URL="postgres://kyma:kyma_dev@localhost:5433/kyma"
export KYMA_S3_ENDPOINT="http://localhost:9000"
export KYMA_S3_BUCKET="kyma"
export KYMA_S3_ACCESS_KEY_ID="kyma_admin"
export KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev"
export KYMA_S3_PATH_STYLE="true"
export KYMA_S3_ALLOW_HTTP="true"
export KYMA_HTTP_ADDR="127.0.0.1:8080"
export KYMA_SELF_TRACE="off"   # deterministic storage-layout assertions
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-e2e.log"
SERVER_PID=""

# ------------------------------------------------------------------
# Pretty output
# ------------------------------------------------------------------
if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; YLW="\033[33m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
    RED=""; GRN=""; YLW=""; BLU=""; DIM=""; NC=""
fi

pass_count=0
fail_count=0

section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()      { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass_count=$((pass_count+1)); }
fail()    { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail_count=$((fail_count+1)); }
info()    { printf "  ${DIM}%s${NC}\n" "$*"; }

assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
        ok "$name"
    else
        fail "$name"
        printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" "$expected" "$actual"
    fi
}

# ------------------------------------------------------------------
# Cleanup on exit
# ------------------------------------------------------------------
cleanup() {
    if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# ------------------------------------------------------------------
# Stack bring-up / reset
# ------------------------------------------------------------------
section "Bring up Postgres + MinIO"
docker-compose up -d --quiet-pull >/dev/null 2>&1

for i in 1 2 3 4 5 6 7 8 9 10; do
    if docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1 &&
       docker exec kyma-minio mc ready local >/dev/null 2>&1; then
        break
    fi
    sleep 2
done
docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null
info "postgres + minio healthy"

section "Reset state (truncate catalog, empty bucket)"
docker exec kyma-postgres psql -U kyma -d kyma -qc "
    TRUNCATE TABLE extents, manifests, snapshots, schema_snapshots, tables, databases, nodes RESTART IDENTITY CASCADE;
" >/dev/null 2>&1 || true
# mc alias is persistent within the container, but `mc alias set` is idempotent.
docker exec kyma-minio mc alias set local http://localhost:9000 kyma_admin kyma_admin_dev >/dev/null 2>&1 || true
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
info "catalog truncated; bucket cleared"

# ------------------------------------------------------------------
# Start kyma
# ------------------------------------------------------------------
section "Start kyma server"
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
info "PID $SERVER_PID; log $LOG_FILE"
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -o /dev/null -w "%{http_code}" "$HTTP_BASE/health" 2>/dev/null | grep -q 200; then
        break
    fi
    sleep 1
done
health=$(curl -s "$HTTP_BASE/health")
assert_eq "server /health returns status=ok" '"ok"' "$(echo "$health" | jq .status)"

# ------------------------------------------------------------------
# Test 1: Bootstrap — create database + two tables
# ------------------------------------------------------------------
section "Test 1 — bootstrap: create database + tables"
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name http_logs   --schema 'timestamp:timestamp,status:int,path:string,message:string' >/dev/null
./target/debug/kyma-cli create-table --db default --name user_events --schema 'timestamp:timestamp,user_id:long,event:string' >/dev/null
# Filter out server-internal tables (e.g. otel_traces, pre-created in `default`
# for self-tracing) — this test asserts the two tables IT created, not the
# bootstrap set.
tables_listing=$(./target/debug/kyma-cli list-tables --db default | awk '{print $1}' \
    | grep -E '^(http_logs|user_events)$' | sort | tr '\n' ',' | sed 's/,$//')
assert_eq "two tables created"      "http_logs,user_events" "$tables_listing"

# ------------------------------------------------------------------
# Test 2 — 100-row ingest to http_logs, assert response + extent row
# ------------------------------------------------------------------
section "Test 2 — 100-row ingest to http_logs"
python3 - <<'PY' > /tmp/http_logs_100.ndjson
import json, random
random.seed(42)
paths = ["/", "/api/users", "/api/orders", "/admin", "/healthz", "/api/search"]
for i in range(100):
    p  = random.choice(paths)
    st = random.choices([200, 301, 404, 500, 502], weights=[70, 5, 10, 10, 5])[0]
    msg = "ok" if st == 200 else ("redirect" if st == 301 else "err")
    ts = f"2026-04-19T10:00:{i % 60:02d}.{(i // 60) * 100_000_000:09d}Z" if False else f"2026-04-19T10:{i // 60:02d}:{i % 60:02d}Z"
    print(json.dumps({"timestamp": ts, "status": st, "path": p, "message": msg}))
PY

ingest1=$(curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: http_logs' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/http_logs_100.ndjson)
assert_eq "100-row ingest rows_ingested = 100" "100" "$(echo "$ingest1" | jq -r .rows_ingested)"
assert_eq "100-row ingest extent_count = 1"    "1"   "$(echo "$ingest1" | jq -r .extent_count)"

# ------------------------------------------------------------------
# Test 3 — a second ingest batch (20 rows) so we exercise multi-extent reads
# ------------------------------------------------------------------
section "Test 3 — second ingest batch (20 rows)"
python3 - <<'PY' > /tmp/http_logs_20.ndjson
import json
for i in range(20):
    print(json.dumps({
        "timestamp": f"2026-04-19T11:00:{i:02d}Z",
        "status": 500 if i % 2 == 0 else 200,
        "path": "/api/search",
        "message": "chaos" if i % 2 == 0 else "ok"
    }))
PY
ingest2=$(curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: http_logs' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/http_logs_20.ndjson)
assert_eq "second ingest rows_ingested = 20" "20" "$(echo "$ingest2" | jq -r .rows_ingested)"

# ------------------------------------------------------------------
# Test 4 — 50-row ingest to user_events (to prove per-table isolation)
# ------------------------------------------------------------------
section "Test 4 — ingest to a different table (user_events)"
python3 - <<'PY' > /tmp/user_events_50.ndjson
import json
evs = ["login", "view", "click", "logout"]
for i in range(50):
    print(json.dumps({
        "timestamp": f"2026-04-19T12:00:{i % 60:02d}Z",
        "user_id": 1000 + (i % 10),
        "event": evs[i % 4]
    }))
PY
ingest3=$(curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: user_events' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/user_events_50.ndjson)
assert_eq "user_events ingest = 50 rows" "50" "$(echo "$ingest3" | jq -r .rows_ingested)"

# ------------------------------------------------------------------
# Query helpers
# ------------------------------------------------------------------
query() {
    curl -s -X POST "$HTTP_BASE/v1/query" \
        -H 'X-Database: default' -H 'Content-Type: application/sql' \
        --data "$1"
}

# ------------------------------------------------------------------
# Test 5 — SELECT COUNT(*) across both ingest batches
# ------------------------------------------------------------------
section "Test 5 — multi-extent COUNT(*) on http_logs"
total=$(query 'SELECT COUNT(*) AS n FROM http_logs' | jq -r .n)
assert_eq "http_logs row count = 120 (100 + 20)" "120" "$total"

# ------------------------------------------------------------------
# Test 6 — WHERE filter
# ------------------------------------------------------------------
section "Test 6 — WHERE clause filtering"
errors=$(query 'SELECT COUNT(*) AS n FROM http_logs WHERE status >= 500' | jq -r .n)
# 100-row batch: expected number of 500/502s (seed=42). 20-row batch: 10 errors.
# Compute expected via a quick repeat of the generator logic in awk not possible;
# use jq over the file instead.
expected_errors_first=$(jq -s '[.[] | select(.status >= 500)] | length' /tmp/http_logs_100.ndjson)
expected_errors_total=$((expected_errors_first + 10))
assert_eq "error rows (status >= 500) = $expected_errors_total" "$expected_errors_total" "$errors"

# ------------------------------------------------------------------
# Test 7 — GROUP BY + aggregation
# ------------------------------------------------------------------
section "Test 7 — GROUP BY status, counts match"
agg=$(query 'SELECT status, COUNT(*) AS n FROM http_logs GROUP BY status ORDER BY status')
# Re-derive expected distribution from the source files.
expected_dist=$(cat /tmp/http_logs_100.ndjson /tmp/http_logs_20.ndjson | \
    jq -c -s 'group_by(.status) | map({status: .[0].status, n: length}) | sort_by(.status)' | \
    jq -c '.[]')
actual_dist=$(echo "$agg" | jq -c '{status, n}')
if [[ "$(echo "$expected_dist" | tr -d '\n')" == "$(echo "$actual_dist" | tr -d '\n')" ]]; then
    ok "aggregation matches ground truth"
    info "distribution: $(echo "$actual_dist" | tr '\n' ' ')"
else
    fail "aggregation mismatch"
    printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" \
        "$(echo "$expected_dist" | tr '\n' ' ')" "$(echo "$actual_dist" | tr '\n' ' ')"
fi

# ------------------------------------------------------------------
# Test 8 — ORDER BY + LIMIT
# ------------------------------------------------------------------
section "Test 8 — ORDER BY timestamp ASC LIMIT 3"
first3=$(query 'SELECT timestamp, status FROM http_logs ORDER BY timestamp ASC LIMIT 3' | jq -c .timestamp | tr '\n' ' ')
expected_first3='"2026-04-19T10:00:00" "2026-04-19T10:00:01" "2026-04-19T10:00:02" '
assert_eq "first 3 timestamps asc" "$expected_first3" "$first3"

# ------------------------------------------------------------------
# Test 9 — cross-extent DISTINCT
# ------------------------------------------------------------------
section "Test 9 — DISTINCT path across extents"
distinct_paths=$(query 'SELECT DISTINCT path FROM http_logs ORDER BY path' | jq -r .path | tr '\n' ',' | sed 's/,$//')
# Source had 6 paths; the 20-row batch only used "/api/search" (already present), so 6.
assert_eq "6 distinct paths" "/,/admin,/api/orders,/api/search,/api/users,/healthz" "$distinct_paths"

# ------------------------------------------------------------------
# Test 10 — per-table isolation: user_events has its own schema
# ------------------------------------------------------------------
section "Test 10 — user_events separate from http_logs"
ue_count=$(query 'SELECT COUNT(*) AS n FROM user_events' | jq -r .n)
assert_eq "user_events count = 50" "50" "$ue_count"
ue_users=$(query 'SELECT COUNT(DISTINCT user_id) AS n FROM user_events' | jq -r .n)
assert_eq "user_events distinct user_id = 10" "10" "$ue_users"

# ------------------------------------------------------------------
# Test 11 — JOIN across tables
# ------------------------------------------------------------------
section "Test 11 — JOIN http_logs and user_events on timestamp minute"
# Cast timestamps to date_trunc('minute', ...). DataFusion supports this.
joined=$(query "
    SELECT COUNT(*) AS n
    FROM http_logs h
    JOIN user_events u
      ON date_trunc('minute', h.timestamp) = date_trunc('minute', u.timestamp)
" | jq -r .n)
# http_logs: 10:00-11:19 window. user_events: all at 12:00:xx. Expected join = 0.
assert_eq "disjoint-window join = 0 rows" "0" "$joined"

# ------------------------------------------------------------------
# Test 12 — bad ingest body returns 400
# ------------------------------------------------------------------
section "Test 12 — error handling: malformed JSON => 400"
http_code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: http_logs' -H 'Content-Type: application/x-ndjson' \
    --data-binary 'this is not json')
assert_eq "malformed ingest returns 400" "400" "$http_code"

# ------------------------------------------------------------------
# Test 13 — missing X-Table header returns 400
# ------------------------------------------------------------------
section "Test 13 — missing X-Table header => 400"
http_code=$(curl -s -o /dev/null -w '%{http_code}' -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'Content-Type: application/x-ndjson' \
    --data-binary '{"timestamp":"2026-04-19T10:00:00Z","status":200,"path":"/","message":"ok"}')
assert_eq "missing X-Table returns 400" "400" "$http_code"

# ------------------------------------------------------------------
# Test 14 — durability across server restart
# ------------------------------------------------------------------
section "Test 14 — durability: kill server, restart, data still queryable"
kill "$SERVER_PID"
wait "$SERVER_PID" 2>/dev/null || true
info "server stopped; objects remain in MinIO, metadata in Postgres"
./target/debug/kyma >>"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -o /dev/null -w "%{http_code}" "$HTTP_BASE/health" 2>/dev/null | grep -q 200; then
        break
    fi
    sleep 1
done
post_restart_count=$(query 'SELECT COUNT(*) AS n FROM http_logs' | jq -r .n)
assert_eq "row count survives restart = 120" "120" "$post_restart_count"

# ------------------------------------------------------------------
# Test 15 — catalog state checks (snapshot chain + extent count)
# ------------------------------------------------------------------
section "Test 15 — catalog consistency checks"
snapshot_count=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM snapshots WHERE table_id = (SELECT id FROM tables WHERE name='http_logs')")
assert_eq "http_logs has 3 snapshots (bootstrap + 2 ingests)" "3" "$snapshot_count"

extent_count=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE table_id = (SELECT id FROM tables WHERE name='http_logs') AND deleted_at IS NULL")
assert_eq "http_logs has 2 live extents" "2" "$extent_count"

minio_object_count=$(docker exec kyma-minio mc ls -r local/kyma/ | grep -c '\.kyma$' || true)
# 2 http_logs extents + 1 user_events extent = 3 objects
assert_eq "MinIO contains 3 extent objects" "3" "$minio_object_count"

# ------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------
section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass_count" "$fail_count"
if [[ $fail_count -gt 0 ]]; then
    printf "\n${YLW}Server log tail:${NC}\n"
    tail -25 "$LOG_FILE"
    exit 1
fi
exit 0
