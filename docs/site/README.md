# kyma docs site

The VitePress site at https://shakedaskayo.github.io/kyma/.

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

The site deploys to **GitHub Pages** at https://shakedaskayo.github.io/kyma/ via
`.github/workflows/docs.yml`, using the official Pages Actions flow
(`upload-pages-artifact` → `deploy-pages`). There is no `gh-pages` branch and no
build output in git.

**Prerequisite (one-time):** the repo must have Pages enabled with source
"GitHub Actions" (Settings → Pages → Build and deployment → Source, or
`gh api -X POST repos/<owner>/<repo>/pages -f build_type=workflow`). The
workflow handles everything after that.

- **Triggers:** push to `main` touching `docs/site/**`, `docs/architecture.md`,
  `docs/benchmarks.md`, `docs/images/**`, the workflow file, or the workspace
  manifests (`package.json`, `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `.npmrc`).
  PRs touching the same paths get a build-only check (no deploy).
  Manual deploy: Actions tab → `docs` → Run workflow.
- **Build job:** `pnpm install --frozen-lockfile`, then `pnpm -C docs/site build`
  (the prebuild sync and diagram check run via npm hooks), then uploads
  `docs/site/.vitepress/dist` as the Pages artifact.
- **Deploy job:** `actions/deploy-pages` into the `github-pages` environment —
  deploy history and the live URL are on the repo's Environments page.
- **Base path:** the site is served under `/kyma/`, set via `base` in
  `.vitepress/config.ts`. Markdown links/images and `themeConfig` assets are
  auto-prefixed by VitePress; raw `<a href>` / `<img src>` in theme components
  must go through `withBase()` (see `Landing.vue`), and `head` entries need the
  literal `/kyma/` prefix.

## Sections

- **Quickstart** — `docker compose up` to first query, plus a slightly deeper walkthrough.
- **Concepts** — the mental model (10 pages).
- **Ingest** — REST/NDJSON, OTLP, Kafka, file-drop, plus the cross-cutting idempotency and coercion rules.
- **Query** — KQL, SQL, PromQL (roadmap), Arrow Flight, the agent endpoint, and pruning rules.
- **Connectors** — the framework, Prometheus reference, plus design pages for Postgres / MySQL / MongoDB (DB-M1/M2/M3).
- **Reference** — HTTP API, CLI, env vars, KQL functions, schema mappings.
- **Architecture** — the synced architecture overview and benchmarks, plus the slice roadmap and storage format.
- **Recipes** — worked examples that run as written.

