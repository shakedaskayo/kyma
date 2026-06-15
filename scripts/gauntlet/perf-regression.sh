#!/usr/bin/env bash
# perf-regression.sh — ingest at a tier-scaled volume against a live isolated
# engine, then measure reference-query latency and guard it against a ceiling
# (a coarse regression tripwire that's robust to shared-runner load — it catches
# pathological slowdowns, not micro-regressions).
#
# A live-engine ratcheting baseline (scripts/perf-baseline.sh + perf-check.sh +
# a committed baseline) is the release-tier mechanism for representative
# hardware; this self-contained variant keeps the family runnable in CI without
# a pre-deployed engine.
set -uo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; source "$DIR/_iso.sh"

TIER="${1#--tier=}"
[ -z "$TIER" -o "$TIER" = "--tier=" ] && { echo "Usage: $0 --tier=pr|nightly|weekly" >&2; exit 2; }
case "$TIER" in weekly) ROWS="${PERF_ROWS:-50000}" ;; *) ROWS="${PERF_ROWS:-10000}" ;; esac
BATCH=500
CEIL_MS="${PERF_CEIL_MS:-3000}"   # generous: trips only on pathological scans

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if ! iso_up 1; then iso_emit_json perf-regression "$TIER" "$START" false "isolated stack failed to start"; exit 0; fi

# Ingest ROWS rows in BATCH-sized NDJSON requests.
ing_ok=0; batches=$(( (ROWS + BATCH - 1) / BATCH )); rid=0
for b in $(seq 1 "$batches"); do
  : > "/tmp/perf-$ISO_SUF.ndjson"
  for r in $(seq 1 "$BATCH"); do
    rid=$((rid+1))
    printf '{"timestamp":"2026-04-19T10:%02d:%02dZ","req_id":%d,"row_id":%d,"message":"perf"}\n' \
      $(( rid % 60 )) $(( rid % 60 )) "$rid" $(( rid % 1000 )) >> "/tmp/perf-$ISO_SUF.ndjson"
  done
  st=$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 -X POST "http://$ISO_NODE_A/v1/ingest" \
    -H 'X-Database: default' -H 'X-Table: soak' -H 'Content-Type: application/x-ndjson' \
    --data-binary @"/tmp/perf-$ISO_SUF.ndjson")
  [ "$st" = "200" ] && ing_ok=$((ing_ok+1))
done

# Warm once, then measure a reference scan-aggregate 5×; take the median ms.
curl -s -o /dev/null -X POST "http://$ISO_NODE_A/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n, SUM(row_id) AS s FROM soak'
times=()
for k in 1 2 3 4 5; do
  t=$(curl -s -o /dev/null -w '%{time_total}' -X POST "http://$ISO_NODE_A/v1/query" \
    -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n, SUM(row_id) AS s FROM soak')
  ms=$(python3 -c "print(int(float('$t')*1000))")
  times+=("$ms")
done
median=$(printf '%s\n' "${times[@]}" | sort -n | sed -n '3p')
cnt=$(curl -s -X POST "http://$ISO_NODE_A/v1/query" -H 'X-Database: default' -H 'Content-Type: application/sql' --data 'SELECT COUNT(*) AS n FROM soak' | jq -r .n 2>/dev/null)

if [ "$ing_ok" = "$batches" ] && [ "${cnt:-0}" = "$rid" ] && [ "${median:-999999}" -lt "$CEIL_MS" ]; then
  iso_emit_json perf-regression "$TIER" "$START" true "ingested $rid rows in $batches batches; reference scan-aggregate median=${median}ms (< ${CEIL_MS}ms ceiling), COUNT=$cnt"
else
  iso_emit_json perf-regression "$TIER" "$START" false "ingest_ok=$ing_ok/$batches count=${cnt:-?} expected=$rid median=${median:-?}ms ceiling=${CEIL_MS}ms"
fi
