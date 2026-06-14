#!/usr/bin/env bash
# fresh-install-validate.sh — validate a clean, from-scratch install works.
#
# The scale program's acceptance gate (per the user directive): after merging
# tested work to main, prove the engine runs from a *fresh* state with no
# pre-existing data or infra. This exercises the LOCAL single-binary path
# (`kyma serve`: SQLite catalog + local-filesystem store, zero external infra) —
# the same path a new user gets from `install.sh` / `cargo install`.
#
# Steps: build the `kyma` binary -> serve from an empty data dir -> health ->
# create a vector table -> ingest -> query (count, filter, aggregate) ->
# vector search (cosine) -> assert each. Exits non-zero on first failure.
#
# Usage: scripts/fresh-install-validate.sh

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PORT="${KYMA_FRESH_PORT:-7799}"
BASE="http://127.0.0.1:${PORT}"
DATA_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kyma-fresh-XXXXXX")"
BIN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kyma-fresh-bin-XXXXXX")"
LOG="${DATA_DIR}/serve.log"
SERVER_PID=""

if [[ -t 1 ]]; then RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"; else RED=""; GRN=""; BLU=""; NC=""; fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()  { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
bad() { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
assert_eq() { [[ "$2" == "$3" ]] && ok "$1" || { bad "$1 (expected '$2', got '$3')"; }; }

cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  [[ -n "$SERVER_PID" ]] && wait "$SERVER_PID" 2>/dev/null || true
  rm -rf "$DATA_DIR" "$BIN_DIR"
}
trap cleanup EXIT

section "Build the kyma binary (from source, like a fresh install)"
cargo build -p kyma-cli --quiet
cp target/debug/kyma-cli "$BIN_DIR/kyma"
KYMA="$BIN_DIR/kyma"
"$KYMA" --help >/dev/null 2>&1 && ok "kyma binary runs" || bad "kyma binary won't run"

section "Serve from an EMPTY data dir (local mode: SQLite + local FS, zero infra)"
ls -A "$DATA_DIR" | grep -q . && bad "data dir not empty before start" || ok "data dir is empty"
KYMA_LOCAL_MODE=1 \
KYMA_LOCAL_DATA="$DATA_DIR/data" \
KYMA_HOME="$DATA_DIR/home" \
KYMA_SECRET_KEY="${KYMA_SECRET_KEY:-fresh_install_validate_secret_key_32}" \
  "$KYMA" serve --addr "127.0.0.1:${PORT}" >"$LOG" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 30); do
  curl -fsS -o /dev/null "${BASE}/health" 2>/dev/null && break
  sleep 1
done
health=$(curl -fsS "${BASE}/health" 2>/dev/null || echo '{}')
echo "$health" | grep -q '"ok"' && ok "/health is ok on fresh serve" || { bad "/health not ok: $health"; tail -30 "$LOG"; exit 1; }

# Local serve has zero-auth by default; if a token is required these would 401.
auth=()
section "Bootstrap: create a vector table (idempotent)"
"$KYMA" create-database default --if-not-exists --catalog-url "sqlite://${DATA_DIR}/data/catalog.db" >/dev/null 2>&1 \
  || true  # serve auto-creates default; tolerate either way
# Create a table with a 4-dim embedding column via the local HTTP ingest auto-create.
section "Ingest rows with a vector column"
python3 - "$BASE" <<'PY'
import json, sys, urllib.request
base = sys.argv[1]
rows = []
# 6 rows; embeddings cluster so cosine search is meaningful.
import math
def norm(v):
    n = math.sqrt(sum(x*x for x in v)) or 1.0
    return [x/n for x in v]
data = [
    ("alpha login ok",   [1.0, 0.0, 0.0, 0.0]),
    ("alpha login retry",[0.9, 0.1, 0.0, 0.0]),
    ("beta logout",      [0.0, 1.0, 0.0, 0.0]),
    ("beta error 500",   [0.0, 0.9, 0.1, 0.0]),
    ("gamma timeout",    [0.0, 0.0, 1.0, 0.0]),
    ("delta crash oom",  [0.0, 0.0, 0.0, 1.0]),
]
for i,(msg,emb) in enumerate(data):
    rows.append({"timestamp": f"2026-06-14T10:00:{i:02d}Z", "id": i, "msg": msg, "embedding": norm(emb)})
body = "\n".join(json.dumps(r) for r in rows).encode()
req = urllib.request.Request(base + "/v1/ingest", data=body, method="POST",
    headers={"X-Database":"default","X-Table":"events","Content-Type":"application/x-ndjson"})
resp = urllib.request.urlopen(req, timeout=30)
print(resp.read().decode())
PY
ok "ingest accepted"

# Give the staging buffer a moment to flush to an extent.
sleep 2

section "Query: COUNT(*) returns 6"
q() { curl -fsS -X POST "${BASE}/v1/query" -H 'Content-Type: application/sql' -H 'X-Database: default' --data "$1"; }
cnt=$(q 'SELECT COUNT(*) AS n FROM events' | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo "ERR")
assert_eq "row count = 6" "6" "$cnt"

section "Query: filter by substring"
beta=$(q "SELECT COUNT(*) AS n FROM events WHERE msg LIKE '%beta%'" | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo ERR)
assert_eq "rows matching 'beta' = 2" "2" "$beta"

section "Vector search: cosine_distance ranks the nearest cluster first"
# Query near 'alpha' cluster [1,0,0,0]; nearest row id should be 0 or 1.
near=$(q "SELECT id, cosine_distance(embedding, make_array(1.0,0.0,0.0,0.0)) AS d FROM events ORDER BY d ASC LIMIT 1" \
  | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["id"])' 2>/dev/null || echo ERR)
[[ "$near" == "0" || "$near" == "1" ]] && ok "nearest vector is an alpha-cluster row (id=$near)" || bad "unexpected nearest id: $near"

section "Unified search endpoint responds"
sc=$(curl -fsS -o /dev/null -w "%{http_code}" -X POST "${BASE}/v1/search" \
  -H 'Content-Type: application/json' -H 'X-Database: default' \
  --data '{"query":"alpha","scope":"events","limit":5}' 2>/dev/null || echo 000)
[[ "$sc" == "200" ]] && ok "/v1/search returns 200" || bad "/v1/search returned $sc"

section "Restart durability: data survives a serve restart"
kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true
KYMA_LOCAL_MODE=1 KYMA_LOCAL_DATA="$DATA_DIR/data" KYMA_HOME="$DATA_DIR/home" \
KYMA_SECRET_KEY="${KYMA_SECRET_KEY:-fresh_install_validate_secret_key_32}" \
  "$KYMA" serve --addr "127.0.0.1:${PORT}" >>"$LOG" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 30); do curl -fsS -o /dev/null "${BASE}/health" 2>/dev/null && break; sleep 1; done
cnt2=$(q 'SELECT COUNT(*) AS n FROM events' | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo ERR)
assert_eq "row count survives restart = 6" "6" "$cnt2"

section "Summary"
printf "${GRN}PASS: %d${NC}  ${RED}FAIL: %d${NC}\n" "$pass" "$fail"
if [[ $fail -gt 0 ]]; then printf "\nServe log tail:\n"; tail -30 "$LOG"; exit 1; fi
echo "fresh-install validation: GREEN"
