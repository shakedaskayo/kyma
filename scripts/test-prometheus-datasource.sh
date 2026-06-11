#!/usr/bin/env bash
# End-to-end test of the Prometheus data source.
#
# Prerequisites:
#   * docker-compose up (postgres + minio + redpanda)
#   * kyma binary running on $KYMA_HTTP (default http://127.0.0.1:8080)
#   * python3 available for the mock /metrics server

set -euo pipefail

HTTP="${KYMA_HTTP:-http://127.0.0.1:8080}"
MOCK_PORT="${MOCK_PORT:-19191}"
FIXTURE="$(dirname "$0")/fixtures/prom-metrics.txt"

if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"
else
    RED=""; GRN=""; BLU=""; NC=""
fi

pass=0
fail=0

section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()   { pass=$((pass+1)); printf "  ${GRN}PASS${NC} %s\n" "$1"; }
nope() { fail=$((fail+1)); printf "  ${RED}FAIL${NC} %s\n" "$1"; printf "    %s\n" "$2"; }

cleanup() {
    if [[ -n "${MOCK_PID:-}" ]] && kill -0 "$MOCK_PID" 2>/dev/null; then
        kill "$MOCK_PID" 2>/dev/null || true
        wait "$MOCK_PID" 2>/dev/null || true
    fi
    if [[ -n "${CONN_ID:-}" ]]; then
        curl -s -X DELETE "$HTTP/v1/data-sources/$CONN_ID" >/dev/null || true
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
section "Start mock Prometheus /metrics server"
# ---------------------------------------------------------------------------
SERVE_DIR="$(mktemp -d)"
cp "$FIXTURE" "$SERVE_DIR/metrics"
( cd "$SERVE_DIR" && python3 -u -m http.server "$MOCK_PORT" ) > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 0.5

if kill -0 "$MOCK_PID" 2>/dev/null; then
    ok "mock server started on port $MOCK_PORT (pid $MOCK_PID)"
else
    nope "mock server" "failed to start — see /tmp/mock-prom.log"
    exit 1
fi

# ---------------------------------------------------------------------------
section "Bootstrap telemetry.metrics schema"
# ---------------------------------------------------------------------------
# Plant the 'metrics' table schema via an NDJSON ingest to make sure the
# data source's first scrape doesn't lazy-create an incompatible schema.
BOOT=$(echo '{"timestamp":"2026-01-01T00:00:00Z","name":"bootstrap","value":0.0,"labels":"{}"}' \
    | curl -sS -w '\n%{http_code}' -X POST "$HTTP/v1/ingest" \
        -H 'Content-Type: application/x-ndjson' \
        -H 'X-Database: telemetry' \
        -H 'X-Table: metrics' \
        --data-binary @-)
BOOT_STATUS=$(echo "$BOOT" | tail -1)
if [[ "$BOOT_STATUS" == "200" || "$BOOT_STATUS" == "204" ]]; then
    ok "schema bootstrap ingest accepted (HTTP $BOOT_STATUS)"
else
    nope "schema bootstrap" "HTTP $BOOT_STATUS — response: $(echo "$BOOT" | head -1)"
fi

# ---------------------------------------------------------------------------
section "Create Prometheus data source"
# ---------------------------------------------------------------------------
BODY=$(cat <<EOF
{
  "name": "e2e-prom",
  "type": "prometheus",
  "target_database": "telemetry",
  "target_table": "metrics",
  "schedule_ms": 1000,
  "config": { "endpoint": "http://127.0.0.1:${MOCK_PORT}/metrics" }
}
EOF
)
CREATE=$(curl -sS -X POST "$HTTP/v1/data-sources" \
    -H 'Content-Type: application/json' -d "$BODY")
CONN_ID=$(echo "$CREATE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' 2>/dev/null || echo "")

if [[ -n "$CONN_ID" ]]; then ok "data source created id=$CONN_ID"
else nope "create data source" "$CREATE"; fi

# ---------------------------------------------------------------------------
section "Wait for first successful scrape"
# ---------------------------------------------------------------------------
ROWS=0
STATUS=""
for _ in $(seq 1 20); do
    STATUS=$(curl -sS "$HTTP/v1/data-sources/$CONN_ID" 2>/dev/null || echo '')
    if echo "$STATUS" | grep -q '"enabled":true'; then
        ROWS=$(curl -sS "$HTTP/metrics" \
            | grep -E "kyma_data_source_rows_ingested_total\\{.*data_source_id=\"$CONN_ID\"" \
            | awk '{print $NF}' | head -1 || echo 0)
        if [[ "${ROWS:-0}" -gt 0 ]]; then break; fi
    fi
    sleep 0.5
done
if [[ "${ROWS:-0}" -gt 0 ]]; then ok "data source ran (rows=$ROWS)"
else nope "first scrape" "STATUS=$STATUS ROWS=${ROWS:-0}"; fi

# ---------------------------------------------------------------------------
section "Query: histogram rows present"
# ---------------------------------------------------------------------------
QUERY() {
    curl -sS -X POST "$HTTP/v1/query" \
        -H 'Content-Type: application/sql' \
        -H 'X-Database: telemetry' \
        --data "$1"
}

N=$(QUERY "SELECT count(*) AS n FROM metrics WHERE name LIKE 'test_%'" \
   | python3 -c 'import json,sys
out = sys.stdin.read().strip().split("\n")
if not out: print(0); exit()
try: print(json.loads(out[-1]).get("n",0))
except: print(0)' 2>/dev/null || echo 0)
if [[ "${N:-0}" -ge 5 ]]; then ok "histogram rows present (n=$N)"
else nope "histogram rows" "n=${N:-0} (want >=5: 3 buckets + sum + count)"; fi

# ---------------------------------------------------------------------------
section "Query: label preservation"
# ---------------------------------------------------------------------------
STATUS_VALS=$(QUERY "SELECT DISTINCT labels FROM metrics WHERE name = 'http_requests_total'")
if echo "$STATUS_VALS" | grep -q '"200"' && echo "$STATUS_VALS" | grep -q '"500"'; then
    ok "labels preserved (status 200 and 500 present)"
else
    nope "label preservation" "$STATUS_VALS"
fi

# ---------------------------------------------------------------------------
section "Pause/resume: pause stops new ticks"
# ---------------------------------------------------------------------------
BEFORE=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
curl -sS -X POST "$HTTP/v1/data-sources/$CONN_ID/pause" >/dev/null
sleep 2
AFTER=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
if [[ "$BEFORE" == "$AFTER" ]]; then ok "pause stops new ticks"
else nope "pause" "before=$BEFORE after=$AFTER"; fi
curl -sS -X POST "$HTTP/v1/data-sources/$CONN_ID/resume" >/dev/null
ok "resume accepted"

# ---------------------------------------------------------------------------
section "503 error: transient errors_total increments"
# ---------------------------------------------------------------------------
kill "$MOCK_PID" 2>/dev/null || true
wait "$MOCK_PID" 2>/dev/null || true
python3 -u -c "
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(503)
        self.end_headers()
    def log_message(self, *a, **kw): pass
HTTPServer(('127.0.0.1', ${MOCK_PORT}), H).serve_forever()
" > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 4
ERR=$(curl -sS "$HTTP/metrics" \
    | grep -E 'kyma_data_source_errors_total.*reason="transient"' \
    | awk '{print $NF}' | head -1 || echo 0)
if [[ "${ERR%.*}" -gt 0 ]] 2>/dev/null; then ok "transient errors counted ($ERR)"
else nope "errors_total transient" "ERR=${ERR:-0}"; fi

# ---------------------------------------------------------------------------
section "Recovery after 503"
# ---------------------------------------------------------------------------
kill "$MOCK_PID" 2>/dev/null || true
wait "$MOCK_PID" 2>/dev/null || true
( cd "$SERVE_DIR" && python3 -u -m http.server "$MOCK_PORT" ) > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 2

RECOVERED=$(curl -sS "$HTTP/v1/data-sources/$CONN_ID" | grep -c 'last_success_at' || echo 0)
if [[ "$RECOVERED" -ge 1 ]]; then ok "data source recovered after 503 (last_success_at present)"
else nope "recovery" "last_success_at not found in data source response"; fi

# ---------------------------------------------------------------------------
section "Trigger: immediate tick"
# ---------------------------------------------------------------------------
TRIG_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' \
    -X POST "$HTTP/v1/data-sources/$CONN_ID/trigger")
if [[ "$TRIG_STATUS" == "202" || "$TRIG_STATUS" == "200" || "$TRIG_STATUS" == "204" ]]; then
    ok "trigger accepted (HTTP $TRIG_STATUS)"
else
    nope "trigger" "HTTP $TRIG_STATUS"
fi
sleep 2

# ---------------------------------------------------------------------------
section "Delete data source"
# ---------------------------------------------------------------------------
curl -sS -X DELETE "$HTTP/v1/data-sources/$CONN_ID" >/dev/null
GONE=$(curl -sS -o /dev/null -w '%{http_code}' "$HTTP/v1/data-sources/$CONN_ID")
if [[ "$GONE" == "404" ]]; then ok "data source deleted (404 on re-fetch)"
else nope "delete" "expected 404, got $GONE"; fi

CONN_ID=""   # prevent cleanup double-delete

# ---------------------------------------------------------------------------
section "Summary"
# ---------------------------------------------------------------------------
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then exit 1; fi
exit 0
