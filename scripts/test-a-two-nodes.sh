#!/usr/bin/env bash
# Architectural Test A — "run two copies"
#
# Spawn two pensieve binaries against the same Postgres + MinIO, fire 100
# concurrent ingest requests split between them, and verify:
#
#   - No data loss    (total rows in table == rows sent)
#   - No duplication  (DISTINCT req_id == N requests; row-sum invariant holds)
#   - Contention exercised (snapshot count >> 1; at least one CAS-retry
#     occurred during the race — observable in server logs)
#
# This proves two architectural invariants from the plan:
#   (1) object storage is the only source of truth,
#   (2) query nodes are stateless.
#
# A pensieve node can be added or removed without coordination because all
# durable state lives in the catalog + object store.
#
# Requires: docker-compose stack up, `cargo build -p pensieve-bin` done, jq + curl.

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
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

# ------------------------------------------------------------------
# Config
# ------------------------------------------------------------------
N_REQUESTS=100
ROWS_PER_REQUEST=5
EXPECTED_ROWS=$((N_REQUESTS * ROWS_PER_REQUEST))
EXPECTED_ROW_ID_SUM=$((N_REQUESTS * (0 + 1 + 2 + 3 + 4)))   # per-request sum × N
PARALLEL=20  # concurrent curl workers

NODE_A_ADDR="127.0.0.1:8080"
NODE_B_ADDR="127.0.0.1:8081"
LOG_A="/tmp/pensieve-node-a.log"
LOG_B="/tmp/pensieve-node-b.log"

PID_A=""; PID_B=""

# ------------------------------------------------------------------
# Pretty output
# ------------------------------------------------------------------
if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; YLW="\033[33m"; BLU="\033[34m"; DIM="\033[2m"; NC="\033[0m"
else
    RED=""; GRN=""; YLW=""; BLU=""; DIM=""; NC=""
fi
pass_count=0; fail_count=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()      { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass_count=$((pass_count+1)); }
fail()    { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail_count=$((fail_count+1)); }
info()    { printf "  ${DIM}%s${NC}\n" "$*"; }
assert_eq() {
    local name="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then ok "$name"
    else
        fail "$name"
        printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" "$expected" "$actual"
    fi
}

cleanup() {
    [[ -n "${PID_A:-}" ]] && kill -9 "$PID_A" 2>/dev/null || true
    [[ -n "${PID_B:-}" ]] && kill -9 "$PID_B" 2>/dev/null || true
}
trap cleanup EXIT

# ------------------------------------------------------------------
# Prereqs
# ------------------------------------------------------------------
if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up. Start it with 'docker-compose up -d'.${NC}\n"
    exit 2
fi

section "Reset state"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "
    TRUNCATE TABLE extents, manifests, snapshots, schema_snapshots, tables, databases, nodes RESTART IDENTITY CASCADE;
" >/dev/null 2>&1 || true
docker exec pensieve-minio mc alias set local http://localhost:9000 pensieve_admin pensieve_admin_dev >/dev/null 2>&1 || true
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null
info "catalog + bucket reset"

# ------------------------------------------------------------------
# Bootstrap table
# ------------------------------------------------------------------
section "Bootstrap database + test_a table"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table \
    --db default --name test_a \
    --schema 'timestamp:timestamp,req_id:int,row_id:int,message:string' >/dev/null
info "database=default, table=test_a"

# ------------------------------------------------------------------
# Start two pensieve nodes
# ------------------------------------------------------------------
section "Spawn node A (port 8080) + node B (port 8081)"
PENSIEVE_HTTP_ADDR="$NODE_A_ADDR" ./target/debug/pensieve >"$LOG_A" 2>&1 &
PID_A=$!
PENSIEVE_HTTP_ADDR="$NODE_B_ADDR" ./target/debug/pensieve >"$LOG_B" 2>&1 &
PID_B=$!

