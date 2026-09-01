# Docs Site → GitHub Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the pensieve docs site from Railway (`getpensieve.dev`) to GitHub Pages at `https://shakedaskayo.github.io/pensieve/`, end to end — build config, CI deploy workflow, reference updates, and Railway decommission.

**Architecture:** VitePress gains `base: '/pensieve/'`; absolute URLs that VitePress does not auto-prefix (head tags, raw HTML/Vue template refs) are fixed manually or via `withBase()`. A new `.github/workflows/docs.yml` builds the site and deploys via the official Pages Actions flow (`upload-pages-artifact` → `deploy-pages`, no `gh-pages` branch). Railway files are deleted; teardown is sequenced **last**, after the Pages deploy is verified live, so there is never an interval with no docs site.

**Tech Stack:** VitePress 1.6.4 (pnpm workspace member `docs/site`), GitHub Actions (`pnpm/action-setup@v4` v9, `setup-node@v4` node 24), GitHub Pages, `gh` CLI.

**Spec:** `docs/superpowers/specs/2026-06-07-docs-site-github-pages-design.md`

**Testing model:** this is static-site/config work with no unit-test framework — the test surface is the production build plus assertions on its output. Every code task follows the same loop: make the change → run `pnpm -C docs/site build` → grep `docs/site/.vitepress/dist/` for the exact expected (and absence of broken) output → commit. Run the greps exactly as written; they are the pass/fail criteria.

---

### Task 1: Branch setup

**Files:** none (git only)

The spec and this plan were committed on `feat/secret-handling` (which carries unrelated in-progress work). Implementation happens on a clean branch off `main`.

- [ ] **Step 1: Create an isolated worktree/branch off main**

Use the `superpowers:using-git-worktrees` skill (branch name `feat/docs-github-pages`, base `origin/main`). Fallback commands if doing it manually:

```bash
git fetch origin
git worktree add .claude/worktrees/docs-github-pages -b feat/docs-github-pages origin/main
cd .claude/worktrees/docs-github-pages
```

- [ ] **Step 2: Cherry-pick the spec + plan commits from feat/secret-handling**

```bash
git log feat/secret-handling --oneline -5 | grep -i "github pages"
# Expect two commits: the spec ("docs: GitHub Pages migration design spec for docs site",
# 8685bbf9) and the plan ("docs: GitHub Pages migration implementation plan").
git cherry-pick 8685bbf9 <plan-commit-sha>   # oldest first
```

Expected: both apply cleanly (doc-only files, no conflicts).

- [ ] **Step 3: Install dependencies in the worktree**

```bash
pnpm install --frozen-lockfile
```

Expected: completes without lockfile errors.

- [ ] **Step 4: Baseline build sanity check**

```bash
pnpm -C docs/site build
```

Expected: `build complete` from VitePress, exit 0. This is the pre-migration baseline.

---

### Task 2: VitePress base path + absolute URLs in config

**Files:**
- Modify: `docs/site/.vitepress/config.ts`

VitePress auto-prefixes markdown links/images and `themeConfig.logo` once `base` is set — those need no edits. The `head` entries, `sitemap.hostname`, and the raw-HTML footer `<img>` are NOT auto-prefixed and are fixed here. (Verified against the installed VitePress 1.6.4: sitemap item URLs are page paths without base, so `hostname` must carry `/pensieve/`.)

- [ ] **Step 1: Add `base` to the config**

In `docs/site/.vitepress/config.ts`, change:

```ts
  title: 'pensieve',
  description: 'Production knowledge, as a query.',
  cleanUrls: true,
```

to:

```ts
  title: 'pensieve',
  description: 'Production knowledge, as a query.',
  cleanUrls: true,

  // Served from GitHub Pages at shakedaskayo.github.io/pensieve/.
  base: '/pensieve/',
```

- [ ] **Step 2: Update the sitemap hostname**

Change:

```ts
  sitemap: {
    hostname: 'https://getpensieve.dev',
  },
```

to:

```ts
  sitemap: {
    // Must include the base — VitePress sitemap URLs are page paths without it.
    hostname: 'https://shakedaskayo.github.io/pensieve/',
  },
```

- [ ] **Step 3: Update the head block (favicon + canonical + OG/Twitter)**

Change:

