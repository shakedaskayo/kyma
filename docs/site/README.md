# kyma docs site

The VitePress site at https://docs.kyma.<your-domain>.

## Local development

From the repo root:

```bash
pnpm install
pnpm -C docs/site dev      # http://localhost:5173
pnpm -C docs/site build    # static output → docs/site/.vitepress/dist
pnpm -C docs/site preview  # serve the built dist locally
```

The build runs three checks before VitePress starts:

- `check:diagrams` — every `<Diagram name="..." />` reference resolves to a real SVG, and every `<image href="/icons/...">` inside an SVG points to a real icon.
- `predev` / `prebuild` — runs `scripts/sync-included.mjs`, which mirrors the kyma engine's `docs/architecture.md` and `docs/benchmarks.md` into `docs/site/architecture/` (under proper page frontmatter). Those two files are gitignored from the site directory because the source-of-truth lives one directory up.
- `mermaid` plugin — renders fenced `mermaid` blocks at build time.

## Editing

- Markdown lives under section folders: `quickstart/`, `concepts/`, `ingest/`, `query/`, `connectors/`, `reference/`, `architecture/`, `recipes/`.
- The kyma-native architecture pages (`architecture/architecture.md`, `architecture/benchmarks.md`) are **synced** from the kyma engine's `docs/` directory at build time. Edit `docs/architecture.md` or `docs/benchmarks.md` (one level up); the build copies them in. Don't edit the synced files directly — they're gitignored and regenerated on every build.
- SVG diagrams: drop into `public/diagrams/` and reference via `<Diagram name="..." caption="..." />`. The `check:diagrams` script enforces references at build time.
- Mermaid: fenced ` ```mermaid ` code blocks render automatically.
- Custom inline-SVG diagrams (e.g., `<KymaArchitectureDiagram />`) live in `.vitepress/theme/components/diagrams/` and are registered globally in `theme/index.ts`.

## Production deployment

`docs/site/Dockerfile` + `docs/site/railway.toml` describe a self-contained Railway service. The build context is the **repo root** because the Dockerfile pulls `docs/architecture.md` and `docs/benchmarks.md` from one directory above the site.

### One-time Railway setup (operator action required)

1. Create a **separate Railway project** named `kyma-docs` (not part of the engine's project).
2. Add a service → connect to the kyma GitHub repo on branch `main`.
3. Service settings:
   - **Root directory:** `/` (repo root — needed because the Dockerfile copies from one level above `docs/site/`).
   - **Watch paths / config-as-code:** point at `docs/site/railway.toml`. Railway picks up `[build]`, `[deploy]`, and `watchPatterns` from there.
4. Add a custom domain on the service: `docs.kyma.<your-domain>`. Railway provisions HTTPS automatically via Let's Encrypt.
5. First deploy: trigger from the dashboard or push a commit; Railway builds from the Dockerfile and serves the static output.

### What the build does (Dockerfile)

- **Builder stage** (`node:22-alpine`):
  1. Activates pnpm 9.15.4 via `corepack`.
  2. Copies the workspace root manifests (`package.json`, `pnpm-workspace.yaml`, `pnpm-lock.yaml`, `.npmrc`).
  3. Copies workspace member manifests so pnpm can resolve the graph; non-Node members (`web/src-tauri`) get a placeholder directory.
  4. Runs `pnpm install --frozen-lockfile --filter kyma-docs...` — installs only the docs-site dependency tree.
  5. Copies the architecture/benchmarks markdown, the images directory, and the docs site source.
  6. Runs `pnpm --filter kyma-docs run build`. The pre-build sync runs first, then VitePress emits static HTML to `.vitepress/dist`.
- **Runner stage** (`node:22-alpine`):
  1. Installs `serve@14`.
  2. Copies only the static `dist/` output (no source, no node_modules) under a non-root user.
  3. Listens on `$PORT` (Railway-injected; defaults to 3000 for local `docker run`).
  4. Health-checks `GET /` every 30 s.

### Local Docker smoke

```bash
# From the repo root.
docker build -f docs/site/Dockerfile -t kyma-docs .
docker run --rm -p 3000:3000 kyma-docs
# Visit http://localhost:3000
```

The image is small — a few MB beyond the Node alpine base. Static-site simple.

### Updating in production

The deployment is git-driven. Push to `main`; Railway picks up the change via `watchPatterns` and rebuilds. The watched paths cover everything that affects the rendered site (page content, theme, build scripts, deps). Code changes in `crates/` don't trigger a docs rebuild — and shouldn't.

## Sections

- **Quickstart** — `docker compose up` to first query, plus a slightly deeper walkthrough.
- **Concepts** — the mental model (10 pages).
- **Ingest** — REST/NDJSON, OTLP, Kafka, file-drop, plus the cross-cutting idempotency and coercion rules.
- **Query** — KQL, SQL, PromQL (roadmap), Arrow Flight, the agent endpoint, and pruning rules.
- **Connectors** — the framework, Prometheus reference, plus design pages for Postgres / MySQL / MongoDB (DB-M1/M2/M3).
- **Reference** — HTTP API, CLI, env vars, KQL functions, schema mappings.
- **Architecture** — the synced architecture overview and benchmarks, plus the slice roadmap and storage format.
- **Recipes** — worked examples that run as written.

