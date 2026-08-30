#!/usr/bin/env bash
# Kafka ingest E2E.
#
# 1. Ensure redpanda is up.
# 2. Create a test topic + pensieve target table.
# 3. Start pensieve with Kafka consumer enabled on that topic.
# 4. Produce NDJSON messages into Kafka.
# 5. Verify rows appear in the table within the batch timeout.
# 6. Stop pensieve, produce more messages, restart pensieve — verify it picks up
#    from where it left off (at-least-once, no data loss).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TOPIC="pensieve-test-$(date +%s)"
GROUP="pensieve-ingest-test-$(date +%s)"

export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
export PENSIEVE_S3_ENDPOINT="http://localhost:9000"
export PENSIEVE_S3_BUCKET="pensieve"
export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
export PENSIEVE_S3_PATH_STYLE="true"
export PENSIEVE_S3_ALLOW_HTTP="true"
export PENSIEVE_HTTP_ADDR="127.0.0.1:8080"
export PENSIEVE_GRPC_ADDR="off"
export PENSIEVE_OTLP_ADDR="off"
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"

export PENSIEVE_KAFKA_ENABLED=1
export PENSIEVE_KAFKA_BROKERS="localhost:9092"
export PENSIEVE_KAFKA_GROUP="$GROUP"
export PENSIEVE_KAFKA_TOPICS="$TOPIC:default.kafka_events"
export PENSIEVE_KAFKA_BATCH_SIZE=10
export PENSIEVE_KAFKA_BATCH_TIMEOUT_MS=500
export RUST_LOG="${RUST_LOG:-info,sqlx=warn,rdkafka=warn,hyper=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-kafka.log"
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

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack (postgres) not up.${NC}\n"; exit 2
fi
if ! docker exec pensieve-redpanda rpk cluster info >/dev/null 2>&1; then
    printf "${RED}redpanda not up. Run: docker-compose up -d redpanda${NC}\n"; exit 2
fi

section "Reset state"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null

section "Create Kafka topic + pensieve target table"
docker exec pensieve-redpanda rpk topic create "$TOPIC" -p 1 -r 1 >/dev/null
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name kafka_events \
    --schema 'timestamp:timestamp,level:string,message:string' >/dev/null
ok "topic=$TOPIC, table=default.kafka_events"

section "Start pensieve with Kafka consumer enabled"
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
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
done | docker exec -i pensieve-redpanda rpk topic produce "$TOPIC" --brokers=localhost:9092 >/dev/null

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

section "Shutdown pensieve, produce more, restart — verify no data loss"
kill -9 "$SERVER_PID"
wait "$SERVER_PID" 2>/dev/null || true
SERVER_PID=""

# Produce 10 more messages while pensieve is down.
for i in $(seq 31 40); do
    payload="{\"timestamp\":\"2026-04-20T11:01:$(printf %02d $((i % 60)))Z\",\"level\":\"INFO\",\"message\":\"kafka-msg-$i\"}"
    echo "$payload"
done | docker exec -i pensieve-redpanda rpk topic produce "$TOPIC" --brokers=localhost:9092 >/dev/null

# Restart pensieve.
./target/debug/pensieve >>"$LOG_FILE" 2>&1 &
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
if curl -s "$HTTP_BASE/metrics" | grep -q 'pensieve_kafka_messages_ingested_total'; then
    ok "pensieve_kafka_messages_ingested_total exported"
else
    f "no kafka metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then tail -30 "$LOG_FILE"; exit 1; fi
exit 0