```ts
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/icons/pensieve-mark.svg' }],
    ['link', { rel: 'canonical', href: 'https://getpensieve.dev/' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { property: 'og:description', content: 'A unified columnar query engine for every signal your stack emits. Logs, traces, metrics, plus first-class federation to Postgres, MySQL, MongoDB. Sub-second latency over a decade of history.' }],
    ['meta', { property: 'og:url', content: 'https://getpensieve.dev/' }],
    ['meta', { property: 'og:image', content: 'https://getpensieve.dev/icons/pensieve-mark.svg' }],
    ['meta', { name: 'twitter:card', content: 'summary' }],
    ['meta', { name: 'twitter:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { name: 'twitter:description', content: 'One columnar query engine for every signal your stack emits. By AgentcyLabs.' }],
    ['meta', { name: 'twitter:image', content: 'https://getpensieve.dev/icons/pensieve-mark.svg' }],
  ],
```

to (only `href`/`content` URLs change — `head` entries are not base-prefixed by VitePress):

```ts
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/pensieve/icons/pensieve-mark.svg' }],
    ['link', { rel: 'canonical', href: 'https://shakedaskayo.github.io/pensieve/' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { property: 'og:description', content: 'A unified columnar query engine for every signal your stack emits. Logs, traces, metrics, plus first-class federation to Postgres, MySQL, MongoDB. Sub-second latency over a decade of history.' }],
    ['meta', { property: 'og:url', content: 'https://shakedaskayo.github.io/pensieve/' }],
    ['meta', { property: 'og:image', content: 'https://shakedaskayo.github.io/pensieve/icons/pensieve-mark.svg' }],
    ['meta', { name: 'twitter:card', content: 'summary' }],
    ['meta', { name: 'twitter:title', content: 'pensieve — production knowledge, as a query' }],
    ['meta', { name: 'twitter:description', content: 'One columnar query engine for every signal your stack emits. By AgentcyLabs.' }],
    ['meta', { name: 'twitter:image', content: 'https://shakedaskayo.github.io/pensieve/icons/pensieve-mark.svg' }],
  ],
```

- [ ] **Step 4: Base-prefix the footer's raw-HTML img**

In `themeConfig.footer`, change:

```ts
      message: 'An open project by <a href="https://agentcylabs.com" target="_blank" rel="noopener" class="pensieve-footer-link"><img src="/icons/brand/agentcy.svg" alt="" width="12" height="12" class="pensieve-footer-mark"/> AgentcyLabs</a> · MIT licensed.',
```

to:

```ts
      message: 'An open project by <a href="https://agentcylabs.com" target="_blank" rel="noopener" class="pensieve-footer-link"><img src="/pensieve/icons/brand/agentcy.svg" alt="" width="12" height="12" class="pensieve-footer-mark"/> AgentcyLabs</a> · MIT licensed.',
```

(`themeConfig.logo` stays `/icons/pensieve-mark.svg` — the default theme runs it through `withBase`.)

- [ ] **Step 5: Build and assert base-prefixed output**

```bash
pnpm -C docs/site build
grep -c 'href="/pensieve/' docs/site/.vitepress/dist/index.html
grep -o 'rel="icon"[^>]*' docs/site/.vitepress/dist/index.html
grep -o '<loc>[^<]*</loc>' docs/site/.vitepress/dist/sitemap.xml | head -3
```

Expected:
- build exits 0
- `grep -c` ≥ 1 (assets now under `/pensieve/`)
- icon line contains `href="/pensieve/icons/pensieve-mark.svg"`
- every `<loc>` starts `https://shakedaskayo.github.io/pensieve/` with exactly one `/pensieve/` segment

- [ ] **Step 6: Commit**

```bash
git add docs/site/.vitepress/config.ts
git commit -m "feat(docs): serve docs site under /pensieve/ base for GitHub Pages"
```

---

### Task 3: `withBase()` for raw template URLs in Landing.vue

**Files:**
- Modify: `docs/site/.vitepress/theme/components/Landing.vue`

Raw `<a href="/...">` / `<img src="/...">` in Vue templates are not base-prefixed by VitePress. There are 17 static refs + 2 dynamic bindings. `Diagram.vue` needs nothing (SVGs are build-time glob-bundled).

- [ ] **Step 1: Import withBase**

Change line 2:

```ts
import { ref, onMounted, onUnmounted, computed } from 'vue'
```

