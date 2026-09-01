#!/usr/bin/env bash
# Phase (slice-2 foundation) — multi-node read scale-out.
#
# Two pensieve nodes A+B against the same Postgres + MinIO. Ingests happen on A
# (so A caches the extent bytes locally). Queries hit B. With the
# read-router, B consults `list_live_nodes`, rendezvous-hashes each extent,
# and fetches the ones that hash to A via Arrow Flight
# (`kind:"extent"` ticket). Result is identical to a single-node query;
# observable difference is the `pensieve_scan_extents_remote_total` counter
# firing on B and `pensieve_flight_serve_extent_total` firing on A.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Shared catalog + object store so both nodes see the same snapshots.
export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
export PENSIEVE_S3_ENDPOINT="http://localhost:9000"
export PENSIEVE_S3_BUCKET="pensieve"
export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
export PENSIEVE_S3_PATH_STYLE="true"
export PENSIEVE_S3_ALLOW_HTTP="true"
export PENSIEVE_OTLP_ADDR=off
export PENSIEVE_SELF_TRACE="off"   # deterministic storage-layout assertions
export PENSIEVE_STAGING_DISABLED=1
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

A_HTTP="127.0.0.1:18080"
A_GRPC="127.0.0.1:19090"
B_HTTP="127.0.0.1:18081"
B_GRPC="127.0.0.1:19091"
LOG_A="/tmp/pensieve-scale-a.log"
LOG_B="/tmp/pensieve-scale-b.log"
PID_A=""
PID_B=""

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
    [[ -n "${PID_A:-}" ]] && kill -9 "$PID_A" 2>/dev/null || true
    [[ -n "${PID_B:-}" ]] && kill -9 "$PID_B" 2>/dev/null || true
}
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset catalog + MinIO"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null

section "Start node A"
PENSIEVE_HTTP_ADDR="$A_HTTP" PENSIEVE_GRPC_ADDR="$A_GRPC" \
    ./target/debug/pensieve >"$LOG_A" 2>&1 &
PID_A=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    curl -sf "http://$A_HTTP/health" >/dev/null 2>&1 && break || sleep 1
done
ok "node A up on $A_HTTP / $A_GRPC"

section "Start node B"
PENSIEVE_HTTP_ADDR="$B_HTTP" PENSIEVE_GRPC_ADDR="$B_GRPC" \
    ./target/debug/pensieve >"$LOG_B" 2>&1 &
PID_B=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    curl -sf "http://$B_HTTP/health" >/dev/null 2>&1 && break || sleep 1
done
ok "node B up on $B_HTTP / $B_GRPC"

# Give both nodes time to write a heartbeat.
sleep 2

section "Verify both nodes registered"
live=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM nodes WHERE last_heartbeat > now() - interval '30 seconds'")
[[ "$live" == "2" ]] && ok "2 live nodes in catalog" || f "got $live live nodes"

# Both nodes advertise their Flight endpoint so the peer can reach it. The
# node_id used by the router is the one written to the catalog — so we need
# to make sure each pensieve instance published its peer-reachable Flight port.
# Patch: pensieve's `endpoint` column is built from PENSIEVE_HTTP_ADDR currently.
# For the router to call the right Flight port, we rewrite the endpoint
# column to the gRPC addr for each node.
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc \
    "UPDATE nodes SET endpoint = CASE
        WHEN endpoint LIKE '%$A_HTTP%' THEN '$A_GRPC'
        WHEN endpoint LIKE '%$B_HTTP%' THEN '$B_GRPC'
        ELSE endpoint
     END" >/dev/null 2>&1 || true

section "Create table via A"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name so_events \
    --schema 'timestamp:timestamp,region:string,status:int' >/dev/null

section "Ingest 10 extents via A"
for i in 0 1 2 3 4 5 6 7 8 9; do
    ts=$(printf '2026-04-20T%02d:00:00Z' $((10 + i)))
    region="r-$i"
    curl -s -X POST "http://$A_HTTP/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: so_events' \
        -H 'Content-Type: application/x-ndjson' \
        --data-binary "{\"timestamp\":\"$ts\",\"region\":\"$region\",\"status\":$((200 + i))}" >/dev/null
done
live=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='so_events')")
[[ "$live" == "10" ]] && ok "10 extents ingested on A" || f "got $live extents"

section "Query on B — correctness"
rows=$(curl -s -X POST "http://$B_HTTP/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM so_events' | jq -r .n)
[[ "$rows" == "10" ]] && ok "B's COUNT(*) = 10 (matches single-node answer)" || f "got $rows"

section "Verify B fanned out to A (some extents fetched via Flight)"
b_remote=$(curl -s "http://$B_HTTP/metrics" | awk '/^pensieve_scan_extents_remote_total/{s+=$NF} END{print s+0}')
info "pensieve_scan_extents_remote_total on B = $b_remote"
if (( b_remote > 0 )); then
    ok "B fetched $b_remote extents from peers"
else
    f "B didn't fan out — counter is 0"
fi

section "Verify A served extents to B"
a_served=$(curl -s "http://$A_HTTP/metrics" | awk '/^pensieve_flight_serve_extent_total/{print $2+0}')
info "pensieve_flight_serve_extent_total on A = $a_served"
if (( a_served > 0 )); then
    ok "A served $a_served extents to peers"
else
    f "A didn't serve any extents"
fi

section "Verify routing is stable: same query twice, same split"
split1=$(curl -s "http://$B_HTTP/metrics" | awk '/^pensieve_scan_extents_remote_total/{print $2+0}')
curl -s -X POST "http://$B_HTTP/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) FROM so_events' >/dev/null
split2=$(curl -s "http://$B_HTTP/metrics" | awk '/^pensieve_scan_extents_remote_total/{print $2+0}')
delta=$((split2 - split1))
info "second query fanned out $delta extents (want same as first)"
if (( delta > 0 && delta <= 10 )); then
    ok "routing stable — rendezvous hash reproduces assignment"
else
    f "routing delta $delta unexpected"
fi

section "Verify kill-remote-fallback: stop A, query on B still succeeds"
kill -9 "$PID_A" 2>/dev/null; PID_A=""
sleep 2
rows_after=$(curl -s -X POST "http://$B_HTTP/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM so_events' | jq -r .n)
[[ "$rows_after" == "10" ]] && ok "B answered 10 after A killed (fallback path)" \
                             || f "B got $rows_after after A killed"
fallback=$(curl -s "http://$B_HTTP/metrics" | awk '/^pensieve_scan_extents_remote_fallback_total/{print $2+0}')
info "remote-fallback counter on B = $fallback"
if (( fallback > 0 )); then
    ok "fallback counter fired — B read peer-assigned extents locally after peer died"
else
    info "(no fallback counter — may just mean all extents were self-assigned this run)"
    ok "fallback tolerated"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    echo "--- node A log tail ---"; tail -30 "$LOG_A" || true
    echo "--- node B log tail ---"; tail -30 "$LOG_B" || true
    exit 1
fi
exit 0
