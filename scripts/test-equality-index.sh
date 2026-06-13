#!/usr/bin/env bash
# Equality-index pushdown E2E test.
#
# Proves per-extent distinct-value sets prune extents at the catalog level
# for `col = value` and `col IN (…)` predicates. Same shape as the time-range
# pushdown test, but for typed-column equality.

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
export KYMA_STAGING_DISABLED=1       # keep one-extent-per-request for deterministic extent count
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-eqidx.log"
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
cleanup() {
    if [[ -n "${SERVER_PID:-}" ]]; then
        kill -9 "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start kyma (staging disabled so each request = one extent)"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create table + ingest 10 extents, one region each"
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name events \
    --schema 'timestamp:timestamp,region:string,shard:int,message:string' >/dev/null

# 10 regions, each in its own extent (5 rows per extent).
REGIONS=(us-east us-west us-central eu-west eu-central apac-se apac-ne sa-east af-south me-east)
for idx in "${!REGIONS[@]}"; do
    region=${REGIONS[$idx]}
    shard=$idx
    ( for r in 1 2 3 4 5; do
        echo "{\"timestamp\":\"2026-04-20T10:$(printf '%02d' $idx):$(printf '%02d' $r)Z\",\"region\":\"$region\",\"shard\":$shard,\"message\":\"evt-$region-$r\"}"
      done ) > /tmp/eqidx-batch.ndjson
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: events' -H 'Content-Type: application/x-ndjson' \
        --data-binary @/tmp/eqidx-batch.ndjson > /dev/null
done

live=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='events')")
if [[ "$live" == "10" ]]; then
    ok "10 extents created, one per region"
else
    f "expected 10 extents, got $live"
fi

section "Verify column_stats populated with distinct sets"
sample_stats=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT column_stats::text FROM extents WHERE deleted_at IS NULL LIMIT 1")
if [[ "$sample_stats" == *'"distinct"'* ]] && [[ "$sample_stats" == *'"region"'* ]]; then
    ok "column_stats has region distinct set: $(echo $sample_stats | head -c 120)..."
else
    f "column_stats missing distinct sets: $sample_stats"
fi

# Counter helper
scan_count() {
    curl -s "$HTTP_BASE/metrics" | awk '/^kyma_scan_extents_listed_total/{print $2}'
}

baseline=$(scan_count)
baseline=${baseline:-0}

section "Query WITHOUT filter (expect 10 extents scanned)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM events' | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "50" ]] && ok "unfiltered COUNT(*) = 50" || f "unfiltered COUNT = $out"
[[ "$delta" == "10" ]] && ok "unfiltered listed all 10 extents" || f "unfiltered listed $delta"

section "Query WITH region = 'us-east' (expect 1 extent scanned)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events WHERE region = 'us-east'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "5" ]] && ok "region='us-east' COUNT = 5" || f "region COUNT = $out"
[[ "$delta" == "1" ]] && ok "equality filter listed exactly 1 extent (10x pruning)" \
                      || f "equality filter listed $delta (want 1)"

section "Query WITH shard = 7 (integer equality)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM events WHERE shard = 7' | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "5" ]] && ok "shard=7 COUNT = 5" || f "shard COUNT = $out"
[[ "$delta" == "1" ]] && ok "int equality listed exactly 1 extent" || f "int listed $delta"

section "Query WITH region IN ('us-east','eu-west') (expect 2 extents)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events WHERE region IN ('us-east','eu-west')" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "10" ]] && ok "IN (2 regions) COUNT = 10" || f "IN COUNT = $out"
[[ "$delta" == "2" ]] && ok "IN-set filter listed exactly 2 extents (5x pruning)" \
                      || f "IN-set listed $delta (want 2)"

section "Query with filter matching no extent"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events WHERE region = 'moon'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "0" ]] && ok "region='moon' COUNT = 0" || f "no-match COUNT = $out"
[[ "$delta" == "0" ]] && ok "no-match filter scanned zero extents (infinite-x pruning!)" \
                      || f "no-match scanned $delta (want 0)"

section "Combined filter: region + time-range"
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events
            WHERE region = 'us-east'
              AND timestamp BETWEEN TIMESTAMP '2026-04-20 10:00:00' AND TIMESTAMP '2026-04-20 10:00:10'" | jq -r .n)
[[ "$out" == "5" ]] && ok "combined equality+time-range returns 5 rows" || f "combined COUNT = $out"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