to:

```ts
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { withBase } from 'vitepress'
```

- [ ] **Step 2: Convert all 19 refs to withBase bindings**

Apply each replacement exactly (line numbers from current file; external `https://` links stay untouched):

| Line | Old | New |
|---|---|---|
| 210 | `<img src="/icons/brand/agentcy.svg" alt="" width="14" height="14" aria-hidden="true" />` | `<img :src="withBase('/icons/brand/agentcy.svg')" alt="" width="14" height="14" aria-hidden="true" />` |
| 224 | `<a href="/quickstart/five-minute-start" class="pensieve-cta pensieve-cta--primary">Quickstart</a>` | `<a :href="withBase('/quickstart/five-minute-start')" class="pensieve-cta pensieve-cta--primary">Quickstart</a>` |
| 225 | `<a href="/agent/connect" class="pensieve-cta">Connect your agent</a>` | `<a :href="withBase('/agent/connect')" class="pensieve-cta">Connect your agent</a>` |
| 226 | `<a href="/agent/memory" class="pensieve-cta">Memory</a>` | `<a :href="withBase('/agent/memory')" class="pensieve-cta">Memory</a>` |
| 295 | `<img src="/icons/pensieve-mark.svg" alt="pensieve" />` | `<img :src="withBase('/icons/pensieve-mark.svg')" alt="pensieve" />` |
| 313 | `:href="node.role === 'source' ? '/ingest/' : '/query/'"` | `:href="withBase(node.role === 'source' ? '/ingest/' : '/query/')"` |
| 315 | `<img :src="`/icons/brand/${node.slug}.svg`" :alt="node.label" width="22" height="22" />` | `<img :src="withBase(`/icons/brand/${node.slug}.svg`)" :alt="node.label" width="22" height="22" />` |
| 493 | `<a href="/query/kql" class="pensieve-cta pensieve-cta--primary">KQL reference</a>` | `<a :href="withBase('/query/kql')" class="pensieve-cta pensieve-cta--primary">KQL reference</a>` |
| 494 | `<a href="/query/sql" class="pensieve-cta">SQL reference</a>` | `<a :href="withBase('/query/sql')" class="pensieve-cta">SQL reference</a>` |
| 533 | `<a href="/concepts/the-pruning-cascade" class="pensieve-cta">Read the deep dive →</a>` | `<a :href="withBase('/concepts/the-pruning-cascade')" class="pensieve-cta">Read the deep dive →</a>` |
| 542 | `<a class="pensieve-section-card" href="/quickstart/">` | `<a class="pensieve-section-card" :href="withBase('/quickstart/')">` |
| 546 | `<a class="pensieve-section-card" href="/concepts/">` | `<a class="pensieve-section-card" :href="withBase('/concepts/')">` |
| 550 | `<a class="pensieve-section-card" href="/ingest/">` | `<a class="pensieve-section-card" :href="withBase('/ingest/')">` |
| 554 | `<a class="pensieve-section-card" href="/query/">` | `<a class="pensieve-section-card" :href="withBase('/query/')">` |
| 558 | `<a class="pensieve-section-card" href="/connectors/">` | `<a class="pensieve-section-card" :href="withBase('/connectors/')">` |
| 562 | `<a class="pensieve-section-card" href="/architecture/architecture">` | `<a class="pensieve-section-card" :href="withBase('/architecture/architecture')">` |
| 566 | `<a class="pensieve-section-card" href="/recipes/">` | `<a class="pensieve-section-card" :href="withBase('/recipes/')">` |
| 570 | `<a class="pensieve-section-card" href="/reference/">` | `<a class="pensieve-section-card" :href="withBase('/reference/')">` |
| 580 | `<img src="/icons/brand/agentcy.svg" alt="" width="16" height="16" aria-hidden="true" />` | `<img :src="withBase('/icons/brand/agentcy.svg')" alt="" width="16" height="16" aria-hidden="true" />` |

- [ ] **Step 3: Build and assert no unprefixed internal URLs remain**

```bash
pnpm -C docs/site build
grep -c 'href="/\(quickstart\|concepts\|ingest\|query\|connectors\|agent\|deploy\|architecture\|recipes\|reference\)' docs/site/.vitepress/dist/index.html || echo "NONE"
grep -c 'src="/icons' docs/site/.vitepress/dist/index.html || echo "NONE"
grep -c 'href="/pensieve/quickstart/' docs/site/.vitepress/dist/index.html
```

