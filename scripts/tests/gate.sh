#!/usr/bin/env bash
# gate.sh — per-stage merge gate for the scale program (S-series).
#
# Usage: GATE=s0 scripts/tests/gate.sh
#    or: scripts/tests/gate.sh s0
#
# A stage's gate is the single command that must be green before the next
# stage of the program starts. It runs (1) the workspace test suite,
# (2) the stage's named integration scripts, and (3) the stage's benchmark
# checks against committed baselines. Stages are cumulative: s1 runs s0's
# scripts too, so a later stage can never silently regress an earlier gate.
#
# Add scripts to a stage by appending to the STAGE_SCRIPTS_<n> arrays below.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

STAGE="${GATE:-${1:-}}"
if [ -z "$STAGE" ]; then
  echo "Usage: GATE=s0|s1|s2|s3|s4 $0   (or: $0 s0)" >&2
  exit 2
fi

# --- Stage manifests (cumulative) -------------------------------------------

# S0: foundations — the pre-existing engine suite must stay green, plus the
# new eval/testkit determinism checks as they land.
STAGE_SCRIPTS_0=(
  scripts/e2e-test.sh
  scripts/test-pushdown.sh
  scripts/test-compaction.sh
  scripts/test-retention.sh
  scripts/test-a-two-nodes.sh
  scripts/test-scale-out.sh
  scripts/chaos-test.sh
)

# S1: retrieval intelligence — populated as S1 phases land.
STAGE_SCRIPTS_1=(
  scripts/retrieval-bench.sh
)

# S2: engine core — populated as S2 phases land.
STAGE_SCRIPTS_2=(
  scripts/backcompat-replay.sh
)

# S3: graph + memory — populated as S3 phases land.
STAGE_SCRIPTS_3=(
)

# S4: final gauntlet (everything + soak tiers, run before merge to main).
STAGE_SCRIPTS_4=(
)

case "$STAGE" in
  s0) LEVELS=(0) ;;
  s1) LEVELS=(0 1) ;;
  s2) LEVELS=(0 1 2) ;;
  s3) LEVELS=(0 1 2 3) ;;
  s4) LEVELS=(0 1 2 3 4) ;;
  *) echo "invalid stage: $STAGE (must be s0..s4)" >&2; exit 2 ;;
esac

# --- Run ---------------------------------------------------------------------

FAILED=()
PASSED=()
SKIPPED=()

run_step() {
  local name="$1"; shift
  echo ""
  echo "=== gate[$STAGE] :: $name ==="
  local start=$SECONDS
  if "$@"; then
    PASSED+=("$name ($((SECONDS - start))s)")
  else
    FAILED+=("$name")
  fi
}

# Run tests the way .github/workflows/rust-tests.yml does — per-crate for the
# testcontainer-backed suites — instead of one monolithic `cargo test
# --workspace`. A single workspace run launches every crate's Postgres/MinIO
# containers at once and thrashes Docker (StartupTimeout flakes); the per-crate
# split is what keeps CI green and makes "green locally" predict "green in CI".
run_step "compile all test targets" \
  cargo test --workspace --locked --no-run \
  --exclude app --exclude pensieve-bin --exclude pensieve-ingest-kafka
run_step "unit suites (no infra)" \
  cargo test --locked \
  -p pensieve-core -p pensieve-storage -p pensieve-kql -p pensieve-memory \
  -p pensieve-catalog-sqlite -p pensieve-retrieval-eval -p pensieve-graph-testkit \
  -p pensieve-graph-topo
run_step "catalog integration (testcontainers)" \
  cargo test --locked -p pensieve-catalog -- --test-threads=2
run_step "server integration (testcontainers)" \
  cargo test --locked -p pensieve-server --features "test-support cloud-auth" -- --test-threads=2
run_step "mcp integration (testcontainers)" \
  cargo test --locked -p pensieve-mcp --features "pensieve-server/test-support" -- --test-threads=2
run_step "jobs integration (testcontainers)" \
  cargo test --locked -p pensieve-jobs -- --test-threads=2
# Retrieval engine: ANN recall-vs-oracle (ann_topk_it, testcontainers), BM25
# round-trip (bm25_topk_it), and the S1 BM25-vs-LIKE NDCG@10 gate.
run_step "exec retrieval (testcontainers)" \
  cargo test --locked -p pensieve-exec -- --test-threads=2
# Index-build activation scheduler (index_scheduler_it, testcontainers).
run_step "compaction integration (testcontainers)" \
  cargo test --locked -p pensieve-compaction -- --test-threads=2
# Local-mode (SQLite) ANN + BM25 sidecar activation parity.
run_step "local index activation (sqlite)" \
  cargo test --locked -p pensieve-local --test local_index_activation_it

# Each integration script boots its own server(s) and resets the shared
# Postgres+MinIO stack. A prior script's server can still be draining (and
# holding its port) when the next one starts, causing "server never healthy"
# flakes. Between scripts, kill any lingering engine processes and wait for the
# common ports to free so every script starts from a clean slate.
reap_servers() {
  pkill -9 -f "target/debug/pensieve$" 2>/dev/null || true
  pkill -9 -f "target/debug/pensieve " 2>/dev/null || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ! lsof -ti tcp:8080,tcp:9090,tcp:18080,tcp:18081 >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
}

for lvl in "${LEVELS[@]}"; do
  arr="STAGE_SCRIPTS_${lvl}[@]"
  for script in "${!arr+"${!arr}"}"; do
    [ -z "$script" ] && continue
    if [ ! -x "$script" ] && [ ! -f "$script" ]; then
      SKIPPED+=("$script (not present yet)")
      continue
    fi
    reap_servers
    run_step "$script" bash "$script"
  done
done
reap_servers

# S4 additionally runs the full gauntlet nightly tier.
if [ "$STAGE" = "s4" ]; then
  run_step "gauntlet --tier=nightly" bash scripts/gauntlet.sh --tier=nightly
fi

# --- Summary -----------------------------------------------------------------

echo ""
echo "================ gate[$STAGE] summary ================"
for p in "${PASSED[@]:-}";  do [ -n "$p" ] && echo "  PASS  $p"; done
for s in "${SKIPPED[@]:-}"; do [ -n "$s" ] && echo "  SKIP  $s"; done
for f in "${FAILED[@]:-}";  do [ -n "$f" ] && echo "  FAIL  $f"; done

if [ "${#FAILED[@]}" -gt 0 ]; then
  echo "gate[$STAGE]: FAILED (${#FAILED[@]} step(s))"
  exit 1
fi
echo "gate[$STAGE]: GREEN"
