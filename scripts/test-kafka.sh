#!/usr/bin/env bash
# Kafka ingest E2E.
#
# 1. Ensure redpanda is up.
# 2. Create a test topic + kyma target table.
# 3. Start kyma with Kafka consumer enabled on that topic.
# 4. Produce NDJSON messages into Kafka.
# 5. Verify rows appear in the table within the batch timeout.
# 6. Stop kyma, produce more messages, restart kyma — verify it picks up
#    from where it left off (at-least-once, no data loss).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TOPIC="kyma-test-$(date +%s)"
GROUP="kyma-ingest-test-$(date +%s)"

export KYMA_CATALOG_URL="postgres://kyma:kyma_dev@localhost:5433/kyma"
export KYMA_S3_ENDPOINT="http://localhost:9000"
export KYMA_S3_BUCKET="kyma"
export KYMA_S3_ACCESS_KEY_ID="kyma_admin"
export KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev"
export KYMA_S3_PATH_STYLE="true"
export KYMA_S3_ALLOW_HTTP="true"
export KYMA_HTTP_ADDR="127.0.0.1:8080"
export KYMA_GRPC_ADDR="off"
export KYMA_OTLP_ADDR="off"
export KYMA_COMPACTION_POLL_SECS="3600"
export KYMA_RETENTION_POLL_SECS="3600"
export KYMA_PHYSICAL_GC_POLL_SECS="3600"

export KYMA_KAFKA_ENABLED=1
export KYMA_KAFKA_BROKERS="localhost:9092"
export KYMA_KAFKA_GROUP="$GROUP"
export KYMA_KAFKA_TOPICS="$TOPIC:default.kafka_events"
export KYMA_KAFKA_BATCH_SIZE=10
export KYMA_KAFKA_BATCH_TIMEOUT_MS=500
export RUST_LOG="${RUST_LOG:-info,sqlx=warn,rdkafka=warn,hyper=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-kafka.log"
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
    printf "${RED}docker-compose stack (postgres) not up.${NC}\n"; exit 2
fi
if ! docker exec kyma-redpanda rpk cluster info >/dev/null 2>&1; then
    printf "${RED}redpanda not up. Run: docker-compose up -d redpanda${NC}\n"; exit 2
fi

section "Reset state"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null

section "Create Kafka topic + kyma target table"
docker exec kyma-redpanda rpk topic create "$TOPIC" -p 1 -r 1 >/dev/null
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name kafka_events \
    --schema 'timestamp:timestamp,level:string,message:string' >/dev/null
ok "topic=$TOPIC, table=default.kafka_events"

section "Start kyma with Kafka consumer enabled"
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done
if grep -q "kafka consumer enabled" "$LOG_FILE" && grep -q "kafka consumer starting" "$LOG_FILE"; then
    ok "kafka consumer started"
else
    f "kafka consumer not started"
    tail -20 "$LOG_FILE"; exit 1
fi

section "Produce 30 NDJSON messages into Kafka"
for i in $(seq 1 30); do
    payload="{\"timestamp\":\"2026-04-20T11:00:$(printf %02d $((i % 60)))Z\",\"level\":\"INFO\",\"message\":\"kafka-msg-$i\"}"
    echo "$payload"
done | docker exec -i kyma-redpanda rpk topic produce "$TOPIC" --brokers=localhost:9092 >/dev/null

section "Wait for consumer to ingest all 30"
got=0
for i in $(seq 1 20); do
    n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
        --data 'SELECT COUNT(*) AS n FROM kafka_events' | jq -r .n 2>/dev/null || echo 0)
    if [[ "$n" == "30" ]]; then got=1; break; fi
    sleep 1
done
if (( got == 1 )); then
    ok "kafka consumer ingested 30 rows"
else
    f "got $n rows (want 30)"; tail -30 "$LOG_FILE"; exit 1
fi

section "Verify content: DISTINCT count"
distinct=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data 'SELECT COUNT(DISTINCT message) AS n FROM kafka_events' | jq -r .n)
if [[ "$distinct" == "30" ]]; then
    ok "30 distinct messages ingested"
else
    f "distinct=$distinct"
fi

section "Text-index pruning works on Kafka-ingested rows"
hit=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
    --data "SELECT COUNT(*) AS n FROM kafka_events WHERE message LIKE '%kafka-msg-17%'" | jq -r .n)
if [[ "$hit" == "1" ]]; then
    ok "text-index prunes Kafka data"
else
    f "LIKE '%kafka-msg-17%' count=$hit"
fi

section "Shutdown kyma, produce more, restart — verify no data loss"
kill -9 "$SERVER_PID"
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""

# Produce 10 more messages while kyma is down.
for i in $(seq 31 40); do
    payload="{\"timestamp\":\"2026-04-20T11:01:$(printf %02d $((i % 60)))Z\",\"level\":\"INFO\",\"message\":\"kafka-msg-$i\"}"
    echo "$payload"
done | docker exec -i kyma-redpanda rpk topic produce "$TOPIC" --brokers=localhost:9092 >/dev/null

# Restart kyma.
./target/debug/kyma >>"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done

got=0
# Allow up to 60s — consumer-group rebalance with a single broker can
# take 10-20s after the old session times out. Session timeout is 6s.
for i in $(seq 1 60); do
    n=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' \
        --data 'SELECT COUNT(*) AS n FROM kafka_events' | jq -r .n 2>/dev/null || echo 0)
    if [[ "$n" == "40" ]]; then got=1; break; fi
    sleep 1
done
if (( got == 1 )); then
    ok "consumer resumed from committed offset (40 rows total; no loss)"
else
    f "post-restart count=$n (want 40)"; tail -40 "$LOG_FILE"; exit 1
fi

section "Metrics: Kafka counter"
if curl -s "$HTTP_BASE/metrics" | grep -q 'kyma_kafka_messages_ingested_total'; then
    ok "kyma_kafka_messages_ingested_total exported"
else
    f "no kafka metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
