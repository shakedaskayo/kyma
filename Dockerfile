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
    protobuf-compiler pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
COPY --from=web /src/web/dist ./web/dist
RUN cargo build -p kyma-bin --release --features web-ui

# ---------- stage 3: runtime ----------
FROM gcr.io/distroless/cc-debian12
COPY --from=server /src/target/release/kyma-bin /usr/local/bin/kyma
EXPOSE 8080 9090
USER nonroot
ENTRYPOINT ["/usr/local/bin/kyma"]
