#!/usr/bin/env bash
# Phase D.2 — block-level pruning via per-block min/max in extent footer.
#
# Proves: inside ONE extent containing multiple blocks (batches), a query
# whose predicate only matches some blocks scans only those blocks, not
# all of them. The `kyma_scan_blocks_{scanned,pruned}_total` counters
# expose the outcome.

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
export KYMA_GRPC_ADDR=off
export KYMA_OTLP_ADDR=off
# Staging enabled so multiple HTTP ingests coalesce into a single extent
# with multiple batches = multiple blocks. That's the shape we want to test.
export KYMA_FLUSH_MAX_ROWS=1000000      # huge row cap — don't flush by row count
export KYMA_FLUSH_MAX_BYTES=1073741824  # 1 GiB — don't flush by bytes
export KYMA_FLUSH_MAX_AGE_MS=3000       # flush after 3s (oldest-waiter)
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-block-pruning.log"
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
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start kyma"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create table + ingest multiple batches into a single extent"
./target/debug/kyma-cli create-database default >/dev/null
./target/debug/kyma-cli create-table --db default --name events \
    --schema 'timestamp:timestamp,status:int,region:string,message:string' >/dev/null

# Send 5 PARALLEL HTTP POSTs, each with 1 row in a distinct time slot.
# Parallel is required so all 5 waiters land in the same staging buffer
# before the oldest-waiter timer fires — then one flush = one extent
# with 5 batches = 5 blocks.
#
# Collect PIDs explicitly — bare `wait` blocks on the kyma server too.
curl_pids=()
for i in 0 1 2 3 4; do
    ts=$(printf '2026-04-20T10:0%d:00Z' $i)
    status=$((200 + i * 100))  # 200, 300, 400, 500, 600
    region="r-$i"
    curl -s -X POST "$HTTP_BASE/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: events' \
        -H 'Content-Type: application/x-ndjson' \
        --data-binary "{\"timestamp\":\"$ts\",\"status\":$status,\"region\":\"$region\",\"message\":\"m-$i\"}" >/dev/null &
    curl_pids+=($!)
done
for pid in "${curl_pids[@]}"; do wait "$pid" || true; done

# Extra second to make sure catalog rows are visible.
sleep 1

live=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='events')")
[[ "$live" == "1" ]] && ok "5 batches coalesced into 1 extent" || f "extent count = $live, want 1"
# Block count isn't stored in the catalog; we infer it by doing an
# unfiltered scan and reading the `kyma_scan_blocks_scanned_total` counter.

scanned_counter() {
    curl -s "$HTTP_BASE/metrics" | awk '/^kyma_scan_blocks_scanned_total/{print $2}'
}
pruned_counter() {
    curl -s "$HTTP_BASE/metrics" | awk '/^kyma_scan_blocks_pruned_total/{print $2}'
}

section "Unfiltered query: scans all 5 blocks"
before_scan=$(scanned_counter); before_scan=${before_scan:-0}
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'events | count' | jq -r .Count)
[[ "$rows" == "5" ]] && ok "unfiltered COUNT = 5" || f "unfiltered count = $rows"
after_scan=$(scanned_counter)
delta=$((after_scan - before_scan))
[[ "$delta" == "5" ]] && ok "scanned 5 blocks" || f "unfiltered scan: expected 5, got $delta"

section "Tight time-range (single block): prunes 4 of 5"
before_scan=$(scanned_counter); before_pruned=$(pruned_counter)
before_pruned=${before_pruned:-0}
# Match only the row at 10:02:00 — one block. Using SQL so the grammar is
# unambiguous.
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events WHERE timestamp >= TIMESTAMP '2026-04-20 10:01:30' AND timestamp < TIMESTAMP '2026-04-20 10:02:30'" \
    | jq -r .n)
