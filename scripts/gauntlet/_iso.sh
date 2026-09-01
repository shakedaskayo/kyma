#!/usr/bin/env bash
# _iso.sh — shared helpers for gauntlet families that need a live engine.
#
# Spins up a *throwaway, isolated* stack (Postgres + MinIO in containers with
# unique names/ports + a private data dir) and one or two pensieve nodes against it,
# then tears everything down. Never touches a shared/compose stack, so it is
# safe to run on a developer machine without wiping live data.
#
# Usage (source this file):
#   source "$(dirname "$0")/_iso.sh"
#   iso_up 1        # or: iso_up 2   (number of pensieve nodes)
#   ... use $ISO_NODE_A (+ $ISO_NODE_B) HTTP addrs, $ISO_PG / $ISO_MINIO names ...
#   iso_down        # (also runs on EXIT via the trap installed by iso_up)
#
# The engine binary is expected at target/debug/pensieve (+ target/debug/pensieve-cli).

ISO_SUF=""; ISO_PG=""; ISO_MINIO=""; ISO_DATADIR=""
ISO_PGPORT=""; ISO_S3PORT=""
ISO_NODE_A=""; ISO_NODE_B=""
ISO_PID_A=""; ISO_PID_B=""
ISO_LOG_A=""; ISO_LOG_B=""
ISO_ROOT=""
# Resolved binary paths (release-first, debug fallback) — exported so family
# scripts (e.g. chaos's replacement node) reuse the same binary the stack ran.
ISO_PENSIEVE=""; ISO_PENSIEVE_CLI=""

# _iso_resolve_bins — locate the engine + CLI binaries, preferring an
# optimized release build (what CI builds) but falling back to debug (local
# dev). Returns non-zero with a build hint if either is missing.
_iso_resolve_bins() {
  local p
  for p in target/release/pensieve target/debug/pensieve; do
    [ -x "$p" ] && { ISO_PENSIEVE="$p"; break; }
  done
  for p in target/release/pensieve-cli target/debug/pensieve-cli; do
    [ -x "$p" ] && { ISO_PENSIEVE_CLI="$p"; break; }
  done
  [ -n "$ISO_PENSIEVE" ] || { echo "iso: pensieve binary not found (build: cargo build [--release] -p pensieve-bin)" >&2; return 1; }
  [ -n "$ISO_PENSIEVE_CLI" ] || { echo "iso: pensieve-cli binary not found (build: cargo build [--release] -p pensieve-cli)" >&2; return 1; }
  return 0
}

iso_down() {
  [ -n "${ISO_PID_A:-}" ] && kill -9 "$ISO_PID_A" 2>/dev/null || true
  [ -n "${ISO_PID_B:-}" ] && kill -9 "$ISO_PID_B" 2>/dev/null || true
  [ -n "${ISO_PG:-}" ] && docker rm -f "$ISO_PG" >/dev/null 2>&1 || true
  [ -n "${ISO_MINIO:-}" ] && docker rm -f "$ISO_MINIO" >/dev/null 2>&1 || true
  [ -n "${ISO_DATADIR:-}" ] && rm -rf "$ISO_DATADIR" 2>/dev/null || true
}

