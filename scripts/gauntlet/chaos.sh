#!/usr/bin/env bash
# chaos.sh — placeholder. F2.4 replaces this with real chaos invocations.
set -euo pipefail

TIER="${1#--tier=}"
if [ -z "$TIER" ] || [ "$TIER" = "--tier=" ]; then
  echo "Usage: $0 --tier=pr|nightly|weekly" >&2
  exit 2
fi

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
sleep 0.1  # minimal time elapsed so finished_at differs from started_at
FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat <<EOF
{"family":"chaos","tier":"$TIER","started_at":"$START","finished_at":"$FINISH","skipped":true,"observations":["not yet implemented in F2.1 placeholder; F2.4 will implement"]}
EOF