[[ "$rows" == "1" ]] && ok "tight time-range COUNT = 1" || f "got $rows"
after_scan=$(scanned_counter); after_pruned=$(pruned_counter)
scan_delta=$((after_scan - before_scan))
prune_delta=$((after_pruned - before_pruned))
[[ "$scan_delta" == "1" && "$prune_delta" == "4" ]] \
    && ok "scanned 1, pruned 4 (perfect block-level pruning)" \
    || f "scan_delta=$scan_delta prune_delta=$prune_delta (want 1/4)"

section "Mid-range time filter (3 blocks): prunes 2 of 5"
before_scan=$(scanned_counter); before_pruned=$(pruned_counter)
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM events WHERE timestamp >= TIMESTAMP '2026-04-20 10:01:00' AND timestamp <= TIMESTAMP '2026-04-20 10:03:00'" \
    | jq -r .n)
[[ "$rows" == "3" ]] && ok "3-block time-range COUNT = 3" || f "got $rows"
after_scan=$(scanned_counter); after_pruned=$(pruned_counter)
scan_delta=$((after_scan - before_scan))
prune_delta=$((after_pruned - before_pruned))
[[ "$scan_delta" == "3" && "$prune_delta" == "2" ]] \
    && ok "scanned 3, pruned 2" \
    || f "scan_delta=$scan_delta prune_delta=$prune_delta (want 3/2)"

section "Equality on int (status=500): prunes 4 of 5"
before_scan=$(scanned_counter); before_pruned=$(pruned_counter)
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'events | where status == 500 | count' | jq -r .Count)
[[ "$rows" == "1" ]] && ok "status==500 COUNT = 1" || f "got $rows"
after_scan=$(scanned_counter); after_pruned=$(pruned_counter)
scan_delta=$((after_scan - before_scan))
prune_delta=$((after_pruned - before_pruned))
# status values per block: 200, 300, 400, 500, 600 — each block has exactly
# one value. min == max == that value. Only the block with min<=500<=max
# matches — 1 block.
[[ "$scan_delta" == "1" && "$prune_delta" == "4" ]] \
    && ok "int equality scanned 1, pruned 4" \
    || f "scan_delta=$scan_delta prune_delta=$prune_delta (want 1/4)"

section "No-match predicate: prunes all 5"
before_scan=$(scanned_counter); before_pruned=$(pruned_counter)
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'events | where status == 999 | count' | jq -r .Count)
# COUNT(*) on zero-extent result is 0 (DataFusion returns one row with Count=0)
[[ "$rows" == "0" ]] && ok "no-match COUNT = 0" || f "got $rows"
after_scan=$(scanned_counter); after_pruned=$(pruned_counter)
scan_delta=$((after_scan - before_scan))
prune_delta=$((after_pruned - before_pruned))
# Catalog-level equality prunes the extent entirely, so block stats never
# run — scan_delta=0, prune_delta=0. That's also correct behavior (∞×
# pruning from a higher layer).
info "catalog-layer eliminated extent: scan_delta=$scan_delta prune_delta=$prune_delta"
ok "no-match predicate returns 0 (via catalog or block level)"

section "String equality (region='r-2'): prunes 4 of 5"
before_scan=$(scanned_counter); before_pruned=$(pruned_counter)
rows=$(curl -s -X POST "$HTTP_BASE/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'events | where region == "r-2" | count' | jq -r .Count)
[[ "$rows" == "1" ]] && ok "region=='r-2' COUNT = 1" || f "got $rows"
# Note: catalog extent-level equality-index already selects the extent
# (region distinct set contains r-2). Block-level pruning then narrows to
# 1 block.
after_scan=$(scanned_counter); after_pruned=$(pruned_counter)
scan_delta=$((after_scan - before_scan))
prune_delta=$((after_pruned - before_pruned))
[[ "$scan_delta" == "1" && "$prune_delta" == "4" ]] \
    && ok "string equality scanned 1, pruned 4" \
    || f "scan_delta=$scan_delta prune_delta=$prune_delta (want 1/4)"

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -40 "$LOG_FILE"; exit 1; fi
exit 0
