#!/usr/bin/env bash
# sim.sh — end-to-end engine simulation family.
#
# Drives the embedded-engine HTTP simulation suite: an in-process pensieve engine
# (SQLite catalog + local object store, no external infra) seeded with graphs,
# then exercised through the real /v1/query HTTP path — Cypher pattern matching,
# the full WHERE surface, aggregates, WITH, and CREATE/MERGE write → MATCH
# read-back round-trips. This simulates a client driving ingest + query + write
# against a live engine, deterministically and without Docker.
set -uo pipefail

TIER="${1#--tier=}"
if [ -z "$TIER" ] || [ "$TIER" = "--tier=" ]; then
  echo "Usage: $0 --tier=pr|nightly|weekly" >&2
  exit 2
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
LOG="$(mktemp)"
if cargo test --quiet -p pensieve-server --features test-support \
     --test cypher_query_it >"$LOG" 2>&1; then
  PASS=true
  N="$(grep -oE '[0-9]+ passed' "$LOG" | head -1 | grep -oE '[0-9]+' || echo '?')"
  OBS="embedded-engine end-to-end sim: ingest→query→write round-trips over /v1/query ($N cases) green (tier=$TIER)"
else
  PASS=false
  OBS="$(tail -3 "$LOG" | tr '\n\"' '  ')"
fi
rm -f "$LOG"
FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat <<EOF
{"family":"sim","tier":"$TIER","started_at":"$START","finished_at":"$FINISH","pass":$PASS,"observations":["$OBS"]}
EOF
