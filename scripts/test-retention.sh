#!/usr/bin/env bash
# Retention + physical-GC E2E test.
#
# Creates a table with retention_days=1 and ingests data timestamped 2 days
# ago. Verifies the full two-pass lifecycle:
#   1. Retention sweeper soft-deletes the expired extents.
#   2. After grace, physical-gc worker removes the bytes from MinIO and the
#      catalog rows.

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
export KYMA_SELF_TRACE="off"   # deterministic storage-layout assertions
# Aggressive timings for the test:
export KYMA_RETENTION_POLL_SECS="2"
export KYMA_PHYSICAL_GC_POLL_SECS="2"
export KYMA_PHYSICAL_GC_GRACE_SECS="1"  # 1s grace so the test doesn't wait 24h
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/kyma-retention.log"
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
assert_eq() {
    if [[ "$2" == "$3" ]]; then ok "$1"
    else
        f "$1"
        printf "    ${RED}expected:${NC} %s\n    ${RED}actual:  ${NC} %s\n" "$2" "$3"
    fi
}
cleanup() {
    [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT

if ! docker exec kyma-postgres pg_isready -U kyma -d kyma >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state"
docker exec kyma-postgres psql -U kyma -d kyma -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec kyma-minio mc rm --recursive --force local/kyma >/dev/null 2>&1 || true
docker exec kyma-minio mc mb --ignore-existing local/kyma >/dev/null

section "Start kyma (retention 2s / grace 1s)"
./target/debug/kyma >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi
    sleep 1
done

section "Create table retention_days=1 + ingest 2-day-old data"
./target/debug/kyma-cli create-database default --if-not-exists >/dev/null
./target/debug/kyma-cli create-table --db default --name aged \
    --schema 'timestamp:timestamp,n:int' --retention-days 1 >/dev/null
./target/debug/kyma-cli create-table --db default --name fresh \
    --schema 'timestamp:timestamp,n:int' --retention-days 1 >/dev/null

# 2-day-old rows (expired).
python3 - <<'PY' > /tmp/aged.ndjson
import json
from datetime import datetime, timedelta, timezone
base = datetime.now(tz=timezone.utc) - timedelta(days=2)
for i in range(5):
    ts = (base + timedelta(minutes=i)).isoformat().replace("+00:00", "Z")
    print(json.dumps({"timestamp": ts, "n": i}))
PY
curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: aged' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/aged.ndjson > /dev/null

# Fresh rows (current — not expired).
python3 - <<'PY' > /tmp/fresh.ndjson
import json
from datetime import datetime, timezone
now = datetime.now(tz=timezone.utc).isoformat().replace("+00:00", "Z")
for i in range(3):
    print(json.dumps({"timestamp": now, "n": i}))
PY
curl -s -X POST "$HTTP_BASE/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: fresh' -H 'Content-Type: application/x-ndjson' \
    --data-binary @/tmp/fresh.ndjson > /dev/null

aged_live_before=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='aged')")
fresh_live_before=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='fresh')")
assert_eq "aged: 1 live extent before sweep"  "1" "$aged_live_before"
assert_eq "fresh: 1 live extent before sweep" "1" "$fresh_live_before"

section "Wait for retention sweep (soft-delete)"
deadline=$((SECONDS + 30))
swept=0
while (( SECONDS < deadline )); do
    live=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
        "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='aged')")
    if (( live == 0 )); then swept=1; break; fi
    sleep 1
done
if (( swept == 0 )); then
    f "retention sweeper never ran"
    tail -40 "$LOG_FILE"; exit 1
fi
ok "aged extents soft-deleted"

# Fresh table should NOT have been swept.
fresh_live_after=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='fresh')")
assert_eq "fresh: still 1 live extent (not expired)" "1" "$fresh_live_after"

section "Wait for physical-gc (remove bytes after grace)"
# Objects before physical-gc (both tables still have objects in MinIO — the
# aged one soft-deleted in catalog but not yet physically gone).
objects_before=$(docker exec kyma-minio mc ls -r local/kyma/ | grep -c '\.kyma$' || true)
printf "  ${DIM}objects before physical-gc: $objects_before${NC}\n"

deadline=$((SECONDS + 30))
gc_ok=0
while (( SECONDS < deadline )); do
    rows_left=$(docker exec kyma-postgres psql -U kyma -d kyma -tAc \
        "SELECT COUNT(*) FROM extents WHERE table_id = (SELECT id FROM tables WHERE name='aged')")
    if (( rows_left == 0 )); then gc_ok=1; break; fi
    sleep 1
done
if (( gc_ok == 0 )); then
    f "physical-gc never completed"
    tail -40 "$LOG_FILE"; exit 1
fi
ok "aged catalog rows physically deleted"

objects_after=$(docker exec kyma-minio mc ls -r local/kyma/ | grep -c '\.kyma$' || true)
printf "  ${DIM}objects after physical-gc: $objects_after${NC}\n"
if (( objects_after < objects_before )); then
    ok "MinIO objects deleted by physical-gc ($objects_before → $objects_after)"
else
    f "MinIO object count did not drop ($objects_before → $objects_after)"
fi

section "Verify fresh table intact"
fresh_rows=$(curl -s -X POST "$HTTP_BASE/v1/query" -H 'X-Database: default' \
    -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM fresh' | jq -r .n)
assert_eq "fresh table still has 3 rows" "3" "$fresh_rows"

section "Verify metrics"
metrics=$(curl -s "$HTTP_BASE/metrics")
if echo "$metrics" | grep -q 'kyma_retention_extents_soft_deleted_total'; then
    ok "retention soft-delete counter fired"
else
    f "no retention_extents_soft_deleted metric"
fi
if echo "$metrics" | grep -q 'kyma_physical_gc_objects_deleted_total'; then
    ok "physical-gc object-delete counter fired"
else
    f "no physical_gc_objects_deleted metric"
fi

section "Summary"
printf "${GRN}PASSED: %d${NC}  ${RED}FAILED: %d${NC}\n" "$pass" "$fail"
if (( fail > 0 )); then
    printf "\n${RED}Server log tail:${NC}\n"
    tail -40 "$LOG_FILE"
    exit 1
fi
exit 0
