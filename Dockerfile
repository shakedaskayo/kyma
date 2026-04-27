# syntax=docker/dockerfile:1.7

# ---------- stage 1: web ----------
FROM node:20-alpine AS web
RUN corepack enable && corepack prepare pnpm@9 --activate
WORKDIR /src
COPY pnpm-workspace.yaml package.json .npmrc ./
COPY web/package.json web/pnpm-lock.yaml* web/
RUN pnpm -C web install --frozen-lockfile || pnpm -C web install
COPY web/ web/
RUN pnpm -C web build

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
RUN cargo build -p kyma-bin -p kyma-cli --release --features web-ui

# ---------- stage 3: runtime ----------
# Use debian:12-slim instead of distroless because kyma dynamically links
# libssl and libz which distroless/cc-debian12 does not include.
FROM debian:12-slim
# postgresql-client provides psql, used by kyma-bootstrap.sh to CREATE DATABASE kyma.
# Only needed by the bootstrap one-shot (not the main kyma server), but we keep
# a single image for simplicity.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 zlib1g postgresql-client && rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 1000 kyma && \
    useradd --system --uid 1000 --gid kyma --create-home --home-dir /home/kyma kyma
COPY --from=server /src/target/release/kyma /usr/local/bin/kyma
COPY --from=server /src/target/release/kyma-cli /usr/local/bin/kyma-cli

# Bootstrap script: creates the kyma catalog database and all context tables.
# Invoked as a one-shot Railway service during customer instance provisioning.
# Uses KYMA_CATALOG_URL env var (set by provisioner via Railway variable reference).
# Written as a RUN heredoc (requires BuildKit, enabled via syntax=docker/dockerfile:1.7
# declared at the top of this file).
RUN <<'BOOTSTRAP'
cat > /usr/local/bin/kyma-bootstrap.sh << 'SCRIPT'
#!/bin/sh
set -e
# KYMA_CATALOG_URL points to the kyma database (e.g. postgres://user:pw@host/kyma).
# Derive the root-database URL by stripping the /kyma suffix so we can run
# CREATE DATABASE kyma against the postgres root.
ROOT_URL=$(echo "$KYMA_CATALOG_URL" | sed 's|/kyma$||')
echo "[bootstrap] Creating kyma Postgres database (idempotent)..."
psql "$ROOT_URL" -c "CREATE DATABASE kyma" 2>&1 | grep -v "already exists" || true
echo "[bootstrap] Kyma database ready. Creating context tables via kyma-cli..."
CLI=/usr/local/bin/kyma-cli
$CLI create-database kyma 2>&1 | grep -v 'duplicate key' | grep -v 'already exists' || true
$CLI create-table --db kyma --name context_nodes \
  --schema "id:string,label:string,realm:string,source_type:string,source_id:string,run_id:string,created_at:timestamp,updated_at:timestamp,properties:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db kyma --name context_edges \
  --schema "id:string,src:string,dst:string,type:string,realm:string,source_type:string,source_id:string,run_id:string,created_at:timestamp,properties:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db kyma --name context_pipeline_runs \
  --schema "id:string,pipeline_id:string,source_type:string,status:string,started_at:timestamp,finished_at:timestamp,error:string,rows_in:long,rows_out:long" \
  2>&1 | grep -v 'duplicate key' || true
$CLI create-table --db kyma --name context_events \
  --schema "ts:timestamp,kind:string,actor:string,subject:string,attributes:dynamic" \
  2>&1 | grep -v 'duplicate key' || true
echo "[bootstrap] Kyma context tables ready."
$CLI list-tables --db kyma
SCRIPT
chmod +x /usr/local/bin/kyma-bootstrap.sh
BOOTSTRAP

EXPOSE 8080 9090
USER kyma
WORKDIR /home/kyma
ENTRYPOINT ["/usr/local/bin/kyma"]
