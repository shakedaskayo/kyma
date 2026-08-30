#!/usr/bin/env bash
# Graph differential gate (S0.5).
#
# Boots the dev stack + pensieve, then runs the `graph_differential` binary from
# crates/pensieve-graph-testkit: 3 synthetic topologies (power_law 2k nodes,
# grid 20x20, cyclic rings) are ingested, registered as stored graphs, and
# ~12 forward-chain Cypher patterns each are evaluated through
# POST /v1/query (application/x-cypher) AND the petgraph oracle. Any set
# divergence fails the gate. This is the correctness contract for the
# EXISTING Cypher subset; S3's path operator extends the same harness.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Catalog DB + bucket are overridable (PENSIEVE_GRAPH_DIFF_PG_DB / _BUCKET) so the
# gate can run isolated from other tests sharing the docker stack; defaults
# match the house scripts (test-graph.sh et al.).
PG_DB="${PENSIEVE_GRAPH_DIFF_PG_DB:-pensieve}"
BUCKET="${PENSIEVE_GRAPH_DIFF_BUCKET:-pensieve}"

export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:5433/${PG_DB}"
export PENSIEVE_S3_ENDPOINT="http://localhost:9000"
export PENSIEVE_S3_BUCKET="$BUCKET"
export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
export PENSIEVE_S3_PATH_STYLE="true"
export PENSIEVE_S3_ALLOW_HTTP="true"
export PENSIEVE_HTTP_ADDR="${PENSIEVE_HTTP_ADDR:-127.0.0.1:8080}"
export PENSIEVE_GRPC_ADDR=off
export PENSIEVE_OTLP_ADDR=off
export PENSIEVE_STAGING_DISABLED=1    # rows visible immediately after ingest
# Dev-only credentials encryption key (same value as docker-compose.yml).
export PENSIEVE_SECRET_KEY="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
export PENSIEVE_COMPACTION_POLL_SECS="3600"
export PENSIEVE_RETENTION_POLL_SECS="3600"
export PENSIEVE_PHYSICAL_GC_POLL_SECS="3600"
export RUST_LOG="${RUST_LOG:-warn}"

export PENSIEVE_BASE_URL="http://${PENSIEVE_HTTP_ADDR}"
export PENSIEVE_DATABASE="default"

# Both pensieve-bin (server) and pensieve-cli emit a binary named `pensieve`, colliding in
# target/debug. Force a re-uplift of each (rm + cheap rebuild, no recompile)
# and copy them to distinct paths.
BIN_DIR="$(mktemp -d /tmp/pensieve-graph-diff.XXXXXX)"
SERVER_BIN="$BIN_DIR/pensieve-server"
export PENSIEVE_CLI_BIN="$BIN_DIR/pensieve-cli"

HTTP_BASE="$PENSIEVE_BASE_URL"
LOG_FILE="/tmp/pensieve-graph-differential.log"
SERVER_PID=""

if [[ -t 1 ]]; then
    RED="\033[31m"; GRN="\033[32m"; BLU="\033[34m"; NC="\033[0m"
else
    RED=""; GRN=""; BLU=""; NC=""
fi
section() { printf "\n${BLU}==> %s${NC}\n" "$*"; }
cleanup() { [[ -n "${SERVER_PID:-}" ]] && kill -9 "$SERVER_PID" 2>/dev/null || true; }
trap cleanup EXIT

if ! docker exec pensieve-postgres pg_isready -U pensieve -d pensieve >/dev/null 2>&1; then
    printf "${RED}docker-compose stack not up.${NC}\n"; exit 2
fi

section "Build (server + cli + differential bin)"
cargo build -q -p pensieve-graph-testkit
rm -f target/debug/pensieve
cargo build -q -p pensieve-cli
cp target/debug/pensieve "$PENSIEVE_CLI_BIN"
rm -f target/debug/pensieve
cargo build -q -p pensieve-bin
cp target/debug/pensieve "$SERVER_BIN"
"$PENSIEVE_CLI_BIN" --help 2>&1 | grep -q "create-graph" \
    || { printf "${RED}%s is not the admin CLI${NC}\n" "$PENSIEVE_CLI_BIN"; exit 2; }

if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then
    printf "${RED}something already listens on %s — stop it first (e.g. the docker-compose pensieve container)${NC}\n" "$HTTP_BASE"
    exit 2
fi

section "Reset + start pensieve (pg db: $PG_DB, bucket: $BUCKET)"
docker exec pensieve-postgres psql -U pensieve -d postgres -qc "CREATE DATABASE ${PG_DB}" >/dev/null 2>&1 || true
docker exec pensieve-postgres psql -U pensieve -d "$PG_DB" -qc "DROP SCHEMA public CASCADE; CREATE SCHEMA public;" >/dev/null 2>&1
docker exec pensieve-minio mc rm --recursive --force "local/${BUCKET}" >/dev/null 2>&1 || true
docker exec pensieve-minio mc mb --ignore-existing "local/${BUCKET}" >/dev/null
"$SERVER_BIN" >"$LOG_FILE" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 30); do
    if curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then break; fi; sleep 1
done
if ! curl -sf "$HTTP_BASE/health" >/dev/null 2>&1; then
    printf "${RED}pensieve did not become healthy${NC}\n"; tail -30 "$LOG_FILE"; exit 2
fi

section "Run differential: engine (Cypher over HTTP) vs petgraph oracle"
if ./target/debug/graph_differential; then
    printf "\n${GRN}GRAPH DIFFERENTIAL GATE: PASS${NC}\n"
    exit 0
else
    rc=$?
    printf "\n${RED}GRAPH DIFFERENTIAL GATE: FAIL (rc=%d)${NC}\n" "$rc"
    tail -30 "$LOG_FILE"
    exit "$rc"
fi
