#!/usr/bin/env bash
# Text-search (token) pruning E2E test.
#
# Ingests 10 extents with distinctive words. A `LIKE '%word%'` query
# should prune extents that never mention that word. Verified via
# `kyma_scan_extents_listed_total` counter delta.

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
export KYMA_STAGING_DISABLED=1
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-textidx.log"
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

section "Reset + start"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create logs table + ingest 10 extents with distinctive tokens"
./target/debug/kyma-cli create-database default >/dev/null
./target/debug/kyma-cli create-table --db default --name logs \
    --schema 'timestamp:timestamp,level:string,message:string' >/dev/null

# Pick one unique word per extent; 10 extents total.
UNIQUE=(alpha beta gamma delta epsilon zeta eta theta iota kappa)
for idx in "${!UNIQUE[@]}"; do
    word=${UNIQUE[$idx]}
    cat > /tmp/textidx-batch.ndjson <<PAYLOAD
{"timestamp":"2026-04-20T10:$(printf %02d $idx):00Z","level":"INFO","message":"starting request handler for endpoint"}
{"timestamp":"2026-04-20T10:$(printf %02d $idx):01Z","level":"INFO","message":"unique-per-extent-marker $word happened here"}
{"timestamp":"2026-04-20T10:$(printf %02d $idx):02Z","level":"WARN","message":"common retry message seen everywhere"}
PAYLOAD
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: logs' -H 'Content-Type: application/x-ndjson' \
        --data-binary @/tmp/textidx-batch.ndjson > /dev/null
done

live=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='logs')")
[[ "$live" == "10" ]] && ok "10 extents ingested" || f "got $live extents"

section "Verify column_stats now has tokens"
sample=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT column_stats->'message'->'tokens'::text FROM extents WHERE deleted_at IS NULL LIMIT 1")
if [[ "$sample" == *'alpha'* ]] || [[ "$sample" == *'beta'* ]] || [[ "$sample" == *'gamma'* ]] \
   || [[ "$sample" == *'delta'* ]] || [[ "$sample" == *'epsilon'* ]] || [[ "$sample" == *'zeta'* ]] \
   || [[ "$sample" == *'eta'* ]] || [[ "$sample" == *'theta'* ]] || [[ "$sample" == *'iota'* ]] \
   || [[ "$sample" == *'kappa'* ]]; then
    ok "column_stats.message.tokens populated"
    printf "  ${DIM}sample: %s${NC}\n" "$(echo $sample | head -c 120)"
else
    f "tokens missing: $sample"
fi

scan_count() {
    curl -s "$HTTP_BASE/metrics" | awk '/^kyma_scan_extents_listed_total/{print $2}'
}

section "Query with unique word pruning (SQL LIKE)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM logs WHERE message LIKE '%alpha%'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "1" ]] && ok "LIKE '%alpha%' COUNT = 1" || f "COUNT = $out"
[[ "$delta" == "1" ]] && ok "scanned exactly 1 extent (10x pruning)" \
                    || f "scanned $delta extents (want 1)"

section "Query with non-existent word → zero extents"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM logs WHERE message LIKE '%xylophone%'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "0" ]] && ok "non-existent word COUNT = 0" || f "COUNT = $out"
[[ "$delta" == "0" ]] && ok "scanned 0 extents (∞× pruning)" \
                    || f "scanned $delta (want 0)"

section "Query via KQL: message contains 'gamma'"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'logs | where message contains "gamma" | count' | jq -r .Count)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "1" ]] && ok "KQL contains 'gamma' Count = 1" || f "Count = $out"
[[ "$delta" == "1" ]] && ok "KQL contains routed through token index" \
                    || f "KQL scanned $delta"

section "Common word hits all extents (no pruning)"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM logs WHERE message LIKE '%retry%'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "10" ]] && ok "common word 'retry' COUNT = 10" || f "COUNT = $out"
[[ "$delta" == "10" ]] && ok "common word scanned all 10 extents (correct)" \
                    || f "common word scanned $delta"

section "Multi-token phrase: both tokens must be in extent"
# 'request handler' appears in EVERY extent's first row.
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM logs WHERE message LIKE '%request handler%'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
[[ "$out" == "10" ]] && ok "'request handler' COUNT = 10" || f "COUNT = $out"
[[ "$delta" == "10" ]] && ok "multi-token phrase scans all matching extents" \
                    || f "scanned $delta"

# And one where only SOME extents have both tokens.
section "Multi-token phrase where one token is unique + one common"
before=$(scan_count); before=${before:-0}
out=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM logs WHERE message LIKE '%alpha%retry%'" | jq -r .n)
after=$(scan_count); delta=$((after - before))
# The 'alpha' extent has both 'alpha' and 'retry'; row-level filter
# requires 'alpha' THEN 'retry' which is false (they're on different
# rows). So COUNT = 0. But pruning should still scan just the alpha extent.
[[ "$out" == "0" ]] && ok "multi-token phrase row-level COUNT = 0 (correct; different rows)" \
                    || f "COUNT = $out"
[[ "$delta" == "1" ]] && ok "pruned to the 1 extent with BOTH tokens" \
                    || f "scanned $delta (want 1)"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -40 "$LOG_FILE"; exit 1; fi
exit 0
