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
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 zlib1g && rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 1000 kyma && \
    useradd --system --uid 1000 --gid kyma --create-home --home-dir /home/kyma kyma
COPY --from=server /src/target/release/kyma /usr/local/bin/kyma
COPY --from=server /src/target/release/kyma-cli /usr/local/bin/kyma-cli
EXPOSE 8080 9090
USER kyma
WORKDIR /home/kyma
ENTRYPOINT ["/usr/local/bin/kyma"]
