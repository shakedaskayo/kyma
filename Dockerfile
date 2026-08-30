# syntax=docker/dockerfile:1.7

# ---------- stage 1: web ----------
# The web app imports the @pensieve-ai/* workspace packages, so the build installs
# the whole pnpm workspace, builds the packages (web consumes their dist/), then
# builds web. Member manifests are copied first for a cache-friendly install
# layer; the root lockfile drives a deterministic --frozen-lockfile install.
FROM node:24-alpine AS web
RUN corepack enable && corepack prepare pnpm@9 --activate
WORKDIR /src
COPY pnpm-workspace.yaml package.json pnpm-lock.yaml .npmrc ./
COPY packages/client/package.json packages/client/
COPY packages/react/package.json packages/react/
COPY web/package.json web/
COPY docs/site/package.json docs/site/
COPY examples/embed-demo/package.json examples/embed-demo/
RUN pnpm install --frozen-lockfile
COPY packages/ packages/
COPY web/ web/
RUN pnpm --filter @pensieve-ai/client build \
 && pnpm --filter @pensieve-ai/react build \
 && pnpm -C web build

# ---------- stage 2: server ----------
FROM rust:1.84-bookworm AS server
RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler pkg-config libssl-dev cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
COPY --from=web /src/web/dist ./web/dist
# Override LTO and codegen-units via env so the linker doesn't OOM inside
# Docker Desktop (which caps memory at ~8 GB on macOS).
ENV CARGO_PROFILE_RELEASE_LTO=off \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4
# pensieve-bin emits `pensieve` (the server); pensieve-cli emits `pensieve-cli` (the management
# CLI). Distinct binary names → no copy-aside dance. Built separately because
# only pensieve-bin carries the web-ui/github features.
RUN cargo build -p pensieve-cli --release \
 && cargo build -p pensieve-bin --release --features web-ui,github

# ---------- stage 3: runtime ----------
# Use debian:12-slim instead of distroless because pensieve dynamically links
# libssl and libz which distroless/cc-debian12 does not include.
FROM debian:12-slim
# postgresql-client provides psql, used by pensieve-bootstrap.sh to CREATE DATABASE pensieve.
# Only needed by the bootstrap one-shot (not the main pensieve server), but we keep
# a single image for simplicity.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 zlib1g postgresql-client && rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 1000 pensieve && \
    useradd --system --uid 1000 --gid pensieve --create-home --home-dir /home/pensieve pensieve
COPY --from=server /src/target/release/pensieve /usr/local/bin/pensieve
COPY --from=server /src/target/release/pensieve-cli /usr/local/bin/pensieve-cli

# Bootstrap script: creates the pensieve catalog database and all context tables.
# Invoked as a one-shot Railway service during customer instance provisioning.
# Uses PENSIEVE_CATALOG_URL env var (set by provisioner via Railway variable reference).
# Written as a RUN heredoc (requires BuildKit, enabled via syntax=docker/dockerfile:1.7
# declared at the top of this file).
RUN <<'BOOTSTRAP'
cat > /usr/local/bin/pensieve-bootstrap.sh << 'SCRIPT'
#!/bin/sh
set -e
# PENSIEVE_CATALOG_URL points to the pensieve database (e.g. postgres://user:pw@host/pensieve).
# Derive the root-database URL by stripping the /pensieve suffix so we can run
# CREATE DATABASE pensieve against the postgres root.
ROOT_URL=$(echo "$PENSIEVE_CATALOG_URL" | sed 's|/pensieve$||')
echo "[bootstrap] Creating pensieve Postgres database (idempotent)..."
psql "$ROOT_URL" -c "CREATE DATABASE pensieve" 2>&1 | grep -v "already exists" || true
echo "[bootstrap] Pensieve database ready. Creating context tables via pensieve-cli..."
CLI=/usr/local/bin/pensieve-cli
$CLI create-database pensieve 2>&1 | grep -v 'duplicate key' | grep -v 'already exists' || true
$CLI create-table --db pensieve --name context_nodes \
  --schema "id:string,label:string,realm:string,source_type:string,source_id:string,run_id:string,created_at:timestamp,updated_at:timestamp,properties:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db pensieve --name context_edges \
  --schema "id:string,src:string,dst:string,type:string,realm:string,source_type:string,source_id:string,run_id:string,created_at:timestamp,properties:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db pensieve --name context_pipeline_runs \
  --schema "id:string,pipeline_id:string,source_type:string,status:string,started_at:timestamp,finished_at:timestamp,error:string,rows_in:long,rows_out:long" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db pensieve --name context_events \
  --schema "ts:timestamp,kind:string,actor:string,subject:string,attributes:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
echo "[bootstrap] Pensieve context tables ready."
$CLI list-tables --db pensieve
SCRIPT
chmod +x /usr/local/bin/pensieve-bootstrap.sh
BOOTSTRAP

EXPOSE 8080 9090
USER pensieve
WORKDIR /home/pensieve
ENTRYPOINT ["/usr/local/bin/pensieve"]
