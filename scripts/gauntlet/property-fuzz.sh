#!/usr/bin/env bash
# property-fuzz.sh — property-based + differential-oracle correctness family.
#
# Runs the deterministic, infra-free property/fuzz suites: the graph traversal
# differential oracle (vs petgraph), the CSR topology differential, and the
# retrieval recall/NDCG-vs-exact-oracle harness. These exercise the engine
# under randomized/adversarial inputs and are the program's correctness ratchet.
#
# Higher tiers widen the proptest case budget (PROPTEST_CASES); the PR tier runs
# the default budget so it stays fast in CI.
set -uo pipefail

TIER="${1#--tier=}"
if [ -z "$TIER" ] || [ "$TIER" = "--tier=" ]; then
  echo "Usage: $0 --tier=pr|nightly|weekly" >&2
  exit 2
fi
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

case "$TIER" in
  nightly|weekly) export PROPTEST_CASES="${PROPTEST_CASES:-2048}" ;;
esac

START="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
LOG="$(mktemp)"
if cargo test --quiet \
     -p kyma-graph-testkit -p kyma-graph-topo -p kyma-retrieval-eval \
     >"$LOG" 2>&1; then
  PASS=true
  OBS="graph differential-vs-petgraph + CSR topo differential + retrieval recall/NDCG-vs-oracle suites green (tier=$TIER)"
else
  PASS=false
  OBS="$(tail -3 "$LOG" | tr '\n\"' '  ')"
fi
rm -f "$LOG"
FINISH="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

cat <<EOF
{"family":"property-fuzz","tier":"$TIER","started_at":"$START","finished_at":"$FINISH","pass":$PASS,"observations":["$OBS"]}
EOF