Expected:
- first two greps print `NONE` (exit 1, zero matches — SSR output has every internal URL prefixed)
- last grep ≥ 1 (the hero CTA renders with the base)

- [ ] **Step 4: Visual spot-check under the base path**

```bash
pnpm -C docs/site preview
```

Open `http://localhost:4173/pensieve/` — landing renders, orbit brand icons load (no broken images), hero CTAs and section cards navigate under `/pensieve/`, footer brand mark loads. Ctrl-C when done.

- [ ] **Step 5: Commit**

```bash
git add docs/site/.vitepress/theme/components/Landing.vue
git commit -m "feat(docs): base-prefix raw template URLs in Landing via withBase"
```

---

### Task 4: GitHub Actions workflow

**Files:**
- Create: `.github/workflows/docs.yml`

Conventions copied from `.github/workflows/web.yml`: checkout@v4, pnpm/action-setup@v4 (v9), setup-node@v4 (node 24, pnpm cache), frozen lockfile. Path filters mirror `docs/site/railway.toml` `watchPatterns`.

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/docs.yml` with exactly:

```yaml
name: docs
on:
  push:
    branches: [main]
    paths:
      - "docs/site/**"
      - "docs/architecture.md"
      - "docs/benchmarks.md"
      - "docs/images/**"
      - ".github/workflows/docs.yml"
      - "package.json"
      - "pnpm-lock.yaml"
      - "pnpm-workspace.yaml"
      - ".npmrc"
  pull_request:
    paths:
      - "docs/site/**"
      - "docs/architecture.md"
      - "docs/benchmarks.md"
      - "docs/images/**"
      - ".github/workflows/docs.yml"
      - "package.json"
      - "pnpm-lock.yaml"
      - "pnpm-workspace.yaml"
      - ".npmrc"
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

# One Pages deploy at a time; let in-flight deploys finish.
concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
        with: { version: 9 }
      - uses: actions/setup-node@v4
        with: { node-version: 24, cache: "pnpm" }
      - run: pnpm install --frozen-lockfile
      # prebuild sync (architecture/benchmarks) + diagram check run via npm hooks.
      - run: pnpm -C docs/site build
      - uses: actions/configure-pages@v5
        if: github.event_name != 'pull_request'
      - uses: actions/upload-pages-artifact@v3
        if: github.event_name != 'pull_request'
        with:
          path: docs/site/.vitepress/dist

  deploy:
    # PRs get the build check only — deploy runs on main push / manual dispatch.
    if: github.event_name != 'pull_request'
    needs: [build]
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Validate the YAML parses**

```bash
pnpm dlx js-yaml .github/workflows/docs.yml > /dev/null && echo "YAML OK"
```

Expected: `YAML OK`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/docs.yml
git commit -m "ci(docs): GitHub Pages deploy workflow (official Actions flow)"
```

---

### Task 5: Remove Railway config + rewrite operator notes

**Files:**
- Delete: `docs/site/Dockerfile`
- Delete: `docs/site/railway.toml`
- Modify: `docs/site/README.md`

- [ ] **Step 1: Delete the Railway files**

```bash
git rm docs/site/Dockerfile docs/site/railway.toml
```

- [ ] **Step 2: Update the README intro line**

In `docs/site/README.md`, change line 3:

```markdown
The VitePress site at https://docs.pensieve.<your-domain>.
```

to:

```markdown
The VitePress site at https://shakedaskayo.github.io/pensieve/.
```

- [ ] **Step 3: Replace the "Production deployment" section**

Replace everything from the `## Production deployment` heading through the end of the "Updating in production" paragraph (currently lines 30–72, ending with "…don't trigger a docs rebuild — and shouldn't.") with:

