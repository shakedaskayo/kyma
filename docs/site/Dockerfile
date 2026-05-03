# syntax=docker/dockerfile:1.7
#
# kyma-docs Dockerfile
#
# Build context is the repo root. The image:
#   1. Installs the kyma-docs pnpm workspace member with its (workspace) lockfile.
#   2. Runs the VitePress build (which prebuilds: syncs architecture.md/benchmarks.md
#      from the kyma engine docs into the site, then builds static HTML).
#   3. Serves the resulting static site on the port Railway injects via $PORT.
#
# The runner stage is intentionally tiny: a Node alpine plus `serve` over the dist dir.
# No Node app server is involved at runtime; this is a static site.

# ─────────────────────────────  Builder  ─────────────────────────────
FROM node:22-alpine AS builder

# pnpm via corepack — pin to the version we develop against.
RUN corepack enable && corepack prepare pnpm@9.15.4 --activate

WORKDIR /app

# Workspace skeleton — files needed for `pnpm install` to resolve the workspace.
# Copying these first lets Docker cache the install layer when only docs change.
COPY package.json pnpm-workspace.yaml pnpm-lock.yaml .npmrc ./

# Workspace member manifests so pnpm can resolve the workspace graph.
# `web/src-tauri/` is a Rust crate without a package.json — pnpm tolerates
# the missing manifest for non-node members; we mkdir it so the path exists.
COPY web/package.json ./web/package.json
COPY docs/site/package.json ./docs/site/package.json
RUN mkdir -p ./web/src-tauri

# Install only the docs-site dependency tree.
# `--frozen-lockfile` is strict mode — fails the build if pnpm-lock.yaml is
# out of sync. `--filter kyma-docs...` resolves the docs-site member and any
# workspace deps it pulls in (currently none, but future-proof).
RUN pnpm install --frozen-lockfile --filter kyma-docs...

# Now the actual content. These are the files that change between builds.
COPY docs/architecture.md ./docs/architecture.md
COPY docs/benchmarks.md   ./docs/benchmarks.md
COPY docs/images          ./docs/images
COPY docs/site            ./docs/site

# Build. The site's prebuild hook runs sync-included.mjs to mirror
# docs/architecture.md and docs/benchmarks.md into docs/site/architecture/.
RUN pnpm --filter kyma-docs run build

# ─────────────────────────────  Runner  ──────────────────────────────
# Tiny static server. `serve@14` is fine for low-traffic docs; if the site
# ever needs serious scale, switch to nginx-alpine without changing the
# build stage.
FROM node:22-alpine AS runner

LABEL org.opencontainers.image.title="kyma-docs" \
      org.opencontainers.image.description="Static VitePress site for the kyma engine documentation." \
      org.opencontainers.image.source="https://github.com/shaked/engine"

RUN npm install -g serve@14 && \
    addgroup -S app && adduser -S app -G app

# Copy the built static site only — no node_modules, no source.
COPY --from=builder --chown=app:app /app/docs/site/.vitepress/dist /site

USER app
WORKDIR /site

# Railway sets $PORT at runtime; default to 3000 for local `docker run`.
ENV PORT=3000
EXPOSE 3000

# `--no-clipboard` suppresses the spammy serve banner; `-s` enables SPA fallback,
# which we don't need for VitePress (every route has a real .html), but it's
# defensive against future router changes.
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -q -O - "http://localhost:${PORT}/" >/dev/null 2>&1 || exit 1

CMD ["sh", "-c", "exec serve --no-clipboard -l tcp://0.0.0.0:${PORT} /site"]
