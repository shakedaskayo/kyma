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

export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:5433/pensieve"
export PENSIEVE_S3_ENDPOINT="http://localhost:9000"
export PENSIEVE_S3_BUCKET="pensieve"
export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
export PENSIEVE_S3_PATH_STYLE="true"
export PENSIEVE_S3_ALLOW_HTTP="true"
export PENSIEVE_HTTP_ADDR="127.0.0.1:8080"
export PENSIEVE_SELF_TRACE="off"   # deterministic storage-layout assertions
# Aggressive timings for the test:
export PENSIEVE_RETENTION_POLL_SECS="2"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="2"
export PENSIEVE_PHYSICAL_GC_GRACE_SECS="1"  # 1s grace so the test doesn't wait 24h
export RUST_LOG="${RUST_LOG:-info,sqlx=warn}"

HTTP_BASE="http://127.0.0.1:8080"
LOG_FILE="/tmp/pensieve-retention.log"
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

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Reset state"
docker exec pensieve-postgres psql -U pensieve -d pensieve -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force local/pensieve >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing local/pensieve >/dev/null

section "Start pensieve (retention 2s / grace 1s)"
./target/debug/pensieve >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi
    sleep 1
done

section "Create table retention_days=1 + ingest 2-day-old data"
./target/debug/pensieve-cli create-database default --if-not-exists >/dev/null
./target/debug/pensieve-cli create-table --db default --name aged \
    --schema 'timestamp:timestamp,n:int' --retention-days 1 >/dev/null
./target/debug/pensieve-cli create-table --db default --name fresh \
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

# The aged extent is expired on arrival (2-day-old data, retention_days=1), so
# the background sweeper (PENSIEVE_RETENTION_POLL_SECS=2) may soft-delete it before
# this check runs — racing on "live before sweep" is non-deterministic. Assert
# instead that the ingest CREATED an extent (total count, robust to soft-delete,
# which only sets deleted_at and keeps the row until physical-gc). The sweep is
# verified deterministically by the soft-delete wait below.
aged_total=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE table_id = (SELECT id FROM tables WHERE name='aged')")
fresh_live_before=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='fresh')")
assert_eq "aged: 1 extent created"            "1" "$aged_total"
assert_eq "fresh: 1 live extent before sweep" "1" "$fresh_live_before"

section "Wait for retention sweep (soft-delete)"
deadline=$((SECONDS + 30))
swept=0
while (( SECONDS < deadline )); do
    live=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
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
fresh_live_after=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
    "SELECT COUNT(*) FROM extents WHERE deleted_at IS NULL AND table_id = (SELECT id FROM tables WHERE name='fresh')")
assert_eq "fresh: still 1 live extent (not expired)" "1" "$fresh_live_after"

section "Wait for physical-gc (remove bytes after grace)"
# Objects before physical-gc (both tables still have objects in MinIO — the
# aged one soft-deleted in catalog but not yet physically gone).
objects_before=$(docker exec pensieve-minio mc ls -r local/pensieve/ | grep -c '\.pensieve$' || true)
printf "  ${DIM}objects before physical-gc: $objects_before${NC}\n"

deadline=$((SECONDS + 30))
gc_ok=0
while (( SECONDS < deadline )); do
    rows_left=$(docker exec pensieve-postgres psql -U pensieve -d pensieve -tAc \
        "SELECT COUNT(*) FROM extents WHERE table_id = (SELECT id FROM tables WHERE name='aged')")
    if (( rows_left == 0 )); then gc_ok=1; break; fi
    sleep 1
done
if (( gc_ok == 0 )); then
    f "physical-gc never completed"
    tail -40 "$LOG_FILE"; exit 1
fi
ok "aged catalog rows physically deleted"

objects_after=$(docker exec pensieve-minio mc ls -r local/pensieve/ | grep -c '\.pensieve$' || true)
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
if echo "$metrics" | grep -q 'pensieve_retention_extents_soft_deleted_total'; then
    ok "retention soft-delete counter fired"
else
    f "no retention_extents_soft_deleted metric"
fi
if echo "$metrics" | grep -q 'pensieve_physical_gc_objects_deleted_total'; then
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
