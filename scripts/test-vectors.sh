#!/usr/bin/env bash
# E2E test: vector column type + cosine_distance UDF end-to-end.
#
# Note: uses SQL (application/sql Content-Type) because the KQL `<->`
# distance operator is not yet wired (Task A.9 deferred). Once A.9
# lands, a follow-up PR can replace the SQL query with the KQL form.

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
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-vectors.log"
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
    if [[ -n "$SERVER_PID" ]]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

section "Build release binary"
cargo build --release -p kyma-bin -q

section "Start kyma server"
./target/release/kyma > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 40); do
    if curl -sf "$HTTP_BASE/health" > /dev/null 2>&1; then
        break
    fi
    sleep 0.25
done
curl -sf "$HTTP_BASE/health" > /dev/null || { printf "${RED}server did not become healthy; see %s${NC}\n" "$LOG_FILE"; exit 1; }
ok "server up on 8080"

DB="vec_smoke"

section "DDL — create table with FixedSizeList<Float32, 3> column"
CODE=$(curl -s -o /tmp/vec-ddl.out -w '%{http_code}' -XPOST "$HTTP_BASE/v1/databases/$DB/tables" \
  -H 'Content-Type: application/json' \
  -d '{
    "name":"memos",
    "schema":{
        "fields":[
            {"name":"id","data_type":"Utf8","nullable":false},
            {"name":"embedding",
             "data_type":{"FixedSizeList":[{"name":"item","data_type":"Float32","nullable":false,"dict_id":0,"dict_is_ordered":false},3]},
             "nullable":false}
        ]
    }
  }')
if [[ "$CODE" == "200" || "$CODE" == "201" ]]; then ok "create_table http $CODE"; else f "create_table (http $CODE): $(cat /tmp/vec-ddl.out)"; fi

section "Ingest two rows with float-array embeddings"
CODE=$(curl -s -o /tmp/vec-ing.out -w '%{http_code}' -XPOST "$HTTP_BASE/v1/ingest" \
  -H "X-Database: $DB" -H "X-Table: memos" \
  -H 'Content-Type: application/x-ndjson' \
  --data-binary $'{"id":"apple","embedding":[1.0,0.0,0.0]}\n{"id":"orange","embedding":[0.0,1.0,0.0]}')
if [[ "$CODE" == "200" ]]; then ok "ingest http 200"; else f "ingest (http $CODE): $(cat /tmp/vec-ing.out)"; fi

section "SQL query — cosine_distance to [1.0, 0.05, 0.05]"
# Expected: 'apple' is nearest (cos ≈ 0.005); 'orange' is further (cos ≈ 1).
# DataFusion has no FixedSizeList literal syntax, so we cast via
# make_array + the registered cosine_distance UDF.
RESP=$(curl -sf -XPOST "$HTTP_BASE/v1/query" \
  -H "X-Database: $DB" \
  -H 'Content-Type: application/sql' \
  --data-binary "SELECT id, cosine_distance(embedding, make_array(1.0::float, 0.05::float, 0.05::float)) AS d FROM memos ORDER BY d ASC LIMIT 1")
if echo "$RESP" | grep -q '"apple"'; then ok "cosine_distance ranks apple first"; else f "apple not first: $RESP"; fi

section "SQL query — l2_distance"
RESP=$(curl -sf -XPOST "$HTTP_BASE/v1/query" \
  -H "X-Database: $DB" \
  -H 'Content-Type: application/sql' \
  --data-binary "SELECT id FROM memos ORDER BY l2_distance(embedding, make_array(1.0::float, 0.0::float, 0.0::float)) ASC LIMIT 1")
if echo "$RESP" | grep -q '"apple"'; then ok "l2_distance ranks apple first"; else f "l2 apple not first: $RESP"; fi

section "Reject wrong dimension on ingest"
CODE=$(curl -s -o /tmp/vec-dim.out -w '%{http_code}' -XPOST "$HTTP_BASE/v1/ingest" \
  -H "X-Database: $DB" -H "X-Table: memos" \
  -H 'Content-Type: application/x-ndjson' \
  --data-binary '{"id":"bad","embedding":[1.0,0.0]}')
if [[ "$CODE" == "400" ]]; then ok "wrong-dimension ingest rejected with 400"; else f "expected 400, got $CODE: $(cat /tmp/vec-dim.out)"; fi

printf "\n${BLU}==> Results:${NC} ${GRN}%d passed${NC}, ${RED}%d failed${NC}\n" "$pass" "$fail"
[[ $fail -eq 0 ]]
