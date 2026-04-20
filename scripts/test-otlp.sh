#!/usr/bin/env bash
# OTLP ingest E2E: start kyma with OTLP server enabled on :4317,
# send an ExportLogsServiceRequest via the Rust client, verify rows
# land in the auto-created otel_logs table.

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
export KYMA_GRPC_ADDR="127.0.0.1:9090"
export KYMA_OTLP_ADDR="127.0.0.1:4317"
export KYMA_OTLP_DATABASE="default"
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-info,sqlx=warn,hyper=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-otlp.log"
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

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset + start kyma with OTLP server"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

# Verify OTLP port is listening.
if nc -z 127.0.0.1 4317 2>/dev/null; then
    ok "OTLP port 4317 open"
else
    f "OTLP port 4317 NOT open"; tail -20 "$LOG_FILE"; exit 1
fi

section "Send ExportLogsServiceRequest via Rust client"
if cargo test -p kyma-ingest-otlp --test otlp_smoke -- --ignored --test-threads=1 2>&1 \
    | tee /tmp/otlp-test.log | grep -qE 'test result: ok\.'; then
    ok "otlp_export_logs client test passed"
else
    f "Rust OTLP client failed"
    tail -40 /tmp/otlp-test.log
    tail -20 "$LOG_FILE"
    exit 1
fi

section "Verify rows landed in otel_logs"
# The client sends 2 records — an INFO and an ERROR with "OutOfMemoryError".
count=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(*) AS n FROM otel_logs' | jq -r .n)
if [[ "$count" == "2" ]]; then
    ok "otel_logs has 2 rows"
else
    f "otel_logs row count = $count (want 2)"
fi

section "Verify service.name extraction"
svc=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT DISTINCT service_name FROM otel_logs' | jq -r .service_name)
if [[ "$svc" == "otlp-smoke-test" ]]; then
    ok "service_name extracted from resource attributes"
else
    f "service_name = $svc"
fi

section "Verify severity + body columns"
error_body=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT body FROM otel_logs WHERE severity_text = 'ERROR'" | jq -r .body)
if [[ "$error_body" == *"OutOfMemoryError"* ]]; then
    ok "ERROR record body preserved"
else
    f "ERROR body = $error_body"
fi

section "Text-index still works on OTLP-ingested data"
oom_count=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM otel_logs WHERE body LIKE '%OutOfMemoryError%'" | jq -r .n)
if [[ "$oom_count" == "1" ]]; then
    ok "text search prunes OTLP-ingested extent (needle matches 1 row)"
else
    f "text search on OTLP rows returned $oom_count"
fi

section "KQL query against OTLP table"
kql_count=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/x-kql' \
    --data 'otel_logs | where severity_text == "INFO" | count' | jq -r .Count)
if [[ "$kql_count" == "1" ]]; then
    ok "KQL query routed to OTLP table"
else
    f "KQL count = $kql_count"
fi

section "Metrics: OTLP record counter"
if curl -s "$HTTP_BASE/metrics" | grep -q 'kyma_otlp_log_records_total'; then
    ok "kyma_otlp_log_records_total exported"
else
    f "no OTLP metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