# iso_up <n_nodes>  — returns 0 on success, non-zero (with a message on stderr)
# if the stack or nodes never came up. Installs an EXIT trap calling iso_down.
iso_up() {
  local n="${1:-1}"
  ISO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
  cd "$ISO_ROOT"
  _iso_resolve_bins || return 9
  ISO_SUF="g$$_$RANDOM"
  ISO_PG="pensieve-gaunt-pg-$ISO_SUF"; ISO_MINIO="pensieve-gaunt-minio-$ISO_SUF"
  ISO_PGPORT=$(( 5500 + (RANDOM % 400) ))
  ISO_S3PORT=$(( 9100 + (RANDOM % 400) ))
  ISO_DATADIR="/tmp/pensieve-gaunt-data-$ISO_SUF"
  ISO_LOG_A="/tmp/pensieve-gaunt-a-$ISO_SUF.log"; ISO_LOG_B="/tmp/pensieve-gaunt-b-$ISO_SUF.log"
  ISO_NODE_A="127.0.0.1:18180"; ISO_NODE_B="127.0.0.1:18181"
  trap iso_down EXIT

  mkdir -p "$ISO_DATADIR/pensieve"   # pre-create the 'pensieve' bucket as a directory
  docker run -d --name "$ISO_PG" -e POSTGRES_USER=pensieve -e POSTGRES_PASSWORD=pensieve_dev \
    -e POSTGRES_DB=pensieve -p "$ISO_PGPORT:5432" pgvector/pgvector:pg16 >/dev/null 2>&1 || return 10
  docker run -d --name "$ISO_MINIO" -e MINIO_ROOT_USER=pensieve_admin \
    -e MINIO_ROOT_PASSWORD=pensieve_admin_dev -p "$ISO_S3PORT:9000" \
    -v "$ISO_DATADIR:/data" minio/minio:latest server /data >/dev/null 2>&1 || return 11

  local i
  for i in $(seq 1 40); do
    docker exec "$ISO_PG" pg_isready -U pensieve -d pensieve >/dev/null 2>&1 && break; sleep 1
  done
  docker exec "$ISO_PG" pg_isready -U pensieve -d pensieve >/dev/null 2>&1 || { echo "iso: postgres not ready" >&2; return 12; }
  for i in $(seq 1 25); do
    curl -sf "http://localhost:$ISO_S3PORT/minio/health/ready" >/dev/null 2>&1 && break; sleep 1
  done

  export PENSIEVE_CATALOG_URL="postgres://pensieve:pensieve_dev@localhost:$ISO_PGPORT/pensieve"
  export PENSIEVE_S3_ENDPOINT="http://localhost:$ISO_S3PORT"
  export PENSIEVE_S3_BUCKET="pensieve"
  export PENSIEVE_S3_ACCESS_KEY_ID="pensieve_admin"
  export PENSIEVE_S3_SECRET_ACCESS_KEY="pensieve_admin_dev"
  export PENSIEVE_S3_PATH_STYLE="true"; export PENSIEVE_S3_ALLOW_HTTP="true"
  export PENSIEVE_SECRET_KEY="dev_secret_key_dev_secret_key_32"
  export PENSIEVE_GRPC_ADDR="off"; export PENSIEVE_OTLP_ADDR="off"
  export RUST_LOG="${RUST_LOG:-warn}"

  # Bootstrap catalog (migrates the fresh PG) + the standard load table.
  "$ISO_PENSIEVE_CLI" create-database default --if-not-exists >/dev/null 2>&1 || return 13
  "$ISO_PENSIEVE_CLI" create-table --db default --name soak \
    --schema 'timestamp:timestamp,req_id:int,row_id:int,message:string' >/dev/null 2>&1 || true

  # `disown` so the shell doesn't print a job-control "Killed: 9" line to stderr
  # when iso_down kills the node — that line would otherwise become the family
  # script's last output and clobber the JSON the gauntlet parses (`2>&1|tail -1`).
  PENSIEVE_HTTP_ADDR="$ISO_NODE_A" "$ISO_PENSIEVE" >"$ISO_LOG_A" 2>&1 & ISO_PID_A=$!
  disown "$ISO_PID_A" 2>/dev/null || true
  if [ "$n" -ge 2 ]; then
    PENSIEVE_HTTP_ADDR="$ISO_NODE_B" "$ISO_PENSIEVE" >"$ISO_LOG_B" 2>&1 & ISO_PID_B=$!
    disown "$ISO_PID_B" 2>/dev/null || true
  fi
  _iso_wait "$ISO_NODE_A" || { echo "iso: node A unhealthy" >&2; tail -15 "$ISO_LOG_A" >&2; return 14; }
  if [ "$n" -ge 2 ]; then
    _iso_wait "$ISO_NODE_B" || { echo "iso: node B unhealthy" >&2; tail -15 "$ISO_LOG_B" >&2; return 15; }
  fi
  return 0
}

_iso_wait() { local i; for i in $(seq 1 30); do curl -sf "http://$1/health" >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }

# iso_emit_json <family> <tier> <start> <pass:true|false> <observation>
iso_emit_json() {
  printf '{"family":"%s","tier":"%s","started_at":"%s","finished_at":"%s","pass":%s,"observations":["%s"]}\n' \
    "$1" "$2" "$3" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$4" "$5"
}
