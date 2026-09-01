#!/usr/bin/env bash
# compose-smoke.sh — docker-compose fresh-install smoke test.
#
# Validates the SHIPPED CONTAINER path: builds the pensieve image from the Dockerfile
# and brings up the full compose stack (Postgres + MinIO + the engine) on an
# ISOLATED project/ports (scripts/compose.smoke-override.yml), then asserts the
# engine serves over HTTP from a clean state — health, admin login, ingest,
# query (count + filter), and search — exactly like the local single-binary
# `fresh-install-validate.sh`, but exercising the Docker image + compose wiring
# instead of a `cargo build`. Tears the stack down on exit.
#
# Safe to run alongside a developer's live `docker compose up`: the override
# renames every container (pensieve-smoke-*) and remaps every host port, so there is
# no collision with the default-named stack.
#
# Usage: scripts/compose-smoke.sh

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROJECT="pensieve-smoke"
COMPOSE=(docker compose -p "$PROJECT" -f docker-compose.yml -f scripts/compose.smoke-override.yml)
BASE="http://127.0.0.1:18090"

if [[ -t 1 ]]; then RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"; else RED=""; GRN=""; BLU=""; NC=""; fi
pass=0; fail=0
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
ok()  { printf "  ${GRN}PASS${NC} %s\n" "$*"; pass=$((pass+1)); }
bad() { printf "  ${RED}FAIL${NC} %s\n" "$*"; fail=$((fail+1)); }
assert_eq() { [[ "$2" == "$3" ]] && ok "$1" || bad "$1 (expected '$2', got '$3')"; }

cleanup() {
  section "Tear down"
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true
  echo "  stack removed"
}
trap cleanup EXIT

section "Build + start the compose stack (isolated project '$PROJECT')"
# redpanda is not needed for the smoke; scale it to 0 to save time.
if ! "${COMPOSE[@]}" up -d --build --scale redpanda=0; then
  echo "compose up failed" >&2
  exit 1
fi

section "Wait for the engine to be healthy on $BASE"
healthy=0
for i in $(seq 1 90); do
  if curl -fsS "$BASE/health" >/dev/null 2>&1; then healthy=1; break; fi
  sleep 2
done
if [[ "$healthy" != "1" ]]; then
  bad "/health never came up"
  echo "--- engine logs (tail) ---"
  "${COMPOSE[@]}" logs --tail 80 pensieve 2>&1 || true
  exit 1
fi
ok "/health is ok"

section "Admin login (seeded admin/pensieve_dev → access token)"
TOKEN="$(curl -fsS -X POST "$BASE/v1/auth/login" \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"pensieve_dev"}' 2>/dev/null \
  | python3 -c 'import sys,json; print(json.load(sys.stdin).get("access_token",""))' 2>/dev/null)"
if [[ -n "$TOKEN" ]]; then ok "logged in as admin"; else bad "login returned no access token"; fi
AUTH=(-H "Authorization: Bearer $TOKEN")

section "Ingest telemetry rows (table auto-created)"
ING="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/ingest" \
  -H 'X-Database: default' -H 'X-Table: smoke' -H 'Content-Type: application/x-ndjson' \
  --data-binary $'{"ts":"2026-04-19T10:00:00Z","service":"alpha","status":"200"}\n{"ts":"2026-04-19T10:01:00Z","service":"beta","status":"500"}\n{"ts":"2026-04-19T10:02:00Z","service":"beta","status":"504"}\n' 2>/dev/null)"
if echo "$ING" | grep -q '"rows_ingested":3'; then ok "ingest accepted (3 rows)"; else bad "ingest response: $ING"; fi

section "Query: COUNT(*) = 3"
CNT="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/query" -H 'X-Database: default' \
  -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM smoke' 2>/dev/null \
  | python3 -c 'import sys,json
rows=[json.loads(l) for l in sys.stdin if l.strip()]
print(rows[0].get("n","?") if rows else "?")' 2>/dev/null)"
assert_eq "row count" "3" "$CNT"

section "Query: filter (status IN 500/504) = 2"
ERRS="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/query" -H 'X-Database: default' \
  -H 'Content-Type: application/sql' --data "SELECT COUNT(*) AS n FROM smoke WHERE status IN ('500','504')" 2>/dev/null \
  | python3 -c 'import sys,json
rows=[json.loads(l) for l in sys.stdin if l.strip()]
print(rows[0].get("n","?") if rows else "?")' 2>/dev/null)"
assert_eq "error-status rows" "2" "$ERRS"

section "Unified search finds an ingested row"
HITS="$(curl -fsS "${AUTH[@]}" -X POST "$BASE/v1/search" \
  -H 'Content-Type: application/json' \
  -d '{"query":"alpha","database":"default","limit":10}' 2>/dev/null \
  | python3 -c 'import sys,json
try:
    d=json.load(sys.stdin); r=d.get("results") or d.get("hits") or []
    print(len(r))
except Exception:
    print("0")' 2>/dev/null)"
if [[ "${HITS:-0}" -ge 1 ]]; then ok "/v1/search returned $HITS hit(s)"; else bad "/v1/search returned no hits"; fi

section "Summary"
printf "PASS: %d  FAIL: %d\n" "$pass" "$fail"
if [[ "$fail" -eq 0 ]]; then
  printf "${GRN}compose smoke: GREEN${NC}\n"
  exit 0
else
  printf "${RED}compose smoke: FAILED${NC}\n"
  exit 1
fi