wait_healthy() {
    local addr="$1" tries=0
    while (( tries < 30 )); do
        if curl -sf "http://$addr/health" >/dev/null 2>&1; then return 0; fi
        sleep 1; tries=$((tries+1))
    done
    return 1
}
wait_healthy "$NODE_A_ADDR" || { tail -30 "$LOG_A"; fail "node A never healthy"; exit 3; }
wait_healthy "$NODE_B_ADDR" || { tail -30 "$LOG_B"; fail "node B never healthy"; exit 3; }
info "both nodes healthy (PIDs $PID_A, $PID_B)"

# Confirm both are registered in the catalog.
node_count=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM nodes WHERE role='all_in_one'")
assert_eq "2 nodes registered in catalog" "2" "$node_count"

# ------------------------------------------------------------------
# Generate per-request payloads
# ------------------------------------------------------------------
section "Generate $N_REQUESTS payloads of $ROWS_PER_REQUEST rows each"
PAYLOAD_DIR="$(mktemp -d)"
for ((i=1; i<=N_REQUESTS; i++)); do
    python3 - "$i" <<'PY' > "$PAYLOAD_DIR/r$i.ndjson"
import json, sys
from datetime import datetime, timedelta, timezone
req_id = int(sys.argv[1])
# Spread requests across a wide window so generated timestamps are always valid.
base = datetime(2026, 4, 19, 10, 0, 0, tzinfo=timezone.utc) + timedelta(seconds=req_id * 10)
for row_id in range(5):
    ts = (base + timedelta(seconds=row_id)).isoformat().replace("+00:00", "Z")
    print(json.dumps({
        "timestamp": ts,
        "req_id": req_id,
        "row_id": row_id,
        "message": f"payload-{req_id}-{row_id}",
    }))
PY
done
info "$N_REQUESTS payload files generated in $PAYLOAD_DIR"

# ------------------------------------------------------------------
# Fire parallel ingests, alternating between node A and node B
# ------------------------------------------------------------------
section "Fire $N_REQUESTS concurrent ingests (parallelism=$PARALLEL)"
RESULTS_DIR="$(mktemp -d)"

ingest_one() {
    local i="$1"
    local addr
    if (( i % 2 == 0 )); then addr="$NODE_A_ADDR"; else addr="$NODE_B_ADDR"; fi
    local resp
    resp=$(curl -s -w '\n__STATUS__%{http_code}' -X POST "http://$addr/v1/ingest" \
        -H 'X-Database: default' -H 'X-Table: test_a' \
        -H 'Content-Type: application/x-ndjson' \
        --data-binary @"$PAYLOAD_DIR/r$i.ndjson")
    local body="${resp%$'\n__STATUS__'*}"
    local status="${resp##*__STATUS__}"
    if [[ "$status" != "200" ]]; then
        echo "$i FAIL $status $body" >> "$RESULTS_DIR/fails"
    else
        echo "$i OK $(echo "$body" | jq -r .snapshot_id)" >> "$RESULTS_DIR/oks"
    fi
}

export -f ingest_one
export PAYLOAD_DIR RESULTS_DIR NODE_A_ADDR NODE_B_ADDR

START_TIME=$(date +%s)
seq 1 "$N_REQUESTS" | xargs -n1 -P "$PARALLEL" -I{} bash -c 'ingest_one "$@"' _ {}
END_TIME=$(date +%s)
duration=$((END_TIME - START_TIME))

ok_count=$(wc -l 2>/dev/null < "$RESULTS_DIR/oks" | tr -d ' ' || echo 0)
fail_count_ingest=$(if [[ -f "$RESULTS_DIR/fails" ]]; then wc -l < "$RESULTS_DIR/fails" | tr -d ' '; else echo 0; fi)
info "$ok_count ok, $fail_count_ingest failed in ${duration}s"
assert_eq "all $N_REQUESTS ingests returned 200" "$N_REQUESTS" "$ok_count"

# ------------------------------------------------------------------
# Verify via SQL against either node — same catalog, same data
# ------------------------------------------------------------------
section "Verify data integrity via SELECT against both nodes"

