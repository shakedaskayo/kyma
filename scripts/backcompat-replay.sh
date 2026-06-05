#!/usr/bin/env bash
# backcompat-replay.sh — replay a fixed query set, compare to fixture's
# expected hashes.
#
# Usage:
#   backcompat-replay.sh [--record] <engine-url> <fixture-dir> <queries-file>
#
# --record writes expected-hashes.txt into the fixture instead of
# comparing. Used once when a fixture is first captured.
#
# Non-determinism handling:
#   - JSON error responses: "request_id" field is stripped before hashing
#     (it is a per-request UUID and changes every call).
#   - JSON top-level semver "version" fields are stripped (/health reports
#     the build version, which changes on every release bump).
#   - Prometheus /metrics responses: numeric values on metric lines are
#     normalised to "0" so the hash reflects metric *shape* (which counters
#     exist) rather than their ever-changing values.
set -euo pipefail

RECORD=0
if [ "${1:-}" = "--record" ]; then
  RECORD=1; shift
fi

ENGINE_URL="${1:?engine URL required}"
FIXTURE_DIR="${2:?fixture directory required}"
QUERIES_FILE="${3:?queries file required}"

run_one() {
  local id="$1" endpoint="$2" content_type="$3" database="$4" body="$5"
  local method path
  method="$(printf '%s' "$endpoint" | awk '{print $1}')"
  path="$(printf '%s' "$endpoint" | awk '{print $2}')"
  if [ "$method" = "GET" ]; then
    curl -sS "$ENGINE_URL$path"
  else
    local -a curl_args=(-sS -X "$method" "$ENGINE_URL$path")
    if [ "$database" != "-" ]; then curl_args+=(-H "X-Database: $database"); fi
    if [ "$content_type" != "-" ]; then curl_args+=(-H "Content-Type: $content_type"); fi
    curl "${curl_args[@]}" --data-binary "$body"
  fi
}

hash_body() {
  # Normalise response body before hashing to remove non-deterministic fields:
  #   1. JSON responses: delete "request_id" key (per-request UUID in errors).
  #   2. Prometheus text format: replace numeric values with "0" (counters
  #      change on every request) AND sort the output lines so that metric
  #      registration order differences do not affect the hash. Only the
  #      set of metric names/labels (the "shape") is compared.
  python3 -c "
import sys, json, hashlib, re

raw = sys.stdin.read()

# Try JSON normalisation first.
try:
    obj = json.loads(raw)
    # Recursively strip 'request_id' from any dict in the tree.
    def strip_request_id(o):
        if isinstance(o, dict):
            return {k: strip_request_id(v) for k, v in o.items() if k != 'request_id'}
        if isinstance(o, list):
            return [strip_request_id(i) for i in o]
        return o
    obj = strip_request_id(obj)
    # /health embeds the build version — per-release metadata, not contract.
    # Strip it (top level only, semver-shaped only) so a version bump doesn't
    # diverge every fixture.
    if isinstance(obj, dict) and isinstance(obj.get('version'), str) \
            and re.fullmatch(r'v?\d+\.\d+\.\d+([.-].*)?', obj['version']):
        obj = {k: v for k, v in obj.items() if k != 'version'}
    norm = json.dumps(obj, sort_keys=True, separators=(',', ':'))
except Exception:
    # Not JSON — treat as Prometheus text format (or plain text).
    # Strategy:
    #   a) Normalise numeric values on metric lines to '0' (counters change).
    #   b) Sort all lines so metric registration order doesn't matter.
    #   c) Strip blank lines (they appear between metric families and their
    #      count varies with registration order).
    lines = []
    for line in raw.splitlines():
        stripped = line.strip()
        if stripped == '':
            continue
        if stripped.startswith('#'):
            lines.append(stripped)
        else:
            # Normalise the trailing value (and optional timestamp) to '0'.
            normalized = re.sub(r'(\s+)\S+(\s+\S+)?\s*$', r'\g<1>0', stripped)
            lines.append(normalized)
    lines.sort()
    norm = '\n'.join(lines)

print(hashlib.sha256(norm.encode()).hexdigest())
"
}

HASHES_FILE="$FIXTURE_DIR/expected-hashes.txt"
TMP_OUT="$(mktemp)"
trap 'rm -f "$TMP_OUT"' EXIT

: > "$TMP_OUT"
while IFS=$'\t' read -r id endpoint content_type database body; do
  case "$id" in ''|\#*) continue ;; esac
  resp="$(run_one "$id" "$endpoint" "$content_type" "$database" "$body")"
  h="$(printf '%s' "$resp" | hash_body)"
  printf '%s\t%s\n' "$id" "$h" >> "$TMP_OUT"
done < "$QUERIES_FILE"

if [ "$RECORD" = "1" ]; then
  mv "$TMP_OUT" "$HASHES_FILE"
  echo "recorded expected hashes to $HASHES_FILE"
  exit 0
fi

if [ ! -f "$HASHES_FILE" ]; then
  echo "FAIL: no expected-hashes.txt in fixture (run with --record first)" >&2
  exit 2
fi

if diff -u "$HASHES_FILE" "$TMP_OUT" >/dev/null; then
  echo "OK: $(wc -l < "$TMP_OUT" | tr -d ' ') queries match fixture"
  exit 0
else
  echo "FAIL: replay diverged from fixture:" >&2
  diff -u "$HASHES_FILE" "$TMP_OUT" >&2
  exit 3
fi
