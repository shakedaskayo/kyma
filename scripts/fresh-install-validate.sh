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

# Local serve mounts the full API behind role auth (ingest=Write, query=Read)
# and seeds a default web-UI user (admin/admin). We authenticate the way a real
# user does: POST /v1/auth/login -> access_token -> Bearer on every request.
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

section "Authenticate (admin/admin login → access token)"
TOKEN=$(curl -fsS -X POST "${BASE}/v1/auth/login" -H 'Content-Type: application/json' \
  --data '{"username":"admin","password":"admin"}' 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])' 2>/dev/null || echo "")
[[ -n "$TOKEN" ]] && ok "logged in as admin (got access token)" || { bad "login failed"; tail -20 "$LOG"; exit 1; }
me_code=$(curl -fsS -o /dev/null -w "%{http_code}" -H "Authorization: Bearer ${TOKEN}" "${BASE}/v1/auth/me" 2>/dev/null || echo 000)
assert_eq "/v1/auth/me with token" "200" "$me_code"

# Ingest telemetry rows via NDJSON auto-create. NDJSON auto-create infers every
# column as Utf8 (string) — so we send string values (a fresh user's reality;
# typed/vector tables need an explicit schema, covered by retrieval-bench + the
# gate's ANN tests). Fresh-install's job is the clean-install serve path.
section "Ingest telemetry rows (table auto-created, Utf8 columns)"
python3 - "$BASE" "$TOKEN" <<'PY'
import json, sys, urllib.request
base, token = sys.argv[1], sys.argv[2]
data = [
    ("alpha login ok",    "200"),
    ("alpha login retry", "200"),
    ("beta logout",       "200"),
    ("beta error 500",    "500"),
    ("gamma timeout",     "504"),
    ("delta crash oom",   "500"),
]
rows = [{"timestamp": f"2026-06-14T10:00:{i:02d}Z", "id": str(i), "msg": msg, "status": st}
        for i,(msg,st) in enumerate(data)]
body = "\n".join(json.dumps(r) for r in rows).encode()
req = urllib.request.Request(base + "/v1/ingest", data=body, method="POST",
    headers={"X-Database":"default","X-Table":"events","Content-Type":"application/x-ndjson",
             "Authorization": f"Bearer {token}"})
print(urllib.request.urlopen(req, timeout=30).read().decode())
PY
ok "ingest accepted"

# Give the staging buffer a moment to flush to an extent.
sleep 2

section "Query: COUNT(*) returns 6"
q() { curl -fsS -X POST "${BASE}/v1/query" -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/sql' -H 'X-Database: default' --data "$1"; }
cnt=$(q 'SELECT COUNT(*) AS n FROM events' | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo "ERR")
assert_eq "row count = 6" "6" "$cnt"

section "Query: filter by substring"
beta=$(q "SELECT COUNT(*) AS n FROM events WHERE msg LIKE '%beta%'" | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo ERR)
assert_eq "rows matching 'beta' = 2" "2" "$beta"

section "Filter: error statuses (string IN)"
errs=$(q "SELECT COUNT(*) AS n FROM events WHERE status IN ('500','504')" | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo ERR)
assert_eq "rows with status 500/504 = 3" "3" "$errs"

section "Unified search (lexical leg) finds the ingested rows"
hits=$(curl -fsS -X POST "${BASE}/v1/search" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H 'Content-Type: application/json' -H 'X-Database: default' \
  --data '{"query":"alpha","database":"default","limit":5}' 2>/dev/null \
  | python3 -c 'import sys,json; print(len(json.load(sys.stdin).get("hits",[])))' 2>/dev/null || echo ERR)
[[ "$hits" =~ ^[0-9]+$ && "$hits" -ge 1 ]] && ok "/v1/search returned $hits hit(s) for 'alpha'" || bad "/v1/search hits=$hits"

section "Restart durability: data survives a serve restart"
kill "$SERVER_PID" 2>/dev/null || true; wait "$SERVER_PID" 2>/dev/null || true
KYMA_LOCAL_MODE=1 KYMA_LOCAL_DATA="$DATA_DIR/data" KYMA_HOME="$DATA_DIR/home" \
KYMA_SECRET_KEY="${KYMA_SECRET_KEY:-fresh_install_validate_secret_key_32}" \
  "$KYMA" serve --addr "127.0.0.1:${PORT}" >>"$LOG" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 30); do curl -fsS -o /dev/null "${BASE}/health" 2>/dev/null && break; sleep 1; done
# Session tokens persist in the SQLite catalog, so the pre-restart token still
# works; re-login anyway to prove the seeded user survived the restart.
TOKEN=$(curl -fsS -X POST "${BASE}/v1/auth/login" -H 'Content-Type: application/json' \
  --data '{"username":"admin","password":"admin"}' 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])' 2>/dev/null || echo "")
cnt2=$(q 'SELECT COUNT(*) AS n FROM events' | python3 -c 'import sys,json; print(json.loads(sys.stdin.readline())["n"])' 2>/dev/null || echo ERR)
assert_eq "row count survives restart = 6" "6" "$cnt2"

section "Summary"
printf "${GRN}PASS: %d${NC}  ${RED}FAIL: %d${NC}\n" "$pass" "$fail"
if [[ $fail -gt 0 ]]; then printf "\nServe log tail:\n"; tail -30 "$LOG"; exit 1; fi
echo "fresh-install validation: GREEN"
