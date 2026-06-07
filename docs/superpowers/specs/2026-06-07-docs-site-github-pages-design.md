# Docs site → GitHub Pages migration

**Date:** 2026-06-07
**Status:** Approved
**Scope:** Move the public docs site (`docs/site/`, VitePress) from Railway to GitHub Pages, end to end: build config, CI workflow, repo settings, Railway decommission, and reference updates.

## Decisions (user-confirmed)

| Decision | Choice |
|---|---|
| Serving URL | `https://shakedaskayo.github.io/kyma/` (default project Pages URL, no DNS work) |
| Old hosting | Decommission Railway entirely; `getkyma.dev` domain handled separately, out of scope |
| Deploy trigger | Push to `main`, path-filtered + `workflow_dispatch`; PRs get a build-only check |
| Mechanism | Official Actions flow (`configure-pages` → `upload-pages-artifact` → `deploy-pages`), Pages source = "GitHub Actions", no `gh-pages` branch |

## Current state

- VitePress 1.6.4 site at `docs/site/`, part of the pnpm workspace (`pnpm-workspace.yaml` lists `docs/site`).
- Build: `pnpm -C docs/site build` — `prebuild` runs `scripts/sync-included.mjs` (mirrors `docs/architecture.md` + `docs/benchmarks.md` into the site), `build` runs `scripts/check-diagrams.mjs` then `vitepress build`. Output: `docs/site/.vitepress/dist/`.
- Hosted on Railway (`docs/site/Dockerfile` + `docs/site/railway.toml`) at `https://getkyma.dev`, root-domain, no `base` path.
- No existing docs CI workflow; no `gh-pages` branch; repo is public at `github.com/shakedaskayo/kyma`.

## 1. VitePress config changes (`docs/site/`)

The site assumes root-domain serving; the `/kyma/` base path is the real migration work.

In `.vitepress/config.ts`:

- Add `base: '/kyma/'`. Markdown links/images and `themeConfig.logo` are auto-prefixed by VitePress — no content edits.
- Manually fix what VitePress does **not** auto-prefix:
  - `head` favicon `href` → `/kyma/icons/kyma-mark.svg`
  - `head` canonical, `og:url`, `og:image`, `twitter:image` → `https://shakedaskayo.github.io/kyma/...`
  - `sitemap.hostname` → `https://shakedaskayo.github.io/kyma/`
  - Footer `message` raw-HTML `<img src="/icons/brand/agentcy.svg">` → base-prefixed

In `.vitepress/theme/components/Landing.vue`:

- 17 raw `href="/..."` / `src="/..."` references → wrap with `withBase()` from `vitepress` (bound `:href`/`:src`, via small computed helpers where repetitive).

`Diagram.vue` needs no change — diagrams are build-time glob-bundled (`import.meta.glob` over `/public/diagrams/*.svg`), base-safe.

## 2. CI workflow: `.github/workflows/docs.yml`

Follows `web.yml` conventions: `actions/checkout@v4`, `pnpm/action-setup@v4` (version 9), `actions/setup-node@v4` (node 24, `cache: pnpm`), `pnpm install --frozen-lockfile`.

**Triggers**

- `push` to `main`, paths: `docs/site/**`, `docs/architecture.md`, `docs/benchmarks.md`, `docs/images/**`, `.github/workflows/docs.yml`, `pnpm-workspace.yaml` (mirrors the `railway.toml` watchPatterns).
- `pull_request` on the same paths → build job only, no deploy.
- `workflow_dispatch` for manual deploys.

**Jobs**

- `build`: checkout → pnpm setup → install → `pnpm -C docs/site build` (prebuild sync + diagram check run via npm hooks) → `actions/configure-pages` → `actions/upload-pages-artifact` with `docs/site/.vitepress/dist`.
- `deploy`: `needs: build`, skipped on `pull_request`; `actions/deploy-pages` into the `github-pages` environment. Permissions `pages: write`, `id-token: write`; `concurrency: { group: pages, cancel-in-progress: false }` so deploys don't race.

**Repo setting (one-time)**: flip Pages source to "GitHub Actions" — `gh api -X POST repos/shakedaskayo/kyma/pages -f build_type=workflow` (or `PUT` if Pages was ever enabled), or via the settings UI.

## 3. Decommission Railway + reference updates

- Delete `docs/site/Dockerfile` and `docs/site/railway.toml`.
- Rewrite the deployment section of `docs/site/README.md` (operator notes) to describe the Pages flow: trigger paths, manual dispatch, where to see deploys.
- Tear down the Railway service (dashboard or `railway` CLI) — outside the repo; exact steps documented in the plan. **Sequenced last, after the Pages deploy is verified live**, so there is never an interval with no docs site.
- Update `getkyma.dev` docs references to `https://shakedaskayo.github.io/kyma/`:
  - `README.md` (docs links/badges)
  - `crates/kyma-cli/Cargo.toml` (`homepage`)
  - `web/index.html`

## 4. Verification & rollout

- Local: `pnpm -C docs/site build && pnpm -C docs/site preview`, click through under the `/kyma/` base — landing CTAs, sidebar nav, inline diagrams, mermaid renders, local search, favicon, footer brand mark.
- PR build check validates the workflow's build job before merge. The deploy job only runs once the workflow lands on `main` (Pages deploys and `workflow_dispatch` require the workflow on the default branch).
- Post-merge: confirm the live URL, `/kyma/sitemap.xml`, OG tags, and that deep routes resolve (`cleanUrls` is fine on Pages because VitePress emits real `.html` files per route; also verify direct navigation to a clean URL like `/kyma/quickstart/five-minute-start`).
- Rollback: until Railway is torn down, `getkyma.dev` keeps serving the old site; if Pages misbehaves, fix forward or re-dispatch a prior commit. Railway teardown happens only after live verification.

## Error handling

- `check-diagrams.mjs` and VitePress dead-link checking run inside the build job — broken diagrams or links fail the PR check, not production.
- `.nojekyll` is unnecessary: the Actions deployment path serves the artifact as-is (no Jekyll pass).
- Concurrency group prevents overlapping deploys from rapid merges; `cancel-in-progress: false` lets each finish to keep deploy history linear.

## Out of scope

- The `getkyma.dev` domain's future (parking, marketing site, redirects).
- Any docs content changes beyond base-path mechanics.