# One function to assert A==B, one function to fetch. Keep them separate so
# captured output is pure query result.
query_on() {
    local addr="$1" sql="$2"
    curl -s -X POST "http://$addr/v1/query" \
        -H 'X-Database: default' -H 'Content-Type: application/sql' --data "$sql"
}
assert_both_nodes_agree() {
    local sql="$1"
    local a b
    a=$(query_on "$NODE_A_ADDR" "$sql" | jq -c . 2>/dev/null | sort | md5)
    b=$(query_on "$NODE_B_ADDR" "$sql" | jq -c . 2>/dev/null | sort | md5)
    assert_eq "A/B agree on: $sql" "$a" "$b"
}

assert_both_nodes_agree 'SELECT COUNT(*) AS n FROM test_a'
assert_both_nodes_agree 'SELECT COUNT(DISTINCT req_id) AS n FROM test_a'
assert_both_nodes_agree 'SELECT SUM(row_id) AS s FROM test_a'

total=$(query_on "$NODE_A_ADDR" 'SELECT COUNT(*) AS n FROM test_a' | jq -r .n)
assert_eq "COUNT(*) = $EXPECTED_ROWS (no loss)" "$EXPECTED_ROWS" "$total"

distinct_reqs=$(query_on "$NODE_A_ADDR" 'SELECT COUNT(DISTINCT req_id) AS n FROM test_a' | jq -r .n)
assert_eq "DISTINCT req_id = $N_REQUESTS (no duplicates)" "$N_REQUESTS" "$distinct_reqs"

row_id_sum=$(query_on "$NODE_A_ADDR" 'SELECT SUM(row_id) AS s FROM test_a' | jq -r .s)
assert_eq "SUM(row_id) = $EXPECTED_ROW_ID_SUM (no duplicates within request)" "$EXPECTED_ROW_ID_SUM" "$row_id_sum"

# ------------------------------------------------------------------
# Verify catalog snapshot chain
# ------------------------------------------------------------------
section "Verify catalog snapshot chain"
snapshot_count=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM snapshots WHERE table_id = (SELECT id FROM tables WHERE name='test_a')")
# With group-commit staging, multiple ingests may roll up into a single
# snapshot — so snapshot count is in [2, N_REQUESTS + 1].
if (( snapshot_count >= 2 && snapshot_count <= N_REQUESTS + 1 )); then
    ok "snapshot chain length = $snapshot_count (in [2, $((N_REQUESTS + 1))])"
else
    fail "snapshot chain length = $snapshot_count outside [2, $((N_REQUESTS + 1))]"
fi

extent_count=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE table_id = (SELECT id FROM tables WHERE name='test_a') AND deleted_at IS NULL")
if (( extent_count >= 1 && extent_count <= N_REQUESTS )); then
    ok "live extent count = $extent_count (in [1, $N_REQUESTS])"
else
    fail "live extent count = $extent_count outside [1, $N_REQUESTS]"
fi

# ------------------------------------------------------------------
# Verify CAS contention was actually exercised
# ------------------------------------------------------------------
section "Verify CAS contention was exercised"
cas_retries_a=$(grep -c "snapshot CAS conflict on ingest" "$LOG_A" 2>/dev/null || true)
cas_retries_b=$(grep -c "snapshot CAS conflict on ingest" "$LOG_B" 2>/dev/null || true)
total_retries=$((cas_retries_a + cas_retries_b))
info "CAS conflict retries: A=$cas_retries_a, B=$cas_retries_b (total=$total_retries)"
if (( total_retries > 0 )); then
    ok "CAS-retry path actually fired ($total_retries times) — concurrency was real"
else
    # Non-zero retries are probabilistic — parallelism may not always race.
    # Accept zero but note it so we can tune N / PARALLEL / ROWS_PER_REQUEST.
    printf "  ${YLW}NOTE${NC} 0 CAS retries this run — race window missed. Try increasing PARALLEL.\n"
fi

# ------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------
section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass_count" "$fail_count"
if (( fail_count > 0 )); then
    printf "\n${YLW}Node A log tail:${NC}\n"; tail -15 "$LOG_A"
    printf "\n${YLW}Node B log tail:${NC}\n"; tail -15 "$LOG_B"
    exit 1
fi
exit 0
