#!/usr/bin/env bash
# chaos.sh — node-loss resilience against a live isolated 2-node stack.
#
# Proves the plan's invariants — object storage is the only source of truth +
# query nodes are stateless — under failure: ingest committed rows via both
# nodes, hard-kill one node, then verify the survivor still serves every
# committed row (no loss) and the killed node can be replaced and rejoin.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; source "$DIR/_iso.sh"

TIER="${1#--tier=}"
[ -z "$TIER" -o "$TIER" = "--tier=" ] && { echo "Usage: $0 --tier=pr|nightly|weekly" >&2; exit 2; }
case "$TIER" in weekly) N="${CHAOS_REQS:-60}" ;; *) N="${CHAOS_REQS:-30}" ;; esac

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if ! iso_up 2; then iso_emit_json chaos "$TIER" "$START" false "isolated 2-node stack failed to start"; exit 0; fi

# Ingest N committed rows, split across both nodes.
ok=0
for i in $(seq 1 "$N"); do
  printf '{"timestamp":"2026-04-19T10:%02d:00Z","req_id":%d,"row_id":0,"message":"chaos"}\n' $(( i % 60 )) "$i" > "/tmp/chaos-$ISO_SUF.ndjson"
  addr=$([ $((i%2)) -eq 0 ] && echo "$ISO_NODE_A" || echo "$ISO_NODE_B")
  st=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST "http://$addr/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: soak' -H 'Content-Type: application/x-ndjson' \
    --data-binary @"/tmp/chaos-$ISO_SUF.ndjson")
  [ "$st" = "200" ] && ok=$((ok+1))
done

# Hard-kill node A. The survivor (B) must still serve every committed row.
kill -9 "$ISO_PID_A" 2>/dev/null || true; ISO_PID_A=""
sleep 2
b_health=$(curl -sf "http://$ISO_NODE_B/health" >/dev/null 2>&1 && echo 1 || echo 0)
b_cnt=$(curl -s -X POST "http://$ISO_NODE_B/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM soak' | jq -r .n 2>/dev/null)

# Replacement node C rejoins the same catalog + store (stateless add).
KYMA_HTTP_ADDR="127.0.0.1:18182" "$ISO_KYMA" >"/tmp/chaos-c-$ISO_SUF.log" 2>&1 & PID_C=$!
disown "$PID_C" 2>/dev/null || true
c_ok=0
for i in $(seq 1 30); do curl -sf "http://127.0.0.1:18182/health" >/dev/null 2>&1 && { c_ok=1; break; }; sleep 1; done
c_cnt=$(curl -s -X POST "http://127.0.0.1:18182/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM soak' | jq -r .n 2>/dev/null)
kill -9 "$PID_C" 2>/dev/null || true

if [ "$ok" = "$N" ] && [ "$b_health" = "1" ] && [ "${b_cnt:-0}" = "$N" ] && [ "$c_ok" = "1" ] && [ "${c_cnt:-0}" = "$N" ]; then
  iso_emit_json chaos "$TIER" "$START" true "killed node A after $N committed ingests; survivor B served COUNT=$b_cnt (no loss); fresh node C rejoined + served COUNT=$c_cnt"
else
  iso_emit_json chaos "$TIER" "$START" false "ingested=$ok/$N B_health=$b_health B_count=${b_cnt:-?} C_up=$c_ok C_count=${c_cnt:-?} (expected $N)"
fi
