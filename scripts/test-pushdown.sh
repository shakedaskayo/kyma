#!/usr/bin/env bash
# Phase D.1: filter-pushdown E2E test.
#
# Creates 10 extents, each spanning a different hour. A query with a tight
# time window should scan far fewer extents than an unconstrained one.
# Verifies via the `pensieve_scan_extents_listed_total` counter delta.

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
LOG_FILE="/tmp/pensieve-pushdown.log"
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

section "Ingest 10 extents spanning 10 separate hours"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name pushdown \
    --schema 'timestamp:timestamp,n:int' >/dev/null
for h in 0 1 2 3 4 5 6 7 8 9; do
    hh=$(printf "2026-04-19T%02d" $((h+10)))
    payload="{\"timestamp\":\"${hh}:00:00Z\",\"n\":$h}
{\"timestamp\":\"${hh}:30:00Z\",\"n\":$((h*10+1))}"
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: pushdown' -H 'Content-Type: application/x-ndjson' \
        --data-binary "$payload" >/dev/null
done

live=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='pushdown')")
[[ "$live" == "10" ]] && ok "10 live extents ingested" || f "expected 10 live extents, got $live"

# Counter helper.
scan_count() {
    curl -s "$HTTP_BASE/metrics" | awk '/^pensieve_scan_extents_listed_total/{print $2}'
}

baseline=$(scan_count)
baseline=${baseline:-0}

section "Query WITHOUT filter (expect all 10 extents scanned)"
curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM pushdown' >/dev/null
after_full=$(scan_count)
delta_full=$((after_full - baseline))
[[ "$delta_full" == "10" ]] && ok "unfiltered query listed 10 extents" || f "unfiltered delta = $delta_full (want 10)"

section "Query WITH tight time-range filter (expect 1-2 extents)"
curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM pushdown WHERE timestamp BETWEEN TIMESTAMP '2026-04-19 12:00:00' AND TIMESTAMP '2026-04-19 12:59:59'" >/dev/null
after_tight=$(scan_count)
delta_tight=$((after_tight - after_full))
if (( delta_tight >= 1 && delta_tight <= 2 )); then
    ok "tight-range query listed only $delta_tight extent(s) (pushdown works)"
else
    f "tight-range query listed $delta_tight extents; expected 1-2"
fi

section "Query WITH upper bound only (expect ≤ half the extents)"
curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM pushdown WHERE timestamp < TIMESTAMP '2026-04-19 15:00:00'" >/dev/null
after_upper=$(scan_count)
delta_upper=$((after_upper - after_tight))
if (( delta_upper >= 4 && delta_upper <= 6 )); then
    ok "upper-bound query listed $delta_upper extents (hours 10-14)"
else
    f "upper-bound query listed $delta_upper extents; expected 4-6"
fi

section "Correctness: pushdown must still return the right rows"
full_n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM pushdown' | jq -r .n)
[[ "$full_n" == "20" ]] && ok "full COUNT = 20" || f "full COUNT = $full_n (want 20)"

tight_n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM pushdown WHERE timestamp BETWEEN TIMESTAMP '2026-04-19 12:00:00' AND TIMESTAMP '2026-04-19 12:59:59'" | jq -r .n)
[[ "$tight_n" == "2" ]] && ok "tight COUNT = 2 (one hour × 2 rows)" || f "tight COUNT = $tight_n (want 2)"

upper_n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM pushdown WHERE timestamp < TIMESTAMP '2026-04-19 15:00:00'" | jq -r .n)
[[ "$upper_n" == "10" ]] && ok "upper-bound COUNT = 10 (hours 10-14, 2 rows each)" || f "upper-bound COUNT = $upper_n (want 10)"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