```markdown
## Production deployment

The site deploys to **GitHub Pages** at https://shakedaskayo.github.io/pensieve/ via
`.github/workflows/docs.yml`, using the official Pages Actions flow
(`upload-pages-artifact` → `deploy-pages`). There is no `gh-pages` branch and no
build output in git.

- **Triggers:** push to `main` touching `docs/site/**`, `docs/architecture.md`,
  `docs/benchmarks.md`, `docs/images/**`, the workflow file, or the workspace
  manifests. PRs touching the same paths get a build-only check (no deploy).
  Manual deploy: Actions tab → `docs` → Run workflow.
- **Build job:** `pnpm install --frozen-lockfile`, then `pnpm -C docs/site build`
  (the prebuild sync and diagram check run via npm hooks), then uploads
  `docs/site/.vitepress/dist` as the Pages artifact.
- **Deploy job:** `actions/deploy-pages` into the `github-pages` environment —
  deploy history and the live URL are on the repo's Environments page.
- **Base path:** the site is served under `/pensieve/`, set via `base` in
  `.vitepress/config.ts`. Markdown links/images and `themeConfig` assets are
  auto-prefixed by VitePress; raw `<a href>` / `<img src>` in theme components
  must go through `withBase()` (see `Landing.vue`), and `head` entries need the
  literal `/pensieve/` prefix.
```

Keep `## Sections` and everything after it unchanged.

- [ ] **Step 4: Confirm the build doesn't reference the deleted files**

```bash
pnpm -C docs/site build
grep -rn "railway\|Dockerfile" docs/site/package.json docs/site/scripts/ || echo "NONE"
```

Expected: build exits 0; grep prints `NONE`.

- [ ] **Step 5: Commit**

```bash
git add -A docs/site
git commit -m "chore(docs): decommission Railway config; document Pages deploy"
```

---

### Task 6: Update getpensieve.dev references repo-wide

**Files:**
- Modify: `README.md`
- Modify: `crates/pensieve-cli/Cargo.toml:9-10`
- Modify: `web/index.html:12`

- [ ] **Step 1: README.md — replace all 8 getpensieve.dev URLs**

| Line | Old | New |
|---|---|---|
| 2 | `<a href="https://www.getpensieve.dev">` | `<a href="https://shakedaskayo.github.io/pensieve/">` |
| 20 | `<a href="https://www.getpensieve.dev"><img alt="Docs" src="https://img.shields.io/badge/docs-getpensieve.dev-7ed957?style=flat-square" /></a>` | `<a href="https://shakedaskayo.github.io/pensieve/"><img alt="Docs" src="https://img.shields.io/badge/docs-github.io%2Fpensieve-7ed957?style=flat-square" /></a>` |
| 21 | `<a href="https://www.getpensieve.dev/agent/memory">` | `<a href="https://shakedaskayo.github.io/pensieve/agent/memory">` |
| 31 | `<a href="https://www.getpensieve.dev/agent/memory">Memory</a> ·` | `<a href="https://shakedaskayo.github.io/pensieve/agent/memory">Memory</a> ·` |
| 32 | `<a href="https://www.getpensieve.dev/connectors/">Connectors</a> ·` | `<a href="https://shakedaskayo.github.io/pensieve/connectors/">Connectors</a> ·` |
| 33 | `<a href="https://www.getpensieve.dev/architecture/architecture">Architecture</a>` | `<a href="https://shakedaskayo.github.io/pensieve/architecture/architecture">Architecture</a>` |
| 204 | `See **[Agentic Memory](https://www.getpensieve.dev/agent/memory)** for the full design.` | `See **[Agentic Memory](https://shakedaskayo.github.io/pensieve/agent/memory)** for the full design.` |
| 381 | `Full docs at **[getpensieve.dev](https://www.getpensieve.dev)**.` | `Full docs at **[shakedaskayo.github.io/pensieve](https://shakedaskayo.github.io/pensieve/)**.` |

- [ ] **Step 2: Cargo.toml — homepage + documentation**

In `crates/pensieve-cli/Cargo.toml`, change:

```toml
homepage = "https://www.getpensieve.dev"
documentation = "https://www.getpensieve.dev"
```

to:

```toml
homepage = "https://shakedaskayo.github.io/pensieve/"
documentation = "https://shakedaskayo.github.io/pensieve/"
```

- [ ] **Step 3: web/index.html — stale comment**

Change line 12's comment fragment `Matches getpensieve.dev; degrades to the system stack offline.` to `Matches the docs site; degrades to the system stack offline.`

- [ ] **Step 4: Verify no stragglers**

```bash
grep -rn "getpensieve.dev" README.md crates/pensieve-cli/Cargo.toml web/index.html docs/site/ --exclude-dir=node_modules --exclude-dir=.vitepress/dist || echo "NONE"
cargo metadata --no-deps --format-version 1 > /dev/null && echo "Cargo.toml OK"
```

Expected: `NONE`, then `Cargo.toml OK`.

- [ ] **Step 5: Commit**

```bash
git add README.md crates/pensieve-cli/Cargo.toml web/index.html
git commit -m "docs: point docs links at shakedaskayo.github.io/pensieve"
```

---

### Task 7: Enable Pages + land on main

**Files:** none (GitHub settings + git)

- [ ] **Step 1: Enable GitHub Pages with workflow build type (one-time)**

```bash
gh api -X POST repos/shakedaskayo/pensieve/pages -f build_type=workflow
```

Expected: HTTP 201 with a JSON body containing `"build_type": "workflow"`.
If it returns 409 (Pages already exists):

```bash
gh api -X PUT repos/shakedaskayo/pensieve/pages -f build_type=workflow
```

Fallback (UI): repo → Settings → Pages → Source: **GitHub Actions**.

- [ ] **Step 2: Merge to main locally and push** (user preference: local `--no-ff` merge, not a PR)

```bash
git checkout main && git pull origin main
git merge --no-ff feat/docs-github-pages -m "feat(docs): migrate docs site to GitHub Pages"
git push origin main
```

Expected: merge commit lands; the push triggers the `docs` workflow (path filters match).

---

### Task 8: Post-deploy verification

**Files:** none

- [ ] **Step 1: Watch the workflow run**

```bash
gh run list --workflow=docs.yml --limit 1
gh run watch $(gh run list --workflow=docs.yml --limit 1 --json databaseId -q '.[0].databaseId')
```

Expected: `build` and `deploy` jobs both succeed; deploy reports the page URL.

- [ ] **Step 2: Assert the live site**

```bash
curl -sI https://shakedaskayo.github.io/pensieve/ | head -1
curl -sI https://shakedaskayo.github.io/pensieve/quickstart/five-minute-start | head -1
curl -sI https://shakedaskayo.github.io/pensieve/icons/pensieve-mark.svg | head -1
curl -s  https://shakedaskayo.github.io/pensieve/sitemap.xml | grep -o '<loc>[^<]*</loc>' | head -3
curl -sI https://shakedaskayo.github.io/pensieve/definitely-not-a-page | head -1
```

Expected, in order: `200`, `200` (clean URLs resolve — Pages serves `foo.html` for `/foo`), `200` (public assets under base), three `<loc>` entries each with exactly one `/pensieve/`, `404`.

- [ ] **Step 3: Browser click-through**

Open `https://shakedaskayo.github.io/pensieve/` — verify landing visuals (orbit icons, brand marks), nav + sidebar navigation, local search returns results, a mermaid diagram renders (e.g. under concepts), and a `<Diagram />` SVG page renders.

---

### Task 9: Decommission Railway (only after Task 8 passes)

**Files:** none (Railway dashboard/CLI)

Rollback window ends here: until this task, `getpensieve.dev` still serves the old site from Railway.

- [ ] **Step 1: Tear down the Railway service**

Dashboard: https://railway.app → project `pensieve-docs` → Settings → Danger → **Delete Project** (this also releases the `getpensieve.dev` custom-domain binding).

CLI alternative (if `railway` is installed and authed):

```bash
railway list                 # find the pensieve-docs project id
railway environment          # confirm context
railway down                 # or delete the project from the dashboard
```

- [ ] **Step 2: Confirm old hosting is gone**

```bash
curl -sI --max-time 10 https://getpensieve.dev/ | head -1
```

Expected: connection failure or a non-200 (Railway no longer serving). The domain's future (parking, redirect, marketing site) is out of scope per the spec.

---

## Self-review (done at write time)

- **Spec coverage:** base path + config URLs → Task 2; Landing.vue `withBase` → Task 3; workflow + repo setting → Tasks 4, 7; Railway file removal + operator notes → Task 5; reference updates → Task 6; verification → Task 8; teardown-last sequencing/rollback → Task 9. All spec sections covered.
- **Placeholder scan:** none — every step has exact code/commands and expected output.
- **Consistency:** the served URL `https://shakedaskayo.github.io/pensieve/` and base `/pensieve/` are identical across Tasks 2–8; workflow path filters match `railway.toml` watchPatterns plus the workflow file itself.
