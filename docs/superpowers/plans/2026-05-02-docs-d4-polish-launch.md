# Docs D4 — Polish & Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Prerequisite:** [Docs D3 Doctests](2026-05-02-docs-d3-doctests.md) is complete and committed.

**Goal:** Take the docs site from "useful" to "shippable launch." Replace the placeholder palette with real kyma brand colors. Replace the placeholder `Landing.vue` with a real hero (animated logo / orbit / featured recipes — same structural pattern as agentcy's `Landing.vue`). Write at least 3 worked recipes. Generate `llms.txt` at build time so the site is first-class for LLM agents. Wire OG metadata + sitemap. Light Lighthouse pass (target ≥ 90 on perf and accessibility). Custom domain live and announced.

**Architecture:** D4 is largely content + cosmetic + one new build script (`gen-llms-txt.mjs`). It assumes the underlying machinery from D0–D3 is in place: site shell, content, generated reference, doctests. The recipes section ships using the same Vue component patterns introduced in D0; recipe metadata lives in a single `recipes.ts` data file (mirroring the agentcy pattern).

**Tech Stack:** VitePress, Vue 3.5, plain CSS. New build script: `gen-llms-txt.mjs`. Spec: [`docs/superpowers/specs/2026-05-02-kyma-docs-site-design.md`](../specs/2026-05-02-kyma-docs-site-design.md).

---

## File Structure

**New files:**

- `docs/site/.vitepress/data/recipes.ts` — typed recipe catalog (single source of truth for the Recipes section).
- `docs/site/.vitepress/theme/components/RecipeCard.vue` — used in recipe listings.
- `docs/site/.vitepress/theme/components/RecipeCatalog.vue` — full grid of recipes (used in `/recipes/index.md`).
- `docs/site/recipes/debug-prod-incident-cross-services.md` — recipe 1.
- `docs/site/recipes/agent-watches-ci-failures.md` — recipe 2.
- `docs/site/recipes/join-postgres-customers-with-kyma-logs.md` — recipe 3 (gated on DB M1).
- `docs/site/scripts/gen-llms-txt.mjs` — generates `public/llms.txt` from the VitePress site map.
- `docs/site/public/og-image.png` — Open Graph preview image (1200×630).

**Modified files:**

- `docs/site/.vitepress/theme/custom.css` — replace placeholder palette with kyma brand values.
- `docs/site/.vitepress/theme/components/Landing.vue` — replace placeholder with the real landing.
- `docs/site/.vitepress/config.ts` — OG metadata, sitemap, lastUpdated, edit-link off, social links polish, recipes sidebar from `recipes.ts`.
- `docs/site/package.json` — add `gen:llms-txt` script + wire into build.
- `docs/site/recipes/index.md` — replace D0 placeholder with `<RecipeCatalog />`.

---

## Task 1: Brand palette

**Files:**
- Modify: `docs/site/.vitepress/theme/custom.css`

- [ ] **Step 1: Decide the palette** at D4 review. Options:
  - kyma logo / favicon already exists; sample dominant colors.
  - Pick a contrast pair for light/dark.

- [ ] **Step 2: Replace placeholder values** with the chosen palette. Example shape (the actual values are decided at D4 review):

```css
:root {
  --vp-c-brand-1: #...;
  --vp-c-brand-2: #...;
  --vp-c-brand-3: #...;
  --vp-c-brand-soft: rgba(..., 0.14);
  --vp-home-hero-name-background: linear-gradient(135deg, ... 0%, ... 50%, ... 100%);
}
.dark { /* ... */ }
```

- [ ] **Step 3: Verify**

```bash
cd docs/site && pnpm dev
```

Walk through every section in light + dark mode; confirm no contrast issues. Run a Lighthouse audit on the landing page (target ≥ 90 on accessibility).

- [ ] **Step 4: Commit**

```bash
git add docs/site/.vitepress/theme/custom.css
git commit -m "feat(docs): kyma brand palette"
```

---

## Task 2: Landing page

**Files:**
- Modify: `docs/site/.vitepress/theme/components/Landing.vue`

- [ ] **Step 1: Read the agentcy landing for reference**

```bash
cat /Users/shaked/projects_new/agentcy/docs/.vitepress/theme/components/Landing.vue
```

- [ ] **Step 2: Replace the placeholder** with a real landing. Keep the structure simple to avoid Vue complexity creep — the goal is "looks production-quality," not "perfect art":

```vue
<script setup lang="ts">
import { recipes } from '../../data/recipes'
const featured = recipes.filter(r => r.featured).slice(0, 3)
</script>

<template>
  <section class="hero">
    <h1 class="hero-title">kyma</h1>
    <p class="hero-tagline">Production knowledge, as a query.</p>
    <p class="hero-subtagline">
      One columnar query engine for every signal your stack emits — logs, traces, metrics,
      and your databases. Your agents ask it anything in KQL or SQL; it answers at sub-second
      latency over a decade of history.
    </p>
    <div class="hero-cta">
      <a href="/quickstart/five-minute-start" class="cta-primary">Five-minute start →</a>
      <a href="https://github.com/shaked/engine" class="cta-secondary">GitHub</a>
    </div>
  </section>

  <section class="value-props">
    <div class="feature-grid">
      <a href="/concepts/what-is-kyma" class="feature-card">
        <h3>One engine, every signal</h3>
        <p>OTLP, REST, Kafka, file-drop. Plus first-class Postgres / MySQL / MongoDB.</p>
      </a>
      <a href="/concepts/the-pruning-cascade" class="feature-card">
        <h3>Sub-second over years of history</h3>
        <p>Three-level pruning skips 99% of data on 99% of queries.</p>
      </a>
      <a href="/concepts/multi-source-data" class="feature-card">
        <h3>Cross-source joins</h3>
        <p>Federation + sync, two modes per source. Live freshness or local speed — your call.</p>
      </a>
    </div>
  </section>

  <section class="featured-recipes">
    <h2>Featured recipes</h2>
    <div class="feature-grid">
      <a v-for="r in featured" :key="r.slug" :href="`/recipes/${r.slug}`" class="feature-card">
        <h3>{{ r.title }}</h3>
        <p>{{ r.summary }}</p>
        <span class="recipe-tag">{{ r.difficulty }}</span>
      </a>
    </div>
  </section>
</template>

<style scoped>
.hero { padding: 4rem 1rem; text-align: center; }
.hero-title {
  font-size: 5rem;
  background: var(--vp-home-hero-name-background);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  margin: 0;
  letter-spacing: -0.02em;
}
.hero-tagline { font-size: 1.75rem; opacity: 0.9; margin: 0.5rem 0 1rem; }
.hero-subtagline { font-size: 1.1rem; opacity: 0.75; max-width: 680px; margin: 0 auto 2rem; line-height: 1.6; }
.hero-cta { display: flex; gap: 1rem; justify-content: center; }
.cta-primary, .cta-secondary {
  display: inline-block;
  padding: 0.75rem 1.5rem;
  border-radius: 6px;
  font-weight: 600;
}
.cta-primary { background: var(--vp-c-brand-1); color: white; }
.cta-secondary { border: 1px solid var(--vp-c-divider); }
.value-props, .featured-recipes { padding: 2rem 1rem; max-width: 1200px; margin: 0 auto; }
.featured-recipes h2 { text-align: center; margin-bottom: 1rem; }
.recipe-tag {
  display: inline-block;
  margin-top: 0.5rem;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--vp-c-brand-soft);
  font-size: 0.85rem;
}
</style>
```

- [ ] **Step 3: Verify in browser**

- [ ] **Step 4: Commit**

```bash
git add docs/site/.vitepress/theme/components/Landing.vue
git commit -m "feat(docs): real Landing component"
```

---

## Task 3: Recipes data + components

**Files:**
- Create: `docs/site/.vitepress/data/recipes.ts`
- Create: `docs/site/.vitepress/theme/components/RecipeCard.vue`
- Create: `docs/site/.vitepress/theme/components/RecipeCatalog.vue`
- Modify: `docs/site/.vitepress/theme/index.ts` (register components)

- [ ] **Step 1: Recipes data**

Write `docs/site/.vitepress/data/recipes.ts`:

```ts
export interface Recipe {
  slug: string
  title: string
  summary: string
  difficulty: 'starter' | 'intermediate' | 'advanced'
  status: 'shipped' | 'coming-soon'
  featured?: boolean
  tags: string[]
}

export const recipes: Recipe[] = [
  {
    slug: 'debug-prod-incident-cross-services',
    title: 'Debug a prod incident across services',
    summary: 'Trace a single error across three services in three KQL queries.',
    difficulty: 'starter',
    status: 'shipped',
    featured: true,
    tags: ['debugging', 'kql', 'logs'],
  },
  {
    slug: 'agent-watches-ci-failures',
    title: 'Agent that watches CI failures',
    summary: 'Send GitHub Actions events to kyma; an agent triages new failures every hour.',
    difficulty: 'intermediate',
    status: 'shipped',
    featured: true,
    tags: ['agent', 'automation', 'ci'],
  },
  {
    slug: 'join-postgres-customers-with-kyma-logs',
    title: 'Join Postgres customers with kyma logs',
    summary: 'Federation in action: cross-source join between live Postgres and synced kyma logs.',
    difficulty: 'intermediate',
    status: 'shipped',
    featured: true,
    tags: ['federation', 'postgres', 'sql'],
  },
]
```

- [ ] **Step 2: `RecipeCard.vue`** renders one recipe.

- [ ] **Step 3: `RecipeCatalog.vue`** renders the grid.

- [ ] **Step 4: Register in `theme/index.ts`.**

- [ ] **Step 5: Update `docs/site/recipes/index.md`**

```markdown
---
title: Recipes
description: Worked examples — copy, paste, adapt.
---

# Recipes

Each recipe follows one template: problem → schema → query → expected output → variations.
All runnable code in these recipes is tested in CI; if a query goes stale here, the build
fails before the page deploys.

<RecipeCatalog />
```

- [ ] **Step 6: Commit**

---

## Task 4: Recipe pages

**Files:**
- Create: 3 markdown files under `docs/site/recipes/`

- [ ] **Step 1: Recipe 1 — `debug-prod-incident-cross-services.md`**

```markdown
---
title: Debug a prod incident across services
description: Trace a single error across three services in three KQL queries.
recipe:
  slug: debug-prod-incident-cross-services
---

# Debug a prod incident across services

## The problem

A user filed a bug saying "checkout broke at 14:32." You don't know which service
to blame. With kyma you don't have to guess — you ask three KQL questions.

## The schema

This recipe uses the auto-created `otel_logs` table populated by your services'
OTLP exporters. Required columns: `_timestamp`, `severity_text`, `service_name`,
`body`, `trace_id`.

## The queries

### 1. Find the error

```kql-runnable
otel_logs
| where _timestamp between datetime(2026-01-01T14:30:00Z) .. datetime(2026-01-01T14:35:00Z)
| where severity_text == "ERROR"
| summarize n=count() by service_name
| order by n desc
```

The service with the most errors in that window is your suspect.

### 2. Pull the error bodies

```kql-runnable
otel_logs
| where _timestamp between datetime(2026-01-01T14:30:00Z) .. datetime(2026-01-01T14:35:00Z)
| where service_name == "checkout-svc" and severity_text == "ERROR"
| project _timestamp, trace_id, body
| take 20
```

Skim a few; pick a representative `trace_id`.

### 3. Follow the trace across services

```kql-runnable
otel_logs
| where trace_id == "abc123"
| project _timestamp, service_name, severity_text, body
| order by _timestamp asc
```

You see the request enter `web-svc`, hop to `checkout-svc`, and crash in the
`payments-svc` call. Total time: under a minute.

## What you should see

[snapshot table — produced by D3 doctests]

## Variations

- **Error rate over time:** add `bin(_timestamp, 1m)` to the first query's `summarize`.
- **Compare to baseline:** `let baseline = ...; let now = ...; baseline | join now on service_name | project ...`.
- **Different signal:** swap `otel_logs` for `traces` if you have spans ingested.
```

- [ ] **Step 2: Recipe 2 — `agent-watches-ci-failures.md`**

(Walks through: emitting GitHub Actions events into kyma via REST; a curl loop; an agent prompt that pulls the day's failures and groups them.)

- [ ] **Step 3: Recipe 3 — `join-postgres-customers-with-kyma-logs.md`** (depends on DB M1)

(Walks through: registering a Postgres connector with `mode=federation`, then a SQL query: `SELECT u.email, COUNT(*) FROM pg_prod.public.users u JOIN otel_logs l ON l.user_id = u.id WHERE l.severity_text = 'ERROR' GROUP BY u.email ORDER BY 2 DESC LIMIT 5`. Includes a `pushdown_summary` snippet.)

- [ ] **Step 4: Run doctests**

```bash
docs/site/doctest/seed.sh
cargo run -p kyma-doctest -- --update-snapshots docs/site/recipes/
cargo run -p kyma-doctest -- docs/site/recipes/
```

Expected: all blocks pass.

- [ ] **Step 5: Commit**

```bash
git add docs/site/recipes/ docs/site/.vitepress/data/recipes.ts docs/site/.vitepress/theme/components/Recipe*.vue docs/site/.vitepress/generated/doctest-snapshots/
git commit -m "feat(docs): three recipe pages with runnable doctested examples"
```

---

## Task 5: `llms.txt` generation

**Files:**
- Create: `docs/site/scripts/gen-llms-txt.mjs`
- Modify: `docs/site/package.json`

- [ ] **Step 1: Generator**

Write `docs/site/scripts/gen-llms-txt.mjs`:

```js
#!/usr/bin/env node
//! Generates public/llms.txt from the VitePress sidebar config.
//! Format: https://llmstxt.org/

import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'

const SITE_URL = process.env.KYMA_DOCS_URL || 'https://docs.kyma.<domain>'

// Read the config; we re-parse the sidebar struct from it. Simplest path:
// import the config via dynamic import and walk its sidebar.
const cfg = (await import('../.vitepress/config.ts')).default
const sidebar = cfg.themeConfig?.sidebar ?? {}

let out = `# kyma docs\n\n> Production knowledge, as a query.\n\n`

for (const [base, groups] of Object.entries(sidebar)) {
  for (const grp of (groups || [])) {
    out += `## ${grp.text}\n`
    for (const item of (grp.items || [])) {
      // Read the page frontmatter for the description.
      const mdPath = path.join('.', `${item.link.replace(/\/$/, '')}.md`)
      let description = ''
      try {
        const body = readFileSync(mdPath, 'utf8')
        const fm = body.match(/^---\n([\s\S]*?)\n---/)
        if (fm) {
          const m = fm[1].match(/^description:\s*(.+)$/m)
          if (m) description = m[1].replace(/^["']|["']$/g, '')
        }
      } catch {}
      out += `- [${item.text}](${SITE_URL}${item.link})${description ? ': ' + description : ''}\n`
    }
    out += '\n'
  }
}

writeFileSync('public/llms.txt', out, 'utf8')
console.log(`generated public/llms.txt (${out.length} bytes)`)
```

(Run with `node --experimental-vm-modules` if needed; alternatively transpile via `tsx` or rewrite in `.mjs` ESM only.)

- [ ] **Step 2: Wire into build**

`docs/site/package.json`:

```json
"scripts": {
  "gen:llms-txt": "node scripts/gen-llms-txt.mjs",
  "build": "npm run check:diagrams && npm run check:generated && npm run check:doctests && npm run gen:llms-txt && vitepress build"
}
```

- [ ] **Step 3: Verify**

```bash
cd docs/site && pnpm gen:llms-txt
cat public/llms.txt | head -30
```

- [ ] **Step 4: Commit**

```bash
git add docs/site/scripts/gen-llms-txt.mjs docs/site/package.json
git commit -m "feat(docs): generate llms.txt from sidebar at build time"
```

---

## Task 6: OG metadata + sitemap + canonical URLs

**Files:**
- Modify: `docs/site/.vitepress/config.ts`
- Create: `docs/site/public/og-image.png` (designer-provided; placeholder if needed)

- [ ] **Step 1: OG + canonical**

Add to `config.ts` `head`:

```ts
['meta', { property: 'og:type',        content: 'website' }],
['meta', { property: 'og:title',       content: 'kyma docs' }],
['meta', { property: 'og:description', content: 'Production knowledge, as a query.' }],
['meta', { property: 'og:image',       content: 'https://docs.kyma.<domain>/og-image.png' }],
['meta', { property: 'og:url',         content: 'https://docs.kyma.<domain>/' }],
['meta', { name: 'twitter:card',       content: 'summary_large_image' }],
['link', { rel: 'canonical',           href: 'https://docs.kyma.<domain>/' }],
```

- [ ] **Step 2: Sitemap**

VitePress emits `sitemap.xml` automatically when `themeConfig.sitemap.hostname` is set:

```ts
themeConfig: {
  sitemap: { hostname: 'https://docs.kyma.<domain>' },
  // ...
}
```

- [ ] **Step 3: OG image**

If a designer provides `og-image.png`, drop it at `docs/site/public/og-image.png`. Otherwise a placeholder image (1200×630, kyma logo on brand background) is fine for D4.

- [ ] **Step 4: Verify** the build emits `dist/sitemap.xml` and `og-image.png` is at `dist/og-image.png`.

- [ ] **Step 5: Commit**

---

## Task 7: Lighthouse pass

- [ ] **Step 1: Run Lighthouse on the deployed staging URL or a local `pnpm preview` build.**

Target metrics (per spec §11.4):
- Performance: ≥ 90
- Accessibility: ≥ 90
- Best Practices: ≥ 90
- SEO: ≥ 95

- [ ] **Step 2: Fix issues found.** Common ones:
  - Image alt text missing (Diagram component should propagate caption as alt).
  - Color contrast on the placeholder palette (caught at Task 1; re-verify).
  - Cumulative layout shift on the landing page (constrain image dimensions).
  - Lazy-loading images (VitePress mostly handles this; verify SVGs).

- [ ] **Step 3: Commit fixes.**

---

## Task 8: Custom domain + announce

- [ ] **Step 1: Verify Railway** has the custom domain `docs.kyma.<domain>` configured (set up at D0). HTTPS cert auto-provisioned.

- [ ] **Step 2: Update OG / sitemap / canonical URLs to the actual domain** (replace `<domain>` placeholder).

- [ ] **Step 3: Add a redirect from any prior URL** (if applicable).

- [ ] **Step 4: Verify the live site:**
  - `https://docs.kyma.<domain>/` loads, shows the new landing.
  - `/llms.txt` returns the generated index.
  - `/sitemap.xml` lists all pages.
  - Random sample of 10 pages all load without 404.
  - Recipe doctests' rendered queries match the live kyma's behavior (use the docker stack to spot-check).

- [ ] **Step 5: Update kyma `README.md`** to reference the new docs URL prominently. (One-line edit.)

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "docs: link kyma docs site in README"
```

---

## Task 9: D4 acceptance smoke

- [ ] Brand palette applied; light + dark mode look polished.
- [ ] Real `Landing.vue` shipped; landing page is shareable.
- [ ] At least 3 recipe pages, all with doctested runnable queries.
- [ ] `llms.txt` generated and served at `/llms.txt`.
- [ ] OG metadata + sitemap + canonical URLs use the live domain.
- [ ] Lighthouse ≥ 90 across perf / accessibility / best practices / SEO.
- [ ] Custom domain live; README updated.
- [ ] `pnpm build` clean end-to-end (diagrams + generated + doctests + llms.txt + vitepress build).
- [ ] Tag `docs-d4-launch`.

---

## Cross-cutting commitments verified at D4

These were promised in the docs spec §11.4:

- [ ] Every PR that adds or changes a public surface regenerates the corresponding JSON or updates a doctest snapshot. CI gates both.
- [ ] Diagram references validated at build time.
- [ ] Custom domain stable from D0; all canonical URLs point at it.
- [ ] Site builds reproducibly: pnpm-lockfile + cargo-lockfile pinned.

---

## D4 Open Decisions

- **Final brand palette values** (Task 1).
- **Whether to add an "ask the docs" widget** (post-v1; out of scope for D4 per spec §2 non-goals).
- **Whether to add an external search service** (post-v1; VitePress local search is sufficient for the page count at launch).
- **README ↔ landing relationship** — README is short summary; site is canonical long-form. Verify at D4 review by walking both and making sure they don't contradict.
