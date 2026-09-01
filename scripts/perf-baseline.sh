#!/usr/bin/env bash
# perf-baseline.sh — run reference workload against a live engine; emit metrics JSON.
#
# Usage: perf-baseline.sh [--engine-url=URL] [--target-rows=N]
#
# Phases:
#   1. Ingest the reference seed.ndjson repeatedly until target row count reached.
#   2. Warm-up: run reference queries once, discard timings.
#   3. Measure: run reference queries K=10 times, record per-call latency.
#   4. Emit metrics JSON to stdout.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ENGINE_URL="${PENSIEVE_HTTP_ADDR:-http://localhost:8080}"
TARGET_ROWS=100000
K_REPS=10

for arg in "$@"; do
  case "$arg" in
    --engine-url=*) ENGINE_URL="${arg#--engine-url=}" ;;
    --target-rows=*) TARGET_ROWS="${arg#--target-rows=}" ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

SEED_FILE="$ROOT/scripts/fixtures/perf-baseline/seed.ndjson"
QUERIES_FILE="$ROOT/scripts/fixtures/perf-baseline/queries.txt"
HARDWARE_LABEL="${GAUNTLET_HARDWARE_LABEL:-unknown}"

[ -f "$SEED_FILE" ] || { echo "missing $SEED_FILE" >&2; exit 2; }
[ -f "$QUERIES_FILE" ] || { echo "missing $QUERIES_FILE" >&2; exit 2; }

SEED_ROWS="$(grep -cv '^[[:space:]]*$' "$SEED_FILE")"
[ "$SEED_ROWS" -gt 0 ] || { echo "seed.ndjson has no rows" >&2; exit 2; }

ITER=$(( (TARGET_ROWS + SEED_ROWS - 1) / SEED_ROWS ))

# Use python3 for portable sub-second timestamps (date +%s.%N is Linux-only; macOS returns %N literally)
_now() { python3 -c "import time; print(time.time())"; }

# --- Phase 1: ingest ---
INGEST_START="$(_now)"
for i in $(seq 1 $ITER); do
  curl -fsS -X POST "$ENGINE_URL/v1/ingest" \
    -H "X-Database: obs" \
    -H "X-Table: otel_logs" \
    -H "Content-Type: application/x-ndjson" \
    --data-binary "@$SEED_FILE" \
    >/dev/null
done
INGEST_END="$(_now)"
INGEST_SECS="$(python3 -c "print(round($INGEST_END - $INGEST_START, 3))")"
TOTAL_ROWS=$(( SEED_ROWS * ITER ))
INGEST_RPS="$(python3 -c "print(round($TOTAL_ROWS / max($INGEST_SECS, 0.001), 1))")"

# Allow commit coordinator to flush.
sleep 3

# --- Phase 2: warm-up (discard timings) ---
while IFS=$'\t' read -r id endpoint content_type database body; do
  case "$id" in ''|\#*) continue ;; esac
  method="$(printf '%s' "$endpoint" | awk '{print $1}')"
  path="$(printf '%s' "$endpoint" | awk '{print $2}')"
  if [ "$method" = "GET" ]; then
    curl -sS "$ENGINE_URL$path" >/dev/null 2>&1 || true
  else
    curl -sS -X "$method" "$ENGINE_URL$path" \
      ${database:+-H "X-Database: $database"} \
      ${content_type:+-H "Content-Type: $content_type"} \
      --data-binary "$body" >/dev/null 2>&1 || true
  fi
done < "$QUERIES_FILE"

# --- Phase 3: measure ---
LATENCIES_TMPFILE="$(mktemp)"
trap 'rm -f "$LATENCIES_TMPFILE"' EXIT

for rep in $(seq 1 $K_REPS); do
  while IFS=$'\t' read -r id endpoint content_type database body; do
    case "$id" in ''|\#*) continue ;; esac
    method="$(printf '%s' "$endpoint" | awk '{print $1}')"
    path="$(printf '%s' "$endpoint" | awk '{print $2}')"
    Q_START="$(_now)"
    if [ "$method" = "GET" ]; then
      curl -sS "$ENGINE_URL$path" >/dev/null 2>&1 || true
    else
      curl -sS -X "$method" "$ENGINE_URL$path" \
        ${database:+-H "X-Database: $database"} \
        ${content_type:+-H "Content-Type: $content_type"} \
        --data-binary "$body" >/dev/null 2>&1 || true
    fi
    Q_END="$(_now)"
    python3 -c "print(round(($Q_END - $Q_START) * 1000, 3))" >> "$LATENCIES_TMPFILE"
  done < "$QUERIES_FILE"
done

# --- Phase 4: emit JSON ---
GIT_SHA="$(cd "$ROOT" && git rev-parse HEAD 2>/dev/null || echo unknown)"
CAPTURED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

python3 - "$LATENCIES_TMPFILE" <<PY
import json, sys
lats_file = sys.argv[1]
with open(lats_file) as f:
    lats = [float(line.strip()) for line in f if line.strip()]
lats.sort()
def percentile(p):
    if not lats: return 0.0
    k = int(round(p * (len(lats) - 1)))
    return lats[k]
p50 = percentile(0.5)
p99 = percentile(0.99)
print(json.dumps({
    "ingest_rps": $INGEST_RPS,
    "query_p50_ms": p50,
    "query_p99_ms": p99,
    "ingest_total_rows": $TOTAL_ROWS,
    "ingest_total_seconds": $INGEST_SECS,
    "captured_at": "$CAPTURED_AT",
    "hardware_label": "$HARDWARE_LABEL",
    "pensieve_git_sha": "$GIT_SHA",
}))
PY
