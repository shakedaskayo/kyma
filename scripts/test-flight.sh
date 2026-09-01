#!/usr/bin/env bash
# E2E test: Arrow Flight gRPC query surface via cargo test (Rust client).

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
export PENSIEVE_GRPC_ADDR="127.0.0.1:9090"
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

LOG_FILE="/tmp/pensieve-flight.log"
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
        # Don't wait — tonic's graceful shutdown can hang on an open gRPC
        # stream, and SIGKILL already freed the resources.
    fi
}
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
    if curl -sf http://127.0.0.1:8080/health >/dev/null 2>&1; then break; fi; sleep 1
done

section "Create table + ingest"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name flight_t \
    --schema 'timestamp:timestamp,n:int,payload:string' >/dev/null
python3 - <<'PY' > /tmp/flight_data.ndjson
import json
for i in range(100):
    print(json.dumps({"timestamp": f"2026-04-19T10:{i//60:02d}:{i%60:02d}Z", "n": i, "payload": f"row-{i}"}))
PY
curl -s -X POST http://127.0.0.1:8080/v1/ingest \
    -H 'X-Database: default' -H 'X-Table: flight_t' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/flight_data.ndjson > /dev/null

section "Verify GRPC port is listening"
if nc -z 127.0.0.1 9090 2>/dev/null; then
    ok "gRPC port 9090 open"
else
    f "gRPC port 9090 NOT open"
    tail -20 "$LOG_FILE"; exit 1
fi

section "Run Flight client tests (Rust)"
# Run the ignored tests that specifically exercise the Flight surface.
if cargo test -p pensieve-server --test flight_smoke -- --ignored --test-threads=1 2>&1 | tee /tmp/flight-tests.log | grep -qE 'test result: ok\.'; then
    ok "flight_sql_roundtrip + flight_kql_roundtrip passed"
else
    f "Rust flight tests failed"
    tail -40 /tmp/flight-tests.log
    tail -20 "$LOG_FILE"
    exit 1
fi

section "Metrics exported"
if curl -s http://127.0.0.1:8080/metrics | grep -q 'pensieve_flight_do_get_total'; then
    ok "pensieve_flight_do_get_total metric present"
else
    f "no Flight metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then exit 1; fi
exit 0
