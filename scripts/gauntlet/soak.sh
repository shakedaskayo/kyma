#!/usr/bin/env bash
# soak.sh — sustained ingest + query against a live isolated engine, asserting
# steady-state (zero errors, node stays healthy, no data loss) over a
# tier-scaled window.
#
# Duration scales with the tier (override with SOAK_SECS). The release-scale
# 1-hour soak is the `weekly` aspiration in real CI on representative hardware;
# here weekly is a bounded stand-in so the family is runnable in-session.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; source "$DIR/_iso.sh"

TIER="${1#--tier=}"
[ -z "$TIER" -o "$TIER" = "--tier=" ] && { echo "Usage: $0 --tier=pr|nightly|weekly" >&2; exit 2; }
case "$TIER" in nightly) DUR="${SOAK_SECS:-30}" ;; weekly) DUR="${SOAK_SECS:-120}" ;; *) DUR="${SOAK_SECS:-10}" ;; esac

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if ! iso_up 1; then iso_emit_json soak "$TIER" "$START" false "isolated stack failed to start"; exit 0; fi

errors=0; iters=0; row=0; deadline=$(( SECONDS + DUR ))
while [ "$SECONDS" -lt "$deadline" ]; do
  iters=$((iters+1)); row=$((row+1))
  printf '{"timestamp":"2026-04-19T10:%02d:%02dZ","req_id":%d,"row_id":0,"message":"soak"}\n' \
    $(( row % 60 )) $(( row % 60 )) "$row" > "/tmp/soak-$ISO_SUF.ndjson"
  st=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST "http://$ISO_NODE_A/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: soak' -H 'Content-Type: application/x-ndjson' \
    --data-binary @"/tmp/soak-$ISO_SUF.ndjson")
  [ "$st" != "200" ] && errors=$((errors+1))
  qst=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST "http://$ISO_NODE_A/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM soak')
  [ "$qst" != "200" ] && errors=$((errors+1))
done
healthy=$(curl -sf "http://$ISO_NODE_A/health" >/dev/null 2>&1 && echo 1 || echo 0)
cnt=$(curl -s -X POST "http://$ISO_NODE_A/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM soak' | jq -r .n 2>/dev/null)

if [ "$errors" -eq 0 ] && [ "$healthy" = "1" ] && [ "${cnt:-0}" = "$row" ]; then
  iso_emit_json soak "$TIER" "$START" true "soak ${DUR}s: ${iters} ingest+query cycles, 0 errors, node healthy, COUNT=$cnt matches $row ingested (steady-state)"
else
  iso_emit_json soak "$TIER" "$START" false "soak ${DUR}s: errors=$errors healthy=$healthy count=${cnt:-?} expected=$row"
fi
