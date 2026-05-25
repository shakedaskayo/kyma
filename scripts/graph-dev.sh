#!/usr/bin/env bash
# graph-dev.sh — bring up the local stack and seed data so the web
# Context Graph (/graph) renders against real schema-graph data.
#
# It starts Postgres + MinIO (docker compose), builds + runs the engine
# (kyma-bin) natively, seeds the `obs` database with cross-referenced
# tables (so the schema-graph has REFERENCES edges), and verifies the
# /v1/graph endpoint. Then run the web dev server separately:
#
#     cd web && pnpm dev
#
# and in the app's Settings set:
#     endpoint  = http://127.0.0.1:8080
#     token     = $KYMA_TOKEN   (default: kyma_dev_token)
#     database  = obs
# then open the "Graph" tab.
#
# Env overrides (useful when host port 5433 is taken by another project):
#     KYMA_PG_PORT   host port for the catalog Postgres (default 5433)
#     KYMA_TOKEN     auth token the server accepts        (default kyma_dev_token)
#
# Usage: ./scripts/graph-dev.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PG_PORT="${KYMA_PG_PORT:-5433}"
TOKEN="${KYMA_TOKEN:-kyma_dev_token}"
SERVER="http://127.0.0.1:8080"

echo "==> Starting Postgres + MinIO (docker compose)"
docker compose up -d postgres minio minio-init

echo "==> Building engine (kyma-bin) + CLI (debug)"
cargo build -p kyma-bin -p kyma-cli

echo "==> Launching kyma-bin → $SERVER"
KYMA_CATALOG_URL="postgres://kyma:kyma_dev@127.0.0.1:${PG_PORT}/kyma" \
KYMA_S3_ENDPOINT="http://127.0.0.1:9000" \
KYMA_S3_BUCKET="kyma" \
KYMA_S3_REGION="us-east-1" \
KYMA_S3_ACCESS_KEY_ID="kyma_admin" \
KYMA_S3_SECRET_ACCESS_KEY="kyma_admin_dev" \
KYMA_S3_PATH_STYLE="true" \
KYMA_S3_ALLOW_HTTP="true" \
KYMA_HTTP_ADDR="127.0.0.1:8080" \
KYMA_GRPC_ADDR="127.0.0.1:9090" \
KYMA_OTLP_ADDR="off" \
KYMA_PATH_PREFIX="kyma" \
KYMA_AUTH_TOKENS="$TOKEN" \
  ./target/debug/kyma &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

echo "==> Waiting for /health"
for i in $(seq 1 60); do
  curl -fsS "$SERVER/health" >/dev/null 2>&1 && { echo "  healthy"; break; }
  sleep 1
done

echo "==> Seeding demo + rich data into the 'obs' database"
./scripts/seed-demo-data.sh "$SERVER" || true
./scripts/seed-rich-data.sh "$SERVER" || true

echo "==> Verifying the schema graph (/v1/graph/schema/overview, db=obs)"
curl -fsS "$SERVER/v1/graph/schema/overview?limit=500" \
  -H "Authorization: Bearer $TOKEN" -H "X-Database: obs" \
  | head -c 800
echo
echo
echo "==> Engine is up (pid $SERVER_PID). Now run the web UI:"
echo "      cd web && pnpm dev"
echo "    then in Settings set endpoint=$SERVER token=$TOKEN database=obs and open Graph."
echo "    (Ctrl-C here stops the engine.)"
wait $SERVER_PID
