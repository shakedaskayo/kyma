# Cloud Slice 2 — Control Plane v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the kyma cloud control plane v1 — a multi-tenant Hono+Drizzle API, Next.js customer dashboard, Next.js admin shell, and four CLI commands — that lets a brand-new user sign up via GitHub, create a workspace, mint an MCP token, and query their data through `mcp.kyma.dev/<workspace-id>` with a Claude skill, all in under 60 seconds.

**Architecture:** Three Node 22 services in a `cloud/` pnpm workspace (api, web, admin) plus a `cloud/packages/shared/` types library. The api owns Drizzle migrations and writes `api_tokens` rows that the existing `kyma-server` `DbAuthBackend` (Slice 0) reads via the same Postgres. Auth is hand-rolled: GitHub OAuth + magic-link, both yielding HttpOnly cookie sessions. Stripe is pinned to API version `2024-11-20.acacia`, with a webhook log-then-reconcile pattern and an hourly drop-detection sweeper. Provisioning in Slice 2 is "shared tenancy": workspace creation = inserting rows; the engine cluster already exists at `mcp.kyma.dev`. Slice 3 will add Railway provisioning and per-workspace KMS — Slice 2 stores connector configs cleartext in `connectors.config` (the `kms_key_id`/`encrypted_secrets` columns from Slice 0 stay null).

**Tech Stack:** Node 22, TypeScript 5.7, Hono 4, Drizzle 0.38 + drizzle-kit 0.30, `pg` 8, jose 6 (signed session cookies), Stripe 17 (pinned `2024-11-20.acacia`), Resend 4, Next.js 15 + React 19 + Tailwind 4.1 + Zustand 5, Vitest 2, Rust 1.95 (CLI retrofit), pnpm workspaces, Docker, Railway.

---

## Scope

**Includes (Slice 2):**
- Phase A — Monorepo workspace, Drizzle schema (mirroring agentcy cloud, minus password fields, minus approval gate, minus instances), local docker-compose Postgres on port 5434, Vitest harness.
- Phase B — Hand-rolled auth: GitHub OAuth start/callback, magic-link issue/exchange, signed session cookies, session middleware, logout.
- Phase C — Workspace CRUD, workspace_members, MCP-token minting that writes the `api_tokens` schema the engine's `DbAuthBackend` already expects (token_hash bytea SHA-256, scopes csv text, tenant_id = workspace.id).
- Phase D — Stripe checkout + portal + webhook (signature-verified, log-then-reconcile, 200-on-handler-error), hourly drop-detection sweeper.
- Phase E — Customer dashboard at `cloud.kyma.dev`: login, workspaces list/create/detail with the MCP install widget, billing page.
- Phase F — Admin dashboard skeleton at `admin.kyma.dev`: GitHub-OAuth gated by allowlist (`KYMA_ADMIN_GITHUB_IDS`), workspace list. "Promote to dedicated" button is not wired in Slice 2.
- Phase G — `kyma login`, `kyma workspace [list|select]`, `kyma ingest <file>`, `kyma table list` over HTTPS.
- Phase H — Engine flip: production deployment runs `kyma-bin` with feature `cloud-auth` and `KYMA_AUTH_BACKEND=db` so cloud-issued tokens authenticate.
- Phase I — Dockerfiles, Railway services in project `0773c99d-5f16-426a-9da1-6b5de1d0b88d`, DNS for `cloud.kyma.dev`, `admin.kyma.dev`, `mcp.kyma.dev`.
- Phase J — Slice 2 verification gate.

**Explicitly excludes (per spec):**
- Real Railway provisioning for dedicated tenants — Slice 3.
- "Promote to dedicated" graduation flow (snapshot, extent copy, atomic cutover, connector cursor migration) — Slice 3.
- Per-workspace KMS encryption for connector secrets — Slice 3 (Slice 0 schema columns stay null in Slice 2).
- Per-tenant query budgets / cancellation — Slice 2.5.
- Full CLI retrofit (`kyma connector`, `kyma mcp` stdio, `~/.config/kyma/profiles.toml`, brew tap, `cargo install`) — Slice 4.
- Marketing landing for `cloud.kyma.dev` — Slice 5.

---

## Decisions locked at slice kickoff

1. **Magic-link library**: hand-rolled (~50 lines). Insert `magic_links` row with SHA-256 hashed token + 15min TTL → email via Resend → exchange burns the row and mints a session.
2. **Connector secret encryption**: deferred to Slice 3. Slice 2 stores configs in cleartext `connectors.config` JSONB.
3. **Cloudflare account**: existing AgentcyLabs account (used for `mcp.kyma.dev` rate limiting in Slice 5; in Slice 2 the CNAME goes direct to Railway).
4. **Stripe API version**: pinned to `2024-11-20.acacia` in `stripe.service.ts` constructor.

---

## File Structure

**Create — Phase A (foundation):**
- `cloud/package.json` — workspace root with `dev`/`build`/`db:*` scripts.
- `cloud/pnpm-workspace.yaml` — globs for `api`, `web`, `admin`, `packages/*`.
- `cloud/docker-compose.yml` — Postgres 16 on host port 5434, db `kyma_cloud`, user `kyma`/`kyma_dev`.
- `cloud/api/package.json` — Hono 4, Drizzle 0.38, drizzle-kit 0.30, jose 6, pg 8, stripe 17, resend 4, zod 3, vitest 2.
- `cloud/api/tsconfig.json`, `cloud/api/drizzle.config.ts`, `cloud/api/vitest.config.ts`.
- `cloud/api/src/env.ts` — zod-parsed env loader.
- `cloud/api/src/db/client.ts` — pg.Pool + drizzle factory.
- `cloud/api/src/db/schema.ts` — every Drizzle table.
- `cloud/api/src/db/migrate.ts` — runs migrations from `./src/db/migrations`.
- `cloud/api/src/lib/errors.ts` — `AppError` + helpers (`badRequest`, `unauthorized`, `forbidden`, `notFound`, `conflict`).
- `cloud/api/src/lib/sessions.ts` — `signSessionCookie` / `verifySessionCookie` using jose HS256 + `SESSION_SECRET`.
- `cloud/api/src/lib/tokens.ts` — `generateMcpToken` (`kyma_` + 32-byte hex), `hashToken` (SHA-256 → Buffer), `randomBytesHex`.
- `cloud/api/src/index.ts` — Hono app composition; routes mounted in the order required by Stripe (webhook before any auth).
- `cloud/packages/shared/package.json`, `cloud/packages/shared/src/index.ts`, `cloud/packages/shared/src/plans.ts`, `cloud/packages/shared/src/types.ts`.

**Create — Phase B (auth):**
- `cloud/api/src/services/auth.service.ts` — GitHub OAuth (start, callback, upsert), magic-link (issue, exchange).
- `cloud/api/src/services/email.service.ts` — Resend wrapper, magic-link email template.
- `cloud/api/src/routes/auth.ts` — `/api/auth/github/start`, `/api/auth/github/callback`, `/api/auth/magic-link/{request,exchange}`, `/api/auth/me`, `/api/auth/logout`.
- `cloud/api/src/middleware/session.ts` — verifies `kyma_session` cookie, sets `c.var.user`.
- `cloud/api/src/middleware/workspace.ts` — resolves workspace from path slug, enforces membership.

**Create — Phase C (workspaces):**
- `cloud/api/src/services/workspace.service.ts` — `createWorkspace`, `listForUser`, `getBySlug`, `mintMcpToken`, `revokeMcpToken`.
- `cloud/api/src/routes/workspaces.ts` — `/api/workspaces` (GET, POST), `/api/workspaces/:slug` (GET), `/api/workspaces/:slug/tokens` (GET, POST, POST `/:id/revoke`).

**Create — Phase D (Stripe):**
- `cloud/api/src/services/stripe.service.ts` — pinned client, plan ↔ price helpers, `applyWorkspaceSubscription`, `downgradeWorkspaceToFree`, `findWorkspaceByCustomerId`.
- `cloud/api/src/services/billing-reconciler.ts` — hourly job that lists Stripe subscriptions for active customers and reconciles `workspaces.plan`.
- `cloud/api/src/routes/billing.ts` — `/api/billing/checkout`, `/api/billing/portal`, `/api/billing/subscription`.
- `cloud/api/src/routes/webhooks.ts` — `/api/webhooks/stripe` (raw-body + signature-verified, log to `billing_events`, dispatch).

**Create — Phase E (customer web):**
- `cloud/web/package.json` (Next 15, React 19, Tailwind 4.1, Zustand 5, lucide-react, recharts, motion, clsx, tailwind-merge).
- `cloud/web/next.config.ts`, `cloud/web/tsconfig.json`, `cloud/web/postcss.config.mjs`.
- `cloud/web/src/app/layout.tsx`, `cloud/web/src/app/globals.css` (kyma tokens — JetBrains Mono + IBM Plex Sans + phosphor green).
- `cloud/web/src/app/page.tsx` — redirects to `/workspaces` if logged in, `/login` otherwise.
- `cloud/web/src/app/login/page.tsx` — GitHub button + magic-link form + magic-link sent confirmation state.
- `cloud/web/src/app/login/callback/page.tsx` — handles `?token=...` magic-link exchange.
- `cloud/web/src/app/(dashboard)/layout.tsx`, `cloud/web/src/app/(dashboard)/workspaces/page.tsx`, `cloud/web/src/app/(dashboard)/workspaces/new/page.tsx`, `cloud/web/src/app/(dashboard)/workspaces/[slug]/page.tsx`, `cloud/web/src/app/(dashboard)/billing/page.tsx`.
- `cloud/web/src/components/mcp-install-widget.tsx` — copy-paste-able URL + token + Claude install snippet.
- `cloud/web/src/components/usage-chart.tsx` — Recharts area chart over `/api/usage`.
- `cloud/web/src/lib/api.ts` — typed fetch client.
- `cloud/web/src/lib/auth-server.ts` — server-side helper that reads the `kyma_session` cookie.
- `cloud/web/src/stores/session.ts` — Zustand store for the current user.

**Create — Phase F (admin web skeleton):**
- `cloud/admin/package.json`, `cloud/admin/next.config.ts`, `cloud/admin/tsconfig.json`, `cloud/admin/postcss.config.mjs`.
- `cloud/admin/src/app/layout.tsx`, `cloud/admin/src/app/globals.css`, `cloud/admin/src/app/page.tsx` (login).
- `cloud/admin/src/app/api/auth/github/start/route.ts`, `cloud/admin/src/app/api/auth/github/callback/route.ts` — gated by `KYMA_ADMIN_GITHUB_IDS`.
- `cloud/admin/src/app/(dash)/layout.tsx`, `cloud/admin/src/app/(dash)/workspaces/page.tsx`.
- `cloud/admin/src/lib/admin-session.ts` — `kyma_admin_session` cookie sign/verify.
- `cloud/admin/src/lib/db.ts` — direct `pg.Pool` reads of `users` / `workspaces` for read-only admin views.

**Create — Phase G (CLI retrofit):**
- `crates/kyma-cli/src/api_client.rs` — reqwest-based `cloud.kyma.dev/api` client.
- `crates/kyma-cli/src/profile.rs` — minimal `~/.config/kyma/credentials.toml` (single profile).
- `crates/kyma-cli/src/commands/login.rs`, `commands/workspace.rs`, `commands/ingest.rs`, `commands/table.rs`, `commands/mod.rs`.

**Create — Phase I (deploy):**
- `cloud/Dockerfile.api`, `cloud/Dockerfile.web`, `cloud/Dockerfile.admin`.
- `cloud/api/railway.toml`, `cloud/web/railway.toml`, `cloud/admin/railway.toml`.

**Modify:**
- `crates/kyma-cli/Cargo.toml` — add `reqwest`, `dirs`, `toml`, `keyring`.
- `crates/kyma-cli/src/main.rs` — replace single-binary catalog admin with subcommand router that dispatches to `commands/`.

---

## Tasks

### Task 1: Monorepo workspace skeleton

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/package.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/pnpm-workspace.yaml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/docker-compose.yml`

- [ ] **Step 1: Create the workspace root package.json**

```json
{
  "name": "kyma-cloud",
  "private": true,
  "scripts": {
    "dev": "pnpm --parallel -r run dev",
    "build": "pnpm -r run build",
    "test": "pnpm -r run test",
    "db:generate": "pnpm --filter @kyma/cloud-api run db:generate",
    "db:migrate": "pnpm --filter @kyma/cloud-api run db:migrate"
  }
}
```

- [ ] **Step 2: Create pnpm-workspace.yaml**

```yaml
packages:
  - 'api'
  - 'web'
  - 'admin'
  - 'packages/*'
```

- [ ] **Step 3: Create docker-compose.yml for local Postgres**

The cloud control plane uses port 5434 to avoid colliding with the engine's catalog Postgres on 5433.

```yaml
version: '3.9'

services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: kyma
      POSTGRES_PASSWORD: kyma_dev
      POSTGRES_DB: kyma_cloud
    ports:
      - '5434:5432'
    volumes:
      - kyma_cloud_pg:/var/lib/postgresql/data

volumes:
  kyma_cloud_pg:
```

- [ ] **Step 4: Verify pnpm workspace recognizes the layout**

Run: `cd /Users/shaked/projects_new/agentcy/kyma/cloud && pnpm install --frozen-lockfile=false`
Expected: install succeeds with "no projects" warning (no packages yet); workspace root only.

- [ ] **Step 5: Commit**

```bash
git add cloud/package.json cloud/pnpm-workspace.yaml cloud/docker-compose.yml
git commit -m "feat(cloud): scaffold monorepo workspace and local Postgres"
```

---

### Task 2: Shared types package (`@kyma/shared`)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/packages/shared/package.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/packages/shared/src/index.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/packages/shared/src/plans.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/packages/shared/src/types.ts`

- [ ] **Step 1: Create package.json**

```json
{
  "name": "@kyma/shared",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "src/index.ts",
  "types": "src/index.ts",
  "exports": {
    ".": "./src/index.ts",
    "./plans": "./src/plans.ts",
    "./types": "./src/types.ts"
  }
}
```

- [ ] **Step 2: Write `plans.ts` with Slice 2 plan tier definitions**

```ts
export type PlanId = 'free' | 'pro' | 'team';

export interface PlanLimits {
  maxWorkspaces: number;     // per-user
  ingestBytesPerMonth: number;
  mcpCallsPerDay: number;    // -1 = unlimited
  pricePerMonth: number;     // USD
  trialDays: number;
  features: string[];
}

export const PLANS: Record<PlanId, PlanLimits> = {
  free: {
    maxWorkspaces: 1,
    ingestBytesPerMonth: 1 * 1024 * 1024 * 1024,         // 1 GiB
    mcpCallsPerDay: 50,
    pricePerMonth: 0,
    trialDays: 0,
    features: ['1 workspace', '1 GiB / month ingest', '50 MCP calls / day', 'Community support'],
  },
  pro: {
    maxWorkspaces: 5,
    ingestBytesPerMonth: 50 * 1024 * 1024 * 1024,        // 50 GiB
    mcpCallsPerDay: 5000,
    pricePerMonth: 49,
    trialDays: 14,
    features: ['5 workspaces', '50 GiB / month ingest', '5,000 MCP calls / day', 'Email support'],
  },
  team: {
    maxWorkspaces: 20,
    ingestBytesPerMonth: 1024 * 1024 * 1024 * 1024,      // 1 TiB
    mcpCallsPerDay: -1,
    pricePerMonth: 299,
    trialDays: 14,
    features: ['20 workspaces', '1 TiB / month ingest', 'Unlimited MCP calls', 'Priority support', 'SSO (Slice 5)'],
  },
};

export function getPlanLimits(plan: PlanId): PlanLimits {
  return PLANS[plan];
}
```

- [ ] **Step 3: Write `types.ts`**

```ts
export interface CloudUser {
  id: string;
  githubId: string | null;
  email: string;
  name: string | null;
  avatarUrl: string | null;
  createdAt: string;
}

export interface Workspace {
  id: string;
  slug: string;
  name: string;
  ownerUserId: string;
  plan: 'free' | 'pro' | 'team';
  planActive: boolean;
  kind: 'shared' | 'dedicated';
  kymaEndpoint: string;       // engine HTTP base
  mcpEndpoint: string;        // <KYMA_ENGINE_BASE_URL>/<id>/mcp/v1
  trialEndsAt: string | null;
  subscriptionPeriodEnd: string | null;
  createdAt: string;
}

export interface WorkspaceMember {
  workspaceId: string;
  userId: string;
  role: 'owner' | 'admin' | 'member';
  joinedAt: string;
}

export interface ApiTokenSummary {
  id: string;
  name: string;
  prefix: string;             // e.g. "kyma_a1b2c3d4"
  scopes: string[];           // ['read'] | ['read','write'] | ['read','write','admin']
  createdAt: string;
  lastUsedAt: string | null;
  revokedAt: string | null;
}

export interface SessionUser {
  id: string;
  email: string;
  name: string | null;
}
```

- [ ] **Step 4: Write `index.ts` re-export**

```ts
export * from './plans.js';
export * from './types.js';
```

- [ ] **Step 5: Commit**

```bash
git add cloud/packages/shared
git commit -m "feat(cloud/shared): plan tiers and shared TypeScript types"
```

---

### Task 3: API package skeleton + env loader

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/package.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/tsconfig.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/vitest.config.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/drizzle.config.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/env.ts`

- [ ] **Step 1: package.json**

```json
{
  "name": "@kyma/cloud-api",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "build": "tsc",
    "start": "node dist/index.js",
    "test": "vitest run",
    "db:generate": "drizzle-kit generate",
    "db:migrate": "tsx src/db/migrate.ts",
    "db:push": "drizzle-kit push"
  },
  "dependencies": {
    "@kyma/shared": "workspace:*",
    "@hono/node-server": "^1.13.0",
    "drizzle-orm": "^0.38.0",
    "dotenv": "^17.0.0",
    "hono": "^4.7.0",
    "jose": "^6.0.0",
    "pg": "^8.13.0",
    "resend": "^4.0.0",
    "stripe": "^17.7.0",
    "uuid": "^11.0.0",
    "zod": "^3.24.0"
  },
  "devDependencies": {
    "@types/node": "^22.0.0",
    "@types/pg": "^8.11.0",
    "@types/uuid": "^10.0.0",
    "drizzle-kit": "^0.30.0",
    "tsx": "^4.19.0",
    "typescript": "^5.7.0",
    "vitest": "^2.1.0"
  }
}
```

- [ ] **Step 2: tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2022"],
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "resolveJsonModule": true,
    "declaration": false
  },
  "include": ["src/**/*"]
}
```

- [ ] **Step 3: drizzle.config.ts**

```ts
import 'dotenv/config';
import { defineConfig } from 'drizzle-kit';

export default defineConfig({
  schema: './src/db/schema.ts',
  out: './src/db/migrations',
  dialect: 'postgresql',
  dbCredentials: {
    url:
      process.env.DRIZZLE_DATABASE_URL ||
      'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud',
  },
});
```

- [ ] **Step 4: vitest.config.ts**

```ts
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    setupFiles: ['./src/test-setup.ts'],
    testTimeout: 20000,
  },
});
```

- [ ] **Step 5: env.ts (zod-parsed loader)**

```ts
import 'dotenv/config';
import { z } from 'zod';

const envSchema = z.object({
  PORT: z.coerce.number().default(3003),
  NODE_ENV: z.enum(['development', 'production', 'test']).default('development'),

  // Cloud control-plane DB. Separate logical DB from kyma-catalog (port 5434).
  DRIZZLE_DATABASE_URL: z.string().default(
    'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud',
  ),

  // GitHub OAuth (customer login). App ID 3577887, AgentcyLabs.
  GITHUB_CLIENT_ID: z.string().default(''),
  GITHUB_CLIENT_SECRET: z.string().default(''),

  // Resend transactional email (magic links).
  RESEND_API_KEY: z.string().default(''),
  RESEND_FROM_EMAIL: z.string().default('noreply@kyma.dev'),

  // Stripe — all optional; routes return 503 when not configured.
  STRIPE_SECRET_KEY: z.string().optional(),
  STRIPE_WEBHOOK_SIGNING_SECRET: z.string().optional(),
  STRIPE_PRICE_PRO: z.string().optional(),
  STRIPE_PRICE_TEAM: z.string().optional(),

  // 48-byte base64 secret for HS256 session cookies (jose).
  SESSION_SECRET: z.string().min(32),

  // Public URLs.
  CLOUD_BASE_URL: z.string().url().default('http://localhost:3001'),
  ADMIN_BASE_URL: z.string().url().default('http://localhost:3002'),

  // Engine endpoint (used to compose mcp_endpoint per workspace).
  KYMA_ENGINE_BASE_URL: z.string().url().default('http://localhost:8080'),

  // Admin allowlist — comma-separated GitHub user IDs (numeric).
  KYMA_ADMIN_GITHUB_IDS: z.string().default(''),
});

export type Env = z.infer<typeof envSchema>;

let _env: Env | null = null;
export function getEnv(): Env {
  if (_env) return _env;
  const parsed = envSchema.safeParse(process.env);
  if (!parsed.success) {
    console.error('Invalid env:');
    for (const issue of parsed.error.issues) {
      console.error(`  ${issue.path.join('.')}: ${issue.message}`);
    }
    process.exit(1);
  }
  _env = parsed.data;
  return _env;
}
```

- [ ] **Step 6: Run `pnpm install` from cloud/**

Run: `cd /Users/shaked/projects_new/agentcy/kyma/cloud && pnpm install`
Expected: api package compiles its deps, no errors.

- [ ] **Step 7: Commit**

```bash
git add cloud/api
git commit -m "feat(cloud/api): scaffold Hono package, zod env loader, vitest harness"
```

---

### Task 4: Drizzle schema for cloud control plane

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/db/schema.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/db/client.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/db/migrate.ts`

**Critical schema constraint** — the engine's `DbAuthBackend` (`crates/kyma-server/src/auth/db_backend.rs`) reads `api_tokens` rows with these exact columns. Match them exactly:

```sql
api_tokens (
  id            uuid PK,
  tenant_id     uuid NOT NULL,         -- = workspace.id
  token_hash    bytea NOT NULL UNIQUE, -- raw SHA-256 bytes (32)
  scopes        text NOT NULL,         -- "read,write,admin" csv
  subject       text NULL,
  last_used_at  timestamptz NULL,
  revoked_at    timestamptz NULL,
  created_at    timestamptz NOT NULL DEFAULT now()
)
```

The engine has no `name`, `prefix`, `expires_at`, or `workspace_id` columns. We add them only as Drizzle-level metadata if needed by the cloud (we add `name` and `prefix` as additional nullable columns; the engine ignores them).

- [ ] **Step 1: Write `client.ts`**

```ts
import { drizzle } from 'drizzle-orm/node-postgres';
import pg from 'pg';
import * as schema from './schema.js';
import { getEnv } from '../env.js';

let _pool: pg.Pool | null = null;
let _db: ReturnType<typeof drizzle<typeof schema>> | null = null;

export function getDb() {
  if (_db) return _db;
  _pool = new pg.Pool({
    connectionString: getEnv().DRIZZLE_DATABASE_URL,
    max: 50,
    idleTimeoutMillis: 30_000,
    connectionTimeoutMillis: 5_000,
  });
  _db = drizzle(_pool, { schema });
  return _db;
}

export function getPool() {
  if (!_pool) getDb();
  return _pool!;
}

export async function closeDb() {
  if (_pool) {
    await _pool.end();
    _pool = null;
    _db = null;
  }
}

export { schema };
```

- [ ] **Step 2: Write `schema.ts` — full schema**

```ts
import {
  pgTable, uuid, varchar, text, boolean, integer, bigint, real,
  timestamp, jsonb, uniqueIndex, index, primaryKey, customType,
} from 'drizzle-orm/pg-core';

// bytea custom type — Drizzle's native Buffer mapping.
const bytea = customType<{ data: Buffer; driverData: Buffer }>({
  dataType() { return 'bytea'; },
});

// ─── users ─────────────────────────────────────────────────────────────────
export const users = pgTable('users', {
  id: uuid('id').primaryKey().defaultRandom(),
  githubId: varchar('github_id', { length: 64 }).unique(),
  email: varchar('email', { length: 255 }).notNull().unique(),
  name: varchar('name', { length: 255 }),
  avatarUrl: text('avatar_url'),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── workspaces ────────────────────────────────────────────────────────────
export const workspaces = pgTable('workspaces', {
  id: uuid('id').primaryKey().defaultRandom(),
  slug: varchar('slug', { length: 64 }).notNull().unique(),
  name: varchar('name', { length: 255 }).notNull(),
  ownerUserId: uuid('owner_user_id').notNull().references(() => users.id),
  plan: varchar('plan', { length: 32 }).notNull().default('free'),
  planActive: boolean('plan_active').notNull().default(true),
  kind: varchar('kind', { length: 32 }).notNull().default('shared'),
  kymaEndpoint: text('kyma_endpoint').notNull(),
  mcpEndpoint: text('mcp_endpoint').notNull(),
  stripeCustomerId: varchar('stripe_customer_id', { length: 255 }).unique(),
  stripeSubscriptionId: varchar('stripe_subscription_id', { length: 255 }).unique(),
  trialEndsAt: timestamp('trial_ends_at', { withTimezone: true }),
  subscriptionPeriodEnd: timestamp('subscription_period_end', { withTimezone: true }),
  dunningState: varchar('dunning_state', { length: 16 }),
  dedicatedRailwayProjectId: varchar('dedicated_railway_project_id', { length: 255 }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── workspace_members ─────────────────────────────────────────────────────
export const workspaceMembers = pgTable('workspace_members', {
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  userId: uuid('user_id').notNull().references(() => users.id, { onDelete: 'cascade' }),
  role: varchar('role', { length: 32 }).notNull().default('member'),
  joinedAt: timestamp('joined_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  primaryKey({ columns: [t.workspaceId, t.userId] }),
]);

// ─── api_tokens ────────────────────────────────────────────────────────────
// Shared with kyma-server's DbAuthBackend (crates/kyma-server/src/auth/db_backend.rs).
// MUST match these columns exactly: tenant_id, token_hash (bytea), scopes (text),
// subject, last_used_at, revoked_at, created_at. Extra columns (workspace_id,
// name, prefix) are cloud-side only and ignored by the engine.
export const apiTokens = pgTable('api_tokens', {
  id: uuid('id').primaryKey().defaultRandom(),
  tenantId: uuid('tenant_id').notNull(),                 // = workspace.id
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  tokenHash: bytea('token_hash').notNull(),
  scopes: text('scopes').notNull().default('read,write'),
  subject: text('subject'),
  name: varchar('name', { length: 128 }),
  prefix: varchar('prefix', { length: 32 }),
  createdByUserId: uuid('created_by_user_id').references(() => users.id, { onDelete: 'set null' }),
  lastUsedAt: timestamp('last_used_at', { withTimezone: true }),
  revokedAt: timestamp('revoked_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  uniqueIndex('api_tokens_token_hash_uniq').on(t.tokenHash),
  index('api_tokens_workspace_idx').on(t.workspaceId),
  index('api_tokens_tenant_idx').on(t.tenantId),
]);

// ─── magic_links ───────────────────────────────────────────────────────────
export const magicLinks = pgTable('magic_links', {
  id: uuid('id').primaryKey().defaultRandom(),
  email: varchar('email', { length: 255 }).notNull(),
  tokenHash: bytea('token_hash').notNull(),
  expiresAt: timestamp('expires_at', { withTimezone: true }).notNull(),
  consumedAt: timestamp('consumed_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  uniqueIndex('magic_links_token_hash_uniq').on(t.tokenHash),
  index('magic_links_email_idx').on(t.email),
]);

// ─── billing_events ────────────────────────────────────────────────────────
export const billingEvents = pgTable('billing_events', {
  id: uuid('id').primaryKey().defaultRandom(),
  stripeEventId: varchar('stripe_event_id', { length: 255 }).notNull().unique(),
  eventType: varchar('event_type', { length: 255 }).notNull(),
  workspaceId: uuid('workspace_id').references(() => workspaces.id, { onDelete: 'set null' }),
  payload: jsonb('payload').notNull(),
  processed: boolean('processed').notNull().default(false),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
});

// ─── credits ───────────────────────────────────────────────────────────────
export const credits = pgTable('credits', {
  id: uuid('id').primaryKey().defaultRandom(),
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  amount: real('amount').notNull(),
  remaining: real('remaining').notNull(),
  source: varchar('source', { length: 64 }).notNull(),
  stripePaymentId: varchar('stripe_payment_id', { length: 255 }),
  expiresAt: timestamp('expires_at', { withTimezone: true }),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('credits_workspace_idx').on(t.workspaceId),
]);

// ─── usage_events ──────────────────────────────────────────────────────────
// Append-only — kind in: 'compute_seconds' | 'ingest_bytes' | 'agent_calls' | 'mcp_calls'
export const usageEvents = pgTable('usage_events', {
  id: uuid('id').primaryKey().defaultRandom(),
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  kind: varchar('kind', { length: 64 }).notNull(),
  value: bigint('value', { mode: 'number' }).notNull(),
  occurredAt: timestamp('occurred_at', { withTimezone: true }).notNull(),
  recordedAt: timestamp('recorded_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('usage_events_workspace_occurred_idx').on(t.workspaceId, t.occurredAt),
]);

// ─── usage_daily ───────────────────────────────────────────────────────────
export const usageDaily = pgTable('usage_daily', {
  workspaceId: uuid('workspace_id').notNull().references(() => workspaces.id, { onDelete: 'cascade' }),
  day: timestamp('day', { withTimezone: true, mode: 'date' }).notNull(),
  kind: varchar('kind', { length: 64 }).notNull(),
  total: bigint('total', { mode: 'number' }).notNull().default(0),
  updatedAt: timestamp('updated_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  primaryKey({ columns: [t.workspaceId, t.day, t.kind] }),
]);

// ─── platform_admin_audit ──────────────────────────────────────────────────
export const platformAdminAudit = pgTable('platform_admin_audit', {
  id: uuid('id').primaryKey().defaultRandom(),
  adminUserId: uuid('admin_user_id').references(() => users.id, { onDelete: 'set null' }),
  action: varchar('action', { length: 64 }).notNull(),
  targetUserId: uuid('target_user_id').references(() => users.id, { onDelete: 'set null' }),
  targetWorkspaceId: uuid('target_workspace_id').references(() => workspaces.id, { onDelete: 'set null' }),
  payload: jsonb('payload').notNull().default({}),
  createdAt: timestamp('created_at', { withTimezone: true }).notNull().defaultNow(),
}, (t) => [
  index('platform_admin_audit_created_at_idx').on(t.createdAt),
]);
```

- [ ] **Step 3: Write `migrate.ts`**

```ts
import 'dotenv/config';
import { migrate } from 'drizzle-orm/node-postgres/migrator';
import { getDb, closeDb } from './client.js';

async function main() {
  console.log('[cloud] running migrations...');
  await migrate(getDb(), { migrationsFolder: './src/db/migrations' });
  console.log('[cloud] migrations complete.');
  await closeDb();
}

main().catch((err) => {
  console.error('[cloud] migration failed:', err);
  process.exit(1);
});
```

- [ ] **Step 4: Boot Postgres + generate migrations**

```bash
cd /Users/shaked/projects_new/agentcy/kyma/cloud
docker-compose up -d postgres
sleep 3
pnpm --filter @kyma/cloud-api db:generate
```

Expected: a new `0000_*.sql` file appears under `cloud/api/src/db/migrations/`.

- [ ] **Step 5: Apply migration against local Postgres**

```bash
pnpm --filter @kyma/cloud-api db:migrate
```

Expected: "[cloud] migrations complete."

- [ ] **Step 6: Smoke-test: `psql` round-trip on `api_tokens`**

```bash
docker exec -i $(docker ps -qf 'ancestor=postgres:16-alpine') \
  psql -U kyma -d kyma_cloud -c "\d api_tokens"
```
Expected: confirm `tenant_id uuid`, `token_hash bytea`, `scopes text`, `subject text` columns are present (matches the engine's expected DDL).

- [ ] **Step 7: Commit**

```bash
git add cloud/api/src/db cloud/api/src/db/migrations
git commit -m "feat(cloud/api): drizzle schema mirroring engine api_tokens contract"
```

---

### Task 5: Vitest harness with disposable Postgres database per test file

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/test-setup.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/db/schema.test.ts`

- [ ] **Step 1: test-setup.ts**

```ts
// Globals for Vitest. Each test file should call `await freshDb()` in its
// beforeAll to get an isolated logical DB.
import { execSync } from 'node:child_process';
import { closeDb } from './db/client.js';

export async function freshDb(): Promise<void> {
  const url = process.env.DRIZZLE_DATABASE_URL ??
    'postgres://kyma:kyma_dev@localhost:5434/kyma_cloud_test';
  process.env.DRIZZLE_DATABASE_URL = url;
  const dbName = url.split('/').pop()!;
  const adminUrl = url.replace(/\/[^/]+$/, '/postgres');
  execSync(
    `psql "${adminUrl}" -c "DROP DATABASE IF EXISTS ${dbName} WITH (FORCE);"`,
    { stdio: 'pipe' },
  );
  execSync(`psql "${adminUrl}" -c "CREATE DATABASE ${dbName};"`, { stdio: 'pipe' });
  execSync('pnpm --filter @kyma/cloud-api run db:migrate', { stdio: 'inherit' });
  await closeDb();
}
```

- [ ] **Step 2: schema.test.ts — round-trip the load-bearing api_tokens row**

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { createHash } from 'node:crypto';
import { eq } from 'drizzle-orm';
import { freshDb } from '../test-setup.js';
import { getDb, closeDb, schema } from './client.js';

describe('schema: api_tokens contract with engine DbAuthBackend', () => {
  beforeAll(async () => { await freshDb(); });
  afterAll(async () => { await closeDb(); });

  it('round-trips a token row with bytea token_hash and csv scopes', async () => {
    const db = getDb();
    const [user] = await db.insert(schema.users).values({
      email: 'a@b.c', githubId: '111', name: 'a',
    }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo', name: 'Demo', ownerUserId: user.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();
    const tokenHash = createHash('sha256').update('kyma_test_token').digest();
    expect(tokenHash.length).toBe(32);
    const [tok] = await db.insert(schema.apiTokens).values({
      tenantId: ws.id,
      workspaceId: ws.id,
      tokenHash,
      scopes: 'read,write',
      name: 't1',
    }).returning();
    expect(tok.scopes).toBe('read,write');
    expect(Buffer.from(tok.tokenHash).equals(tokenHash)).toBe(true);
    // Engine query shape — verify the SELECT used by DbAuthBackend works:
    const rows = await db
      .select({ tid: schema.apiTokens.tenantId, sc: schema.apiTokens.scopes })
      .from(schema.apiTokens)
      .where(eq(schema.apiTokens.tokenHash, tokenHash));
    expect(rows[0].tid).toBe(ws.id);
    expect(rows[0].sc).toBe('read,write');
  });
});
```

- [ ] **Step 3: Run the test**

```bash
cd /Users/shaked/projects_new/agentcy/kyma/cloud
pnpm --filter @kyma/cloud-api run test
```

Expected: `1 passed`.

- [ ] **Step 4: Commit**

```bash
git add cloud/api/src/test-setup.ts cloud/api/src/db/schema.test.ts
git commit -m "test(cloud/api): vitest harness + api_tokens engine-contract test"
```

---

### Task 6: Error helpers and signed session cookies

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/lib/errors.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/lib/sessions.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/lib/sessions.test.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/lib/tokens.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/lib/tokens.test.ts`

- [ ] **Step 1: errors.ts**

```ts
import { HTTPException } from 'hono/http-exception';

export class AppError extends HTTPException {
  constructor(public statusCode: number, public code: string, message: string) {
    super(statusCode as any, { message });
  }
}

export const badRequest    = (m: string, c = 'BAD_REQUEST')    => new AppError(400, c, m);
export const unauthorized  = (m = 'Unauthorized', c = 'UNAUTHORIZED')  => new AppError(401, c, m);
export const forbidden     = (m = 'Forbidden', c = 'FORBIDDEN')        => new AppError(403, c, m);
export const notFound      = (m = 'Not found', c = 'NOT_FOUND')        => new AppError(404, c, m);
export const conflict      = (m: string, c = 'CONFLICT')               => new AppError(409, c, m);
```

- [ ] **Step 2: tokens.ts (token gen + sha256)**

```ts
import { createHash, randomBytes } from 'node:crypto';

const TOKEN_PREFIX = 'kyma_';

export function generateMcpToken(): { plain: string; hash: Buffer; prefix: string } {
  const raw = randomBytes(32).toString('hex');             // 64 hex = 256 bits
  const plain = `${TOKEN_PREFIX}${raw}`;
  const prefix = `${TOKEN_PREFIX}${raw.slice(0, 8)}`;
  const hash = hashToken(plain);
  return { plain, hash, prefix };
}

/** Returns the raw 32-byte SHA-256 — matches the engine's DbAuthBackend hashing. */
export function hashToken(plain: string): Buffer {
  return createHash('sha256').update(plain).digest();
}

export function randomBytesHex(n: number): string {
  return randomBytes(n).toString('hex');
}
```

- [ ] **Step 3: tokens.test.ts**

```ts
import { describe, it, expect } from 'vitest';
import { createHash } from 'node:crypto';
import { generateMcpToken, hashToken } from './tokens.js';

describe('tokens', () => {
  it('mints a 69-char token with kyma_ prefix and a 32-byte hash', () => {
    const t = generateMcpToken();
    expect(t.plain.length).toBe(69);                 // 5 + 64
    expect(t.plain.startsWith('kyma_')).toBe(true);
    expect(t.hash.length).toBe(32);
    expect(t.prefix.length).toBe(13);                // 5 + 8
  });

  it('hashToken matches plain crypto.createHash output (engine compat)', () => {
    const expected = createHash('sha256').update('kyma_x').digest();
    expect(hashToken('kyma_x').equals(expected)).toBe(true);
  });
});
```

- [ ] **Step 4: sessions.ts (HS256 signed cookie via jose)**

```ts
import * as jose from 'jose';
import { getEnv } from '../env.js';

export interface SessionClaims {
  sub: string;        // user.id
  email: string;
}

const ISS = 'kyma-cloud';
const COOKIE_NAME = 'kyma_session';
const TTL = '30d';

function secret(): Uint8Array {
  return new TextEncoder().encode(getEnv().SESSION_SECRET);
}

export async function signSessionCookie(claims: SessionClaims): Promise<string> {
  return new jose.SignJWT({ ...claims })
    .setProtectedHeader({ alg: 'HS256' })
    .setIssuedAt()
    .setExpirationTime(TTL)
    .setIssuer(ISS)
    .sign(secret());
}

export async function verifySessionCookie(jwt: string): Promise<SessionClaims> {
  const { payload } = await jose.jwtVerify(jwt, secret(), { issuer: ISS });
  return { sub: payload.sub as string, email: payload.email as string };
}

export const SESSION_COOKIE_NAME = COOKIE_NAME;
```

- [ ] **Step 5: sessions.test.ts**

```ts
import { describe, it, expect, beforeAll } from 'vitest';
import { signSessionCookie, verifySessionCookie } from './sessions.js';

describe('sessions', () => {
  beforeAll(() => { process.env.SESSION_SECRET = 'a'.repeat(48); });

  it('round-trips a signed claim', async () => {
    const jwt = await signSessionCookie({ sub: 'u1', email: 'a@b.c' });
    const claims = await verifySessionCookie(jwt);
    expect(claims.sub).toBe('u1');
    expect(claims.email).toBe('a@b.c');
  });

  it('rejects a tampered cookie', async () => {
    const jwt = await signSessionCookie({ sub: 'u1', email: 'a@b.c' });
    const tampered = jwt.slice(0, -2) + 'aa';
    await expect(verifySessionCookie(tampered)).rejects.toThrow();
  });
});
```

- [ ] **Step 6: Run tests**

Run: `pnpm --filter @kyma/cloud-api run test`
Expected: 4 passed (schema + sessions + tokens).

- [ ] **Step 7: Commit**

```bash
git add cloud/api/src/lib
git commit -m "feat(cloud/api): error helpers, signed sessions, MCP token mint"
```

---

### Task 7: Hono app skeleton with health route + error handler

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.test.ts`

- [ ] **Step 1: index.ts**

```ts
import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { cors } from 'hono/cors';
import { logger } from 'hono/logger';
import { getEnv } from './env.js';
import { AppError } from './lib/errors.js';

export function buildApp() {
  const app = new Hono();
  app.use('*', logger());
  app.use('*', cors({
    origin: (o) => o || '*',
    credentials: true,
    allowMethods: ['GET', 'POST', 'PATCH', 'DELETE', 'OPTIONS'],
    allowHeaders: ['Content-Type', 'Authorization', 'Cookie'],
  }));

  app.get('/health', (c) => c.json({ status: 'ok', service: 'kyma-cloud-api' }));

  app.onError((err, c) => {
    if (err instanceof AppError) {
      return c.json({ error: { code: err.code, message: err.message } }, err.statusCode as any);
    }
    console.error('[cloud] unhandled:', err);
    return c.json({ error: { code: 'INTERNAL_ERROR', message: 'Internal server error' } }, 500);
  });

  app.notFound((c) => c.json({ error: { code: 'NOT_FOUND', message: 'Route not found' } }, 404));

  return app;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const env = getEnv();
  const app = buildApp();
  console.log(`[cloud] listening on :${env.PORT}`);
  serve({ fetch: app.fetch, port: env.PORT });
}
```

- [ ] **Step 2: index.test.ts**

```ts
import { describe, it, expect } from 'vitest';
import { buildApp } from './index.js';

describe('app', () => {
  it('GET /health returns ok', async () => {
    const res = await buildApp().request('/health');
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ status: 'ok', service: 'kyma-cloud-api' });
  });
  it('GET /unknown returns 404 envelope', async () => {
    const res = await buildApp().request('/unknown');
    expect(res.status).toBe(404);
    const body = await res.json() as any;
    expect(body.error.code).toBe('NOT_FOUND');
  });
});
```

- [ ] **Step 3: Run tests**

Run: `pnpm --filter @kyma/cloud-api run test`
Expected: 6 passed.

- [ ] **Step 4: Commit**

```bash
git add cloud/api/src/index.ts cloud/api/src/index.test.ts
git commit -m "feat(cloud/api): Hono app skeleton with /health and error envelope"
```

---

### Task 8: GitHub OAuth start + callback

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/auth.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/auth.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/auth.service.test.ts`

GitHub OAuth callback URL: `${CLOUD_BASE_URL}/api/auth/github/callback`. CSRF guard via random `state` param stashed in a short-lived `kyma_oauth_state` cookie.

- [ ] **Step 1: auth.service.ts — pure logic, no Hono dependency for testability**

```ts
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import { badRequest, unauthorized } from '../lib/errors.js';
import { randomBytesHex } from '../lib/tokens.js';

export function buildGithubAuthorizeUrl(state: string, redirectUri: string): string {
  const params = new URLSearchParams({
    client_id: getEnv().GITHUB_CLIENT_ID,
    redirect_uri: redirectUri,
    scope: 'read:user user:email',
    state,
  });
  return `https://github.com/login/oauth/authorize?${params}`;
}

export function newOauthState(): string { return randomBytesHex(16); }

export async function exchangeGithubCode(code: string, redirectUri: string): Promise<{
  user: typeof schema.users.$inferSelect;
}> {
  const env = getEnv();
  const tokRes = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { 'Accept': 'application/json', 'Content-Type': 'application/json' },
    body: JSON.stringify({
      client_id: env.GITHUB_CLIENT_ID,
      client_secret: env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: redirectUri,
    }),
  });
  if (!tokRes.ok) throw badRequest('GitHub token exchange failed', 'GH_TOKEN_FAIL');
  const tok = await tokRes.json() as { access_token?: string; error?: string };
  if (!tok.access_token) throw unauthorized(tok.error ?? 'No access token from GitHub');

  const ghHeaders = {
    'Authorization': `Bearer ${tok.access_token}`,
    'User-Agent': 'kyma-cloud',
    'Accept': 'application/vnd.github+json',
  };
  const [profileRes, emailsRes] = await Promise.all([
    fetch('https://api.github.com/user', { headers: ghHeaders }),
    fetch('https://api.github.com/user/emails', { headers: ghHeaders }),
  ]);
  if (!profileRes.ok) throw unauthorized('GitHub profile fetch failed');
  const profile = await profileRes.json() as {
    id: number; login: string; name: string | null; avatar_url: string | null; email: string | null;
  };
  const emails = emailsRes.ok ? await emailsRes.json() as Array<{ email: string; primary: boolean; verified: boolean }> : [];
  const primaryEmail =
    emails.find(e => e.primary && e.verified)?.email
    ?? profile.email
    ?? null;
  if (!primaryEmail) throw badRequest('GitHub account has no verified primary email', 'NO_EMAIL');

  const db = getDb();
  const ghIdStr = String(profile.id);

  // Upsert by github_id; fallback to email match for existing magic-link users.
  let [user] = await db.select().from(schema.users).where(eq(schema.users.githubId, ghIdStr)).limit(1);
  if (!user) {
    [user] = await db.select().from(schema.users).where(eq(schema.users.email, primaryEmail)).limit(1);
    if (user) {
      [user] = await db.update(schema.users).set({
        githubId: ghIdStr,
        avatarUrl: profile.avatar_url,
        name: user.name ?? profile.name ?? profile.login,
        updatedAt: new Date(),
      }).where(eq(schema.users.id, user.id)).returning();
    } else {
      [user] = await db.insert(schema.users).values({
        githubId: ghIdStr,
        email: primaryEmail,
        name: profile.name ?? profile.login,
        avatarUrl: profile.avatar_url,
      }).returning();
    }
  }
  return { user };
}
```

- [ ] **Step 2: routes/auth.ts**

```ts
import { Hono } from 'hono';
import { setCookie, getCookie, deleteCookie } from 'hono/cookie';
import { getEnv } from '../env.js';
import { badRequest, unauthorized } from '../lib/errors.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import * as auth from '../services/auth.service.js';

const STATE_COOKIE = 'kyma_oauth_state';

export const authRoutes = new Hono();

// GET /api/auth/github/start
authRoutes.get('/github/start', (c) => {
  const env = getEnv();
  const state = auth.newOauthState();
  setCookie(c, STATE_COOKIE, state, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 600,
  });
  const redirectUri = `${env.CLOUD_BASE_URL}/api/auth/github/callback`;
  return c.redirect(auth.buildGithubAuthorizeUrl(state, redirectUri));
});

// GET /api/auth/github/callback?code&state
authRoutes.get('/github/callback', async (c) => {
  const env = getEnv();
  const code = c.req.query('code');
  const state = c.req.query('state');
  const cookieState = getCookie(c, STATE_COOKIE);
  if (!code || !state) throw badRequest('Missing code or state');
  if (!cookieState || cookieState !== state) throw unauthorized('OAuth state mismatch (CSRF)');
  deleteCookie(c, STATE_COOKIE, { path: '/' });

  const redirectUri = `${env.CLOUD_BASE_URL}/api/auth/github/callback`;
  const { user } = await auth.exchangeGithubCode(code, redirectUri);

  const jwt = await signSessionCookie({ sub: user.id, email: user.email });
  setCookie(c, SESSION_COOKIE_NAME, jwt, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 60 * 60 * 24 * 30,
  });
  return c.redirect(`${env.CLOUD_BASE_URL}/workspaces`);
});

// POST /api/auth/logout
authRoutes.post('/logout', (c) => {
  deleteCookie(c, SESSION_COOKIE_NAME, { path: '/' });
  return c.json({ ok: true });
});
```

- [ ] **Step 3: Mount in index.ts**

Add to `buildApp()` after the cors block:

```ts
import { authRoutes } from './routes/auth.js';
// ...
app.route('/api/auth', authRoutes);
```

- [ ] **Step 4: auth.service.test.ts — verifies state generator and authorize URL**

```ts
import { describe, it, expect, beforeAll } from 'vitest';
import { buildGithubAuthorizeUrl, newOauthState } from './auth.service.js';

beforeAll(() => { process.env.GITHUB_CLIENT_ID = 'client_xyz'; process.env.SESSION_SECRET = 'a'.repeat(48); });

describe('auth.service', () => {
  it('newOauthState returns 32 hex chars', () => {
    const s = newOauthState();
    expect(s.length).toBe(32);
    expect(/^[0-9a-f]+$/.test(s)).toBe(true);
  });
  it('buildGithubAuthorizeUrl encodes redirect + state', () => {
    const url = buildGithubAuthorizeUrl('abc', 'http://localhost/cb');
    expect(url).toContain('client_id=client_xyz');
    expect(url).toContain('state=abc');
    expect(url).toContain('redirect_uri=http%3A%2F%2Flocalhost%2Fcb');
  });
});
```

- [ ] **Step 5: Run tests + smoke /api/auth/github/start**

```bash
pnpm --filter @kyma/cloud-api run test
pnpm --filter @kyma/cloud-api run dev &
curl -i -c /tmp/cookies http://localhost:3003/api/auth/github/start
# Expected: 302 with Location: github.com/login/oauth/authorize?... and a Set-Cookie kyma_oauth_state
kill %1
```

- [ ] **Step 6: Commit**

```bash
git add cloud/api/src/services/auth.service.ts cloud/api/src/services/auth.service.test.ts \
        cloud/api/src/routes/auth.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): GitHub OAuth start + callback + logout"
```

---

### Task 9: Magic-link issue + exchange (hand-rolled)

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/auth.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/email.service.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/auth.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/email.service.test.ts`

- [ ] **Step 1: email.service.ts — Resend wrapper**

```ts
import { Resend } from 'resend';
import { getEnv } from '../env.js';

let _resend: Resend | null = null;
function client() {
  if (_resend) return _resend;
  const key = getEnv().RESEND_API_KEY;
  if (!key) throw new Error('RESEND_API_KEY not set');
  _resend = new Resend(key);
  return _resend;
}

export async function sendMagicLinkEmail(to: string, link: string): Promise<void> {
  const env = getEnv();
  if (!env.RESEND_API_KEY) {
    console.log(`[email] DEV: magic link for ${to}: ${link}`);
    return;
  }
  await client().emails.send({
    from: env.RESEND_FROM_EMAIL,
    to,
    subject: 'Sign in to kyma cloud',
    html: `
      <div style="font-family: 'IBM Plex Sans', system-ui, sans-serif; max-width: 480px; margin: 0 auto; padding: 32px;">
        <h1 style="font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 22px; margin: 0 0 16px;">kyma cloud</h1>
        <p>Click the link below to sign in. It expires in 15 minutes.</p>
        <p style="margin: 24px 0;">
          <a href="${link}"
             style="display: inline-block; background: #2d6f1f; color: white; padding: 12px 24px;
                    border-radius: 6px; text-decoration: none; font-weight: 600;">
            Sign in to kyma cloud
          </a>
        </p>
        <p style="color: #767880; font-size: 13px;">
          If you didn't request this, you can safely ignore this email.
        </p>
      </div>
    `,
  });
}
```

- [ ] **Step 2: Append magic-link to auth.service.ts**

```ts
// (append to auth.service.ts)
import { hashToken } from '../lib/tokens.js';
import { and, gt, isNull, eq as eqOp } from 'drizzle-orm';

export async function issueMagicLink(email: string): Promise<{ link: string }> {
  const db = getDb();
  const env = getEnv();
  const raw = randomBytesHex(32);
  const tokenHash = hashToken(raw);
  const expiresAt = new Date(Date.now() + 15 * 60 * 1000);
  await db.insert(schema.magicLinks).values({ email, tokenHash, expiresAt });
  return { link: `${env.CLOUD_BASE_URL}/login/callback?token=${raw}` };
}

export async function exchangeMagicLink(rawToken: string): Promise<{
  user: typeof schema.users.$inferSelect;
}> {
  const db = getDb();
  const tokenHash = hashToken(rawToken);
  const [row] = await db.select().from(schema.magicLinks)
    .where(and(
      eqOp(schema.magicLinks.tokenHash, tokenHash),
      gt(schema.magicLinks.expiresAt, new Date()),
      isNull(schema.magicLinks.consumedAt),
    ))
    .limit(1);
  if (!row) throw unauthorized('Invalid or expired magic link');

  await db.update(schema.magicLinks)
    .set({ consumedAt: new Date() })
    .where(eqOp(schema.magicLinks.id, row.id));

  let [user] = await db.select().from(schema.users).where(eqOp(schema.users.email, row.email)).limit(1);
  if (!user) {
    [user] = await db.insert(schema.users).values({ email: row.email }).returning();
  }
  return { user };
}
```

- [ ] **Step 3: Add routes to routes/auth.ts**

```ts
// (append to routes/auth.ts)
import { z } from 'zod';
import { sendMagicLinkEmail } from '../services/email.service.js';

const requestSchema = z.object({ email: z.string().email() });
const exchangeSchema = z.object({ token: z.string().min(32) });

authRoutes.post('/magic-link/request', async (c) => {
  const body = await c.req.json();
  const parsed = requestSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { link } = await auth.issueMagicLink(parsed.data.email);
  await sendMagicLinkEmail(parsed.data.email, link);
  return c.json({ ok: true });
});

authRoutes.post('/magic-link/exchange', async (c) => {
  const env = getEnv();
  const body = await c.req.json();
  const parsed = exchangeSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { user } = await auth.exchangeMagicLink(parsed.data.token);
  const jwt = await signSessionCookie({ sub: user.id, email: user.email });
  setCookie(c, SESSION_COOKIE_NAME, jwt, {
    httpOnly: true, sameSite: 'Lax', secure: env.NODE_ENV === 'production',
    path: '/', maxAge: 60 * 60 * 24 * 30,
  });
  return c.json({ user: { id: user.id, email: user.email, name: user.name } });
});
```

- [ ] **Step 4: email.service.test.ts**

```ts
import { describe, it, expect, beforeAll, vi } from 'vitest';
import { sendMagicLinkEmail } from './email.service.js';

beforeAll(() => { delete process.env.RESEND_API_KEY; });

describe('email.service', () => {
  it('logs the link in dev when no Resend key', async () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});
    await sendMagicLinkEmail('a@b.c', 'http://example/link');
    expect(log).toHaveBeenCalledWith(expect.stringContaining('http://example/link'));
    log.mockRestore();
  });
});
```

- [ ] **Step 5: Test issue+exchange end-to-end**

Add `cloud/api/src/services/auth.service.it.test.ts`:

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb } from '../db/client.js';
import { issueMagicLink, exchangeMagicLink } from './auth.service.js';

describe('magic-link round-trip', () => {
  beforeAll(async () => {
    process.env.SESSION_SECRET = 'a'.repeat(48);
    await freshDb();
  });
  afterAll(async () => { await closeDb(); });

  it('issues and exchanges, creating user on first exchange', async () => {
    const { link } = await issueMagicLink('first@kyma.dev');
    const token = link.split('token=')[1];
    const { user } = await exchangeMagicLink(token);
    expect(user.email).toBe('first@kyma.dev');
  });

  it('rejects re-exchange of consumed token', async () => {
    const { link } = await issueMagicLink('again@kyma.dev');
    const token = link.split('token=')[1];
    await exchangeMagicLink(token);
    await expect(exchangeMagicLink(token)).rejects.toThrow();
  });
});
```

Run: `pnpm --filter @kyma/cloud-api run test`
Expected: all tests pass, including magic-link round-trip and re-exchange rejection.

- [ ] **Step 6: Commit**

```bash
git add cloud/api/src/services/email.service.ts cloud/api/src/services/email.service.test.ts \
        cloud/api/src/services/auth.service.ts cloud/api/src/services/auth.service.it.test.ts \
        cloud/api/src/routes/auth.ts
git commit -m "feat(cloud/api): hand-rolled magic-link issue/exchange via Resend"
```

---

### Task 10: Session middleware + `/api/auth/me`

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/middleware/session.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/auth.ts`

- [ ] **Step 1: middleware/session.ts**

```ts
import { createMiddleware } from 'hono/factory';
import { getCookie } from 'hono/cookie';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { unauthorized } from '../lib/errors.js';
import { verifySessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';

export interface SessionContext {
  userId: string;
  email: string;
  name: string | null;
}

export const sessionMiddleware = createMiddleware<{
  Variables: { user: SessionContext };
}>(async (c, next) => {
  const cookie = getCookie(c, SESSION_COOKIE_NAME);
  if (!cookie) throw unauthorized('Not signed in');
  let claims;
  try { claims = await verifySessionCookie(cookie); }
  catch { throw unauthorized('Invalid session'); }
  const db = getDb();
  const [u] = await db.select().from(schema.users)
    .where(eq(schema.users.id, claims.sub)).limit(1);
  if (!u) throw unauthorized('User not found');
  c.set('user', { userId: u.id, email: u.email, name: u.name });
  await next();
});
```

- [ ] **Step 2: Add `/api/auth/me` to routes/auth.ts**

```ts
// (append)
import { sessionMiddleware } from '../middleware/session.js';

authRoutes.get('/me', sessionMiddleware, (c) => {
  const u = c.get('user');
  return c.json({ user: { id: u.userId, email: u.email, name: u.name } });
});
```

- [ ] **Step 3: Test**

Add a smoke test in `cloud/api/src/routes/auth.test.ts`:

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import { buildApp } from '../index.js';

describe('GET /api/auth/me', () => {
  beforeAll(async () => { process.env.SESSION_SECRET = 'a'.repeat(48); await freshDb(); });
  afterAll(async () => { await closeDb(); });

  it('returns user info when cookie is valid', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'me@kyma.dev', name: 'Me' }).returning();
    const jwt = await signSessionCookie({ sub: u.id, email: u.email });
    const res = await buildApp().request('/api/auth/me', {
      headers: { Cookie: `${SESSION_COOKIE_NAME}=${jwt}` },
    });
    expect(res.status).toBe(200);
    const body = await res.json() as any;
    expect(body.user.email).toBe('me@kyma.dev');
  });

  it('returns 401 with no cookie', async () => {
    const res = await buildApp().request('/api/auth/me');
    expect(res.status).toBe(401);
  });
});
```

- [ ] **Step 4: Run tests + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/middleware/session.ts cloud/api/src/routes/auth.ts cloud/api/src/routes/auth.test.ts
git commit -m "feat(cloud/api): session middleware + /api/auth/me"
```

---

### Task 11: Workspace service — create + list + slug uniqueness

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/workspace.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/workspace.service.test.ts`

- [ ] **Step 1: workspace.service.ts**

```ts
import { eq, and } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import { badRequest, conflict, notFound, forbidden } from '../lib/errors.js';
import { PLANS, type PlanId } from '@kyma/shared';

export function slugify(input: string): string {
  return input.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48);
}

export async function listForUser(userId: string) {
  const db = getDb();
  const rows = await db
    .select({
      id: schema.workspaces.id,
      slug: schema.workspaces.slug,
      name: schema.workspaces.name,
      plan: schema.workspaces.plan,
      kind: schema.workspaces.kind,
      planActive: schema.workspaces.planActive,
      mcpEndpoint: schema.workspaces.mcpEndpoint,
      kymaEndpoint: schema.workspaces.kymaEndpoint,
      role: schema.workspaceMembers.role,
      createdAt: schema.workspaces.createdAt,
    })
    .from(schema.workspaceMembers)
    .innerJoin(schema.workspaces, eq(schema.workspaces.id, schema.workspaceMembers.workspaceId))
    .where(eq(schema.workspaceMembers.userId, userId));
  return rows;
}

export async function createWorkspace(userId: string, input: { name: string; slug?: string }) {
  const db = getDb();
  const env = getEnv();
  if (!input.name.trim()) throw badRequest('name is required');

  const baseSlug = input.slug ? slugify(input.slug) : slugify(input.name);
  if (!baseSlug) throw badRequest('slug must contain at least one alphanumeric character');

  // Plan-tier limit (read user's most-permissive workspace plan, default free).
  const existing = await listForUser(userId);
  const userPlan: PlanId = (existing[0]?.plan as PlanId | undefined) ?? 'free';
  if (existing.length >= PLANS[userPlan].maxWorkspaces) {
    throw forbidden(`Plan '${userPlan}' allows at most ${PLANS[userPlan].maxWorkspaces} workspaces.`, 'PLAN_LIMIT');
  }

  let slug = baseSlug;
  let attempt = 0;
  while (true) {
    const [hit] = await db.select({ id: schema.workspaces.id })
      .from(schema.workspaces).where(eq(schema.workspaces.slug, slug)).limit(1);
    if (!hit) break;
    attempt += 1;
    if (attempt > 5) throw conflict(`Slug '${baseSlug}' is taken`);
    slug = `${baseSlug}-${Math.floor(1000 + Math.random() * 9000)}`;
  }

  const [ws] = await db.insert(schema.workspaces).values({
    slug,
    name: input.name.trim(),
    ownerUserId: userId,
    plan: 'free',
    kind: 'shared',
    kymaEndpoint: env.KYMA_ENGINE_BASE_URL,
    // Per-workspace MCP path; matches the topology where the engine routes
    // by workspace-id segment. Slice 2 keeps a single shared engine, so we
    // use {KYMA_ENGINE_BASE_URL}/<wsid>/mcp/v1 — the engine's middleware
    // pulls workspace from the bearer token's tenant_id, the path segment
    // is purely informational so users see "their" URL.
    mcpEndpoint: '',  // backfilled below
  }).returning();

  const [updated] = await db.update(schema.workspaces)
    .set({ mcpEndpoint: `${env.KYMA_ENGINE_BASE_URL}/${ws.id}/mcp/v1` })
    .where(eq(schema.workspaces.id, ws.id))
    .returning();

  await db.insert(schema.workspaceMembers).values({
    workspaceId: ws.id, userId, role: 'owner',
  });
  return updated;
}

export async function getBySlugForUser(userId: string, slug: string) {
  const db = getDb();
  const [row] = await db
    .select({
      ws: schema.workspaces,
      role: schema.workspaceMembers.role,
    })
    .from(schema.workspaces)
    .innerJoin(schema.workspaceMembers, and(
      eq(schema.workspaceMembers.workspaceId, schema.workspaces.id),
      eq(schema.workspaceMembers.userId, userId),
    ))
    .where(eq(schema.workspaces.slug, slug))
    .limit(1);
  if (!row) throw notFound('Workspace not found');
  return { workspace: row.ws, role: row.role };
}
```

- [ ] **Step 2: workspace.service.test.ts**

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { createWorkspace, listForUser, getBySlugForUser, slugify } from './workspace.service.js';

beforeAll(async () => {
  process.env.SESSION_SECRET = 'a'.repeat(48);
  process.env.KYMA_ENGINE_BASE_URL = 'http://e';
  await freshDb();
});
afterAll(async () => { await closeDb(); });

describe('workspace.service', () => {
  it('slugify', () => {
    expect(slugify('My Demo Workspace!')).toBe('my-demo-workspace');
  });

  it('createWorkspace inserts workspace + owner membership + mcpEndpoint', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const ws = await createWorkspace(u.id, { name: 'My Demo' });
    expect(ws.slug).toBe('my-demo');
    expect(ws.kind).toBe('shared');
    expect(ws.plan).toBe('free');
    expect(ws.mcpEndpoint).toBe(`http://e/${ws.id}/mcp/v1`);
    const list = await listForUser(u.id);
    expect(list).toHaveLength(1);
    expect(list[0].role).toBe('owner');
  });

  it('rejects creating beyond the free-plan workspace limit', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'b@b.c' }).returning();
    await createWorkspace(u.id, { name: 'first' });
    await expect(createWorkspace(u.id, { name: 'second' })).rejects.toThrow(/PLAN_LIMIT/);
  });

  it('appends a random suffix on slug collision', async () => {
    const db = getDb();
    const [u1] = await db.insert(schema.users).values({ email: 'c@b.c' }).returning();
    const [u2] = await db.insert(schema.users).values({ email: 'd@b.c' }).returning();
    await createWorkspace(u1.id, { name: 'shared-name' });
    const ws2 = await createWorkspace(u2.id, { name: 'shared-name' });
    expect(ws2.slug).toMatch(/^shared-name-\d{4}$/);
  });

  it('getBySlugForUser rejects non-members', async () => {
    const db = getDb();
    const [u1] = await db.insert(schema.users).values({ email: 'e@b.c' }).returning();
    const [u2] = await db.insert(schema.users).values({ email: 'f@b.c' }).returning();
    const ws = await createWorkspace(u1.id, { name: 'private' });
    await expect(getBySlugForUser(u2.id, ws.slug)).rejects.toThrow(/Workspace not found/);
  });
});
```

- [ ] **Step 3: Run tests**

Run: `pnpm --filter @kyma/cloud-api run test`
Expected: all 4 service tests pass.

- [ ] **Step 4: Commit**

```bash
git add cloud/api/src/services/workspace.service.ts cloud/api/src/services/workspace.service.test.ts
git commit -m "feat(cloud/api): workspace create/list/get with slug-collision handling"
```

---

### Task 12: MCP token mint + revoke + list

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/workspace.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/mcp-token.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/mcp-token.service.test.ts`

- [ ] **Step 1: mcp-token.service.ts**

```ts
import { and, eq, isNull } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { generateMcpToken, hashToken } from '../lib/tokens.js';
import { badRequest, notFound } from '../lib/errors.js';

export interface MintInput {
  workspaceId: string;        // = tenant_id used by engine
  createdByUserId: string;
  name?: string;
  scopes?: Array<'read' | 'write' | 'admin'>;
}

export async function mintMcpToken(input: MintInput): Promise<{
  plain: string; prefix: string; id: string;
}> {
  const db = getDb();
  const scopes = (input.scopes && input.scopes.length ? input.scopes : ['read', 'write']).join(',');
  const { plain, hash, prefix } = generateMcpToken();
  const [row] = await db.insert(schema.apiTokens).values({
    tenantId: input.workspaceId,
    workspaceId: input.workspaceId,
    tokenHash: hash,
    scopes,
    name: input.name?.slice(0, 128) ?? 'mcp',
    prefix,
    createdByUserId: input.createdByUserId,
  }).returning({ id: schema.apiTokens.id });
  return { plain, prefix, id: row.id };
}

export async function listTokens(workspaceId: string) {
  const db = getDb();
  return db.select({
    id: schema.apiTokens.id,
    name: schema.apiTokens.name,
    prefix: schema.apiTokens.prefix,
    scopes: schema.apiTokens.scopes,
    createdAt: schema.apiTokens.createdAt,
    lastUsedAt: schema.apiTokens.lastUsedAt,
    revokedAt: schema.apiTokens.revokedAt,
  })
  .from(schema.apiTokens)
  .where(eq(schema.apiTokens.workspaceId, workspaceId));
}

export async function revokeToken(workspaceId: string, tokenId: string) {
  const db = getDb();
  const [updated] = await db
    .update(schema.apiTokens)
    .set({ revokedAt: new Date() })
    .where(and(
      eq(schema.apiTokens.id, tokenId),
      eq(schema.apiTokens.workspaceId, workspaceId),
      isNull(schema.apiTokens.revokedAt),
    ))
    .returning({ id: schema.apiTokens.id });
  if (!updated) throw notFound('Token not found or already revoked');
}

/**
 * Returns the principal for a given plain token, or null. Mirrors the
 * engine's DbAuthBackend exactly so we can sanity-check from the cloud side
 * (e.g. in admin tools).
 */
export async function authenticateForDebug(plain: string): Promise<
  { tenantId: string; scopes: string[] } | null
> {
  const db = getDb();
  const hash = hashToken(plain);
  const [row] = await db
    .select({ tenantId: schema.apiTokens.tenantId, scopes: schema.apiTokens.scopes })
    .from(schema.apiTokens)
    .where(and(eq(schema.apiTokens.tokenHash, hash), isNull(schema.apiTokens.revokedAt)))
    .limit(1);
  if (!row) return null;
  return { tenantId: row.tenantId, scopes: row.scopes.split(',').map((s) => s.trim()) };
}
```

- [ ] **Step 2: mcp-token.service.test.ts**

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { mintMcpToken, listTokens, revokeToken, authenticateForDebug } from './mcp-token.service.js';
import { hashToken } from '../lib/tokens.js';

beforeAll(async () => { process.env.KYMA_ENGINE_BASE_URL = 'http://e'; await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('mcp-token.service', () => {
  it('mints a token whose hash matches the engine contract', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo', name: 'Demo', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();

    const { plain, prefix, id } = await mintMcpToken({
      workspaceId: ws.id, createdByUserId: u.id, name: 'cli',
    });
    expect(plain.startsWith('kyma_')).toBe(true);
    expect(prefix.startsWith('kyma_')).toBe(true);
    expect(id).toBeTruthy();

    // The exact lookup the engine performs (SELECT tenant_id, scopes ... WHERE token_hash = $1):
    const resolved = await authenticateForDebug(plain);
    expect(resolved?.tenantId).toBe(ws.id);
    expect(resolved?.scopes).toEqual(['read', 'write']);
  });

  it('listTokens returns rows with prefix only (no plain)', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'b@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo2', name: 'Demo2', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/y/mcp/v1',
    }).returning();
    await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });
    const list = await listTokens(ws.id);
    expect(list).toHaveLength(1);
    expect(list[0].prefix?.startsWith('kyma_')).toBe(true);
    expect((list[0] as any).tokenHash).toBeUndefined();
  });

  it('revokeToken sets revoked_at and the lookup falls through', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'c@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'demo3', name: 'Demo3', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/z/mcp/v1',
    }).returning();
    const { plain, id } = await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });
    await revokeToken(ws.id, id);
    expect(await authenticateForDebug(plain)).toBeNull();
  });
});
```

- [ ] **Step 3: Run + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/services/mcp-token.service.ts cloud/api/src/services/mcp-token.service.test.ts
git commit -m "feat(cloud/api): MCP token mint/list/revoke matching engine DbAuthBackend contract"
```

---

### Task 13: Workspace + token routes

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/workspaces.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/workspaces.test.ts`

- [ ] **Step 1: routes/workspaces.ts**

```ts
import { Hono } from 'hono';
import { z } from 'zod';
import { sessionMiddleware } from '../middleware/session.js';
import { badRequest, forbidden } from '../lib/errors.js';
import * as ws from '../services/workspace.service.js';
import * as tok from '../services/mcp-token.service.js';

export const workspaceRoutes = new Hono();
workspaceRoutes.use('*', sessionMiddleware);

const createSchema = z.object({
  name: z.string().min(1).max(255),
  slug: z.string().min(1).max(64).optional(),
});
const tokenCreateSchema = z.object({
  name: z.string().max(128).optional(),
  scopes: z.array(z.enum(['read', 'write', 'admin'])).optional(),
});

workspaceRoutes.get('/', async (c) => {
  const u = c.get('user');
  const list = await ws.listForUser(u.userId);
  return c.json({ workspaces: list });
});

workspaceRoutes.post('/', async (c) => {
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = createSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const created = await ws.createWorkspace(u.userId, parsed.data);
  return c.json({ workspace: created }, 201);
});

workspaceRoutes.get('/:slug', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({ workspace, role });
});

workspaceRoutes.get('/:slug/tokens', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({ tokens: await tok.listTokens(workspace.id) });
});

workspaceRoutes.post('/:slug/tokens', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can mint tokens');
  const body = await c.req.json().catch(() => ({}));
  const parsed = tokenCreateSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const minted = await tok.mintMcpToken({
    workspaceId: workspace.id,
    createdByUserId: u.userId,
    name: parsed.data.name,
    scopes: parsed.data.scopes,
  });
  return c.json({
    token: minted.plain,           // returned ONCE — never again
    prefix: minted.prefix,
    id: minted.id,
    mcpEndpoint: workspace.mcpEndpoint,
  }, 201);
});

workspaceRoutes.post('/:slug/tokens/:id/revoke', async (c) => {
  const u = c.get('user');
  const { workspace, role } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can revoke tokens');
  await tok.revokeToken(workspace.id, c.req.param('id'));
  return c.json({ ok: true });
});
```

- [ ] **Step 2: Mount in index.ts**

Add:

```ts
import { workspaceRoutes } from './routes/workspaces.js';
app.route('/api/workspaces', workspaceRoutes);
```

- [ ] **Step 3: HTTP test (auth flow + create + mint + revoke)**

In `cloud/api/src/routes/workspaces.test.ts`:

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';
import { signSessionCookie, SESSION_COOKIE_NAME } from '../lib/sessions.js';
import { buildApp } from '../index.js';

let cookie = '';
let userId = '';

beforeAll(async () => {
  process.env.SESSION_SECRET = 'a'.repeat(48);
  process.env.KYMA_ENGINE_BASE_URL = 'http://engine.local';
  await freshDb();
  const db = getDb();
  const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
  userId = u.id;
  cookie = `${SESSION_COOKIE_NAME}=${await signSessionCookie({ sub: u.id, email: u.email })}`;
});
afterAll(async () => { await closeDb(); });

describe('workspaces routes', () => {
  it('POST creates a workspace with the owning member', async () => {
    const res = await buildApp().request('/api/workspaces', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Cookie: cookie },
      body: JSON.stringify({ name: 'Acme' }),
    });
    expect(res.status).toBe(201);
    const body = await res.json() as any;
    expect(body.workspace.slug).toBe('acme');
    expect(body.workspace.mcpEndpoint).toMatch(/\/mcp\/v1$/);
  });

  it('POST tokens mints a one-time token', async () => {
    const res = await buildApp().request('/api/workspaces/acme/tokens', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Cookie: cookie },
      body: JSON.stringify({ name: 'cli' }),
    });
    expect(res.status).toBe(201);
    const body = await res.json() as any;
    expect(body.token).toMatch(/^kyma_/);
    expect(body.mcpEndpoint).toMatch(/\/mcp\/v1$/);
  });
});
```

- [ ] **Step 4: Run + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/routes/workspaces.ts cloud/api/src/routes/workspaces.test.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): workspace + token HTTP routes"
```

---

### Task 14: Stripe service (pinned to 2024-11-20.acacia)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/stripe.service.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/stripe.service.test.ts`

- [ ] **Step 1: stripe.service.ts**

```ts
import Stripe from 'stripe';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import type { PlanId } from '@kyma/shared';
import { PLANS } from '@kyma/shared';

// Pinned per Slice 2 spec — bumping must be a deliberate code change. The
// stripe@17 SDK's LatestApiVersion is currently 2025-02-24.acacia, but Slice
// 2 pins to the version we tested against.
export const STRIPE_API_VERSION = '2024-11-20.acacia' as const;

let _client: Stripe | null = null;
export function getStripe(): Stripe {
  if (_client) return _client;
  const key = getEnv().STRIPE_SECRET_KEY;
  if (!key) throw new Error('STRIPE_SECRET_KEY not set');
  _client = new Stripe(key, { apiVersion: STRIPE_API_VERSION as any });
  return _client;
}

export function isStripeConfigured(): boolean {
  return Boolean(getEnv().STRIPE_SECRET_KEY);
}

export function getPriceIdForPlan(plan: PlanId): string | null {
  const env = getEnv();
  if (plan === 'pro')  return env.STRIPE_PRICE_PRO  ?? null;
  if (plan === 'team') return env.STRIPE_PRICE_TEAM ?? null;
  return null;
}

export function getPlanForPriceId(priceId: string): PlanId | null {
  const env = getEnv();
  if (env.STRIPE_PRICE_PRO  && priceId === env.STRIPE_PRICE_PRO)  return 'pro';
  if (env.STRIPE_PRICE_TEAM && priceId === env.STRIPE_PRICE_TEAM) return 'team';
  return null;
}

export async function findWorkspaceByCustomerId(customerId: string) {
  const db = getDb();
  const [row] = await db.select({ id: schema.workspaces.id })
    .from(schema.workspaces)
    .where(eq(schema.workspaces.stripeCustomerId, customerId))
    .limit(1);
  return row ?? null;
}

export async function getOrCreateStripeCustomer(workspace: {
  id: string; ownerUserId: string; stripeCustomerId: string | null;
}): Promise<Stripe.Customer> {
  const stripe = getStripe();
  const db = getDb();
  if (workspace.stripeCustomerId) {
    const c = await stripe.customers.retrieve(workspace.stripeCustomerId);
    if (!('deleted' in c) || !c.deleted) return c as Stripe.Customer;
  }
  const [owner] = await db.select({ email: schema.users.email })
    .from(schema.users).where(eq(schema.users.id, workspace.ownerUserId)).limit(1);
  const customer = await stripe.customers.create({
    email: owner?.email,
    metadata: { workspace_id: workspace.id, owner_email: owner?.email ?? '' },
  });
  await db.update(schema.workspaces)
    .set({ stripeCustomerId: customer.id, updatedAt: new Date() })
    .where(eq(schema.workspaces.id, workspace.id));
  return customer;
}

/** Idempotent — webhook may invoke multiple times for the same subscription. */
export async function applyWorkspaceSubscription(workspaceId: string, sub: Stripe.Subscription): Promise<void> {
  const db = getDb();
  const planItem = sub.items.data.find((it) => it.price?.recurring?.usage_type !== 'metered');
  const priceId = planItem?.price.id ?? null;
  const plan: PlanId = priceId ? (getPlanForPriceId(priceId) ?? 'free') : 'free';

  const status = sub.status;
  const planActive = status === 'active' || status === 'trialing';
  const trialEndsAt = sub.trial_end ? new Date(sub.trial_end * 1000) : null;

  const periodEndUnix =
    (planItem as unknown as { current_period_end?: number } | undefined)?.current_period_end
    ?? (sub as unknown as { current_period_end?: number }).current_period_end
    ?? null;
  const subscriptionPeriodEnd = periodEndUnix ? new Date(periodEndUnix * 1000) : null;

  await db.update(schema.workspaces).set({
    plan,
    planActive,
    stripeSubscriptionId: sub.id,
    trialEndsAt,
    subscriptionPeriodEnd,
    dunningState: status === 'past_due' || status === 'unpaid' ? 'past_due' : 'paid',
    updatedAt: new Date(),
  }).where(eq(schema.workspaces.id, workspaceId));
}

export async function downgradeWorkspaceToFree(workspaceId: string): Promise<void> {
  const db = getDb();
  await db.update(schema.workspaces).set({
    plan: 'free', planActive: true,
    stripeSubscriptionId: null, trialEndsAt: null, subscriptionPeriodEnd: null,
    dunningState: null, updatedAt: new Date(),
  }).where(eq(schema.workspaces.id, workspaceId));
}

export { PLANS };
```

- [ ] **Step 2: stripe.service.test.ts (config-only — no live HTTP)**

```ts
import { describe, it, expect, beforeAll } from 'vitest';
import { STRIPE_API_VERSION, getPriceIdForPlan, getPlanForPriceId, isStripeConfigured } from './stripe.service.js';

describe('stripe.service config', () => {
  beforeAll(() => {
    process.env.STRIPE_SECRET_KEY = '';
    process.env.STRIPE_PRICE_PRO = 'price_pro_x';
    process.env.STRIPE_PRICE_TEAM = 'price_team_x';
    process.env.SESSION_SECRET = 'a'.repeat(48);
  });

  it('pins API version to 2024-11-20.acacia', () => {
    expect(STRIPE_API_VERSION).toBe('2024-11-20.acacia');
  });
  it('isStripeConfigured returns false when secret missing', () => {
    expect(isStripeConfigured()).toBe(false);
  });
  it('plan ↔ price helpers round-trip', () => {
    expect(getPriceIdForPlan('pro')).toBe('price_pro_x');
    expect(getPlanForPriceId('price_pro_x')).toBe('pro');
    expect(getPlanForPriceId('unknown')).toBe(null);
  });
});
```

- [ ] **Step 3: Run + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/services/stripe.service.ts cloud/api/src/services/stripe.service.test.ts
git commit -m "feat(cloud/api): Stripe service pinned to 2024-11-20.acacia"
```

---

### Task 15: Stripe checkout, portal, subscription routes

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/billing.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`

- [ ] **Step 1: routes/billing.ts**

```ts
import { Hono } from 'hono';
import { z } from 'zod';
import { eq } from 'drizzle-orm';
import { sessionMiddleware } from '../middleware/session.js';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import { AppError, badRequest, forbidden, notFound } from '../lib/errors.js';
import * as ws from '../services/workspace.service.js';
import {
  getStripe, isStripeConfigured, getPriceIdForPlan, getOrCreateStripeCustomer,
} from '../services/stripe.service.js';

export const billingRoutes = new Hono();
billingRoutes.use('*', sessionMiddleware);

function billingDisabled() {
  return new AppError(503, 'BILLING_UNAVAILABLE', 'Stripe is not configured on this server');
}

const checkoutSchema = z.object({
  workspaceSlug: z.string(),
  plan: z.enum(['pro', 'team']),
  returnUrl: z.string().url().optional(),
});

billingRoutes.post('/checkout', async (c) => {
  if (!isStripeConfigured()) throw billingDisabled();
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = checkoutSchema.safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);

  const { workspace, role } = await ws.getBySlugForUser(u.userId, parsed.data.workspaceSlug);
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can change plan');
  const priceId = getPriceIdForPlan(parsed.data.plan);
  if (!priceId) throw badRequest(`Plan ${parsed.data.plan} not configured`, 'PLAN_UNCONFIGURED');

  const customer = await getOrCreateStripeCustomer({
    id: workspace.id,
    ownerUserId: workspace.ownerUserId,
    stripeCustomerId: workspace.stripeCustomerId,
  });

  const env = getEnv();
  const baseReturn = parsed.data.returnUrl ?? `${env.CLOUD_BASE_URL}/workspaces/${workspace.slug}/billing`;
  const cleanBase = baseReturn.split('?')[0];
  const stripe = getStripe();

  const session = await stripe.checkout.sessions.create({
    mode: 'subscription',
    customer: customer.id,
    client_reference_id: workspace.id,
    line_items: [{ price: priceId, quantity: 1 }],
    success_url: `${cleanBase}?status=success&session_id={CHECKOUT_SESSION_ID}`,
    cancel_url: `${cleanBase}?status=cancelled`,
    allow_promotion_codes: true,
    subscription_data: {
      metadata: { workspace_id: workspace.id },
      ...(workspace.stripeSubscriptionId ? {} : { trial_period_days: 14 }),
    },
  });
  return c.json({ url: session.url });
});

billingRoutes.post('/portal', async (c) => {
  if (!isStripeConfigured()) throw billingDisabled();
  const u = c.get('user');
  const body = await c.req.json();
  const parsed = z.object({
    workspaceSlug: z.string(),
    returnUrl: z.string().url().optional(),
  }).safeParse(body);
  if (!parsed.success) throw badRequest(parsed.error.issues[0].message);
  const { workspace, role } = await ws.getBySlugForUser(u.userId, parsed.data.workspaceSlug);
  if (!['owner', 'admin'].includes(role)) throw forbidden('Only owner or admin can open billing portal');
  if (!workspace.stripeCustomerId) throw badRequest('No customer yet — start a checkout first', 'NO_CUSTOMER');

  const env = getEnv();
  const session = await getStripe().billingPortal.sessions.create({
    customer: workspace.stripeCustomerId,
    return_url: parsed.data.returnUrl ?? `${env.CLOUD_BASE_URL}/workspaces/${workspace.slug}/billing`,
  });
  return c.json({ url: session.url });
});

billingRoutes.get('/:slug/subscription', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  return c.json({
    plan: workspace.plan,
    planActive: workspace.planActive,
    trialEndsAt: workspace.trialEndsAt,
    currentPeriodEnd: workspace.subscriptionPeriodEnd,
    stripeCustomerId: workspace.stripeCustomerId,
    stripeSubscriptionId: workspace.stripeSubscriptionId,
    dunningState: workspace.dunningState,
  });
});
```

- [ ] **Step 2: Mount BEFORE the webhook in index.ts**

The webhook is mounted in Task 16 with raw-body before any auth. Billing is auth-gated. Mount order in `index.ts`:

```ts
import { billingRoutes } from './routes/billing.js';
// ...
// (webhook mounted in Task 16 — keep above this line)
app.route('/api/billing', billingRoutes);
```

- [ ] **Step 3: Commit**

```bash
git add cloud/api/src/routes/billing.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): Stripe checkout, portal, subscription summary"
```

---

### Task 16: Stripe webhook (signature-verified, log-then-reconcile)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/webhooks.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`

Mounting order matters: the webhook reads raw bytes, so it MUST be mounted BEFORE any `app.use('*', ...)` JSON-parsing middleware and BEFORE auth-gated routes. Hono's `c.req.raw.text()` works regardless, but cors+logger are fine to keep above; we just must NOT JSON-parse before signature verification.

- [ ] **Step 1: routes/webhooks.ts**

```ts
import { Hono } from 'hono';
import type Stripe from 'stripe';
import { eq } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getEnv } from '../env.js';
import {
  getStripe, isStripeConfigured, applyWorkspaceSubscription,
  downgradeWorkspaceToFree, findWorkspaceByCustomerId,
} from '../services/stripe.service.js';

export const stripeWebhookRoutes = new Hono();

stripeWebhookRoutes.post('/stripe', async (c) => {
  const env = getEnv();
  if (!env.STRIPE_WEBHOOK_SIGNING_SECRET) {
    return c.json({ error: { code: 'WEBHOOK_DISABLED' } }, 503);
  }
  if (!isStripeConfigured()) {
    return c.json({ error: { code: 'BILLING_UNAVAILABLE' } }, 503);
  }
  const sig = c.req.header('stripe-signature');
  if (!sig) return c.json({ error: { code: 'NO_SIGNATURE' } }, 400);

  const raw = await c.req.raw.text();
  let event: Stripe.Event;
  try {
    event = getStripe().webhooks.constructEvent(raw, sig, env.STRIPE_WEBHOOK_SIGNING_SECRET);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error('[stripe.webhook] sig verify failed:', msg);
    return c.json({ error: { code: 'INVALID_SIGNATURE', message: msg } }, 400);
  }

  const db = getDb();
  const inserted = await db.insert(schema.billingEvents).values({
    stripeEventId: event.id,
    eventType: event.type,
    payload: event as unknown as Record<string, unknown>,
    processed: false,
  }).onConflictDoNothing({ target: schema.billingEvents.stripeEventId })
    .returning({ id: schema.billingEvents.id });

  if (inserted.length === 0) {
    return c.json({ received: true, duplicate: true });
  }

  try {
    await dispatch(event);
    await db.update(schema.billingEvents)
      .set({ processed: true })
      .where(eq(schema.billingEvents.stripeEventId, event.id));
  } catch (err) {
    console.error(`[stripe.webhook] handler ${event.type} failed:`, err);
    // Intentionally still 200 — the row stays processed=false in billing_events
    // and the hourly reconciler picks it up. Returning 5xx triggers Stripe's
    // 3-day retry storm.
  }
  return c.json({ received: true });
});

async function dispatch(event: Stripe.Event): Promise<void> {
  const db = getDb();
  const tagWs = (eventId: string, workspaceId: string) =>
    db.update(schema.billingEvents).set({ workspaceId }).where(eq(schema.billingEvents.stripeEventId, eventId));

  switch (event.type) {
    case 'customer.subscription.created':
    case 'customer.subscription.updated': {
      const sub = event.data.object as Stripe.Subscription;
      const customerId = typeof sub.customer === 'string' ? sub.customer : sub.customer.id;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) { console.warn(`[stripe.webhook] ${event.type} for unknown customer ${customerId}`); return; }
      const metaWsId = sub.metadata?.workspace_id;
      if (metaWsId && metaWsId !== ws.id) {
        console.error(`[stripe.webhook] metadata.workspace_id=${metaWsId} mismatches customer's workspace=${ws.id}; refusing`);
        return;
      }
      await applyWorkspaceSubscription(ws.id, sub);
      await tagWs(event.id, ws.id);
      return;
    }
    case 'customer.subscription.deleted': {
      const sub = event.data.object as Stripe.Subscription;
      const customerId = typeof sub.customer === 'string' ? sub.customer : sub.customer.id;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await downgradeWorkspaceToFree(ws.id);
      await tagWs(event.id, ws.id);
      return;
    }
    case 'invoice.payment_failed': {
      const inv = event.data.object as Stripe.Invoice;
      const customerId = typeof inv.customer === 'string' ? inv.customer : inv.customer?.id;
      if (!customerId) return;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await db.update(schema.workspaces).set({
        planActive: false, dunningState: 'failed', updatedAt: new Date(),
      }).where(eq(schema.workspaces.id, ws.id));
      await tagWs(event.id, ws.id);
      return;
    }
    case 'invoice.paid': {
      const inv = event.data.object as Stripe.Invoice;
      const customerId = typeof inv.customer === 'string' ? inv.customer : inv.customer?.id;
      if (!customerId) return;
      const ws = await findWorkspaceByCustomerId(customerId);
      if (!ws) return;
      await db.update(schema.workspaces).set({
        planActive: true, dunningState: 'paid', updatedAt: new Date(),
      }).where(eq(schema.workspaces.id, ws.id));
      await tagWs(event.id, ws.id);
      return;
    }
  }
}
```

- [ ] **Step 2: Mount webhook BEFORE any auth-gated routes in index.ts**

```ts
import { stripeWebhookRoutes } from './routes/webhooks.js';
// (right after the cors block, before app.route('/api/auth', ...) etc)
app.route('/api/webhooks', stripeWebhookRoutes);
```

- [ ] **Step 3: Commit**

```bash
git add cloud/api/src/routes/webhooks.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): Stripe webhook with sig verify + idempotent log-then-reconcile"
```

---

### Task 17: Hourly Stripe drop-detection reconciler

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/billing-reconciler.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/billing-reconciler.test.ts`

The hourly job lists active customers' subscriptions and re-runs `applyWorkspaceSubscription` so any dropped/lost webhook is healed.

- [ ] **Step 1: billing-reconciler.ts**

```ts
import { isNotNull } from 'drizzle-orm';
import { getDb, schema } from '../db/client.js';
import { getStripe, isStripeConfigured, applyWorkspaceSubscription, downgradeWorkspaceToFree } from './stripe.service.js';

export async function reconcileOnce(): Promise<{ scanned: number; updated: number }> {
  if (!isStripeConfigured()) return { scanned: 0, updated: 0 };
  const db = getDb();
  const stripe = getStripe();
  const wss = await db.select({
    id: schema.workspaces.id,
    customerId: schema.workspaces.stripeCustomerId,
    subscriptionId: schema.workspaces.stripeSubscriptionId,
  }).from(schema.workspaces).where(isNotNull(schema.workspaces.stripeCustomerId));

  let updated = 0;
  for (const ws of wss) {
    if (!ws.customerId) continue;
    try {
      const subs = await stripe.subscriptions.list({ customer: ws.customerId, status: 'all', limit: 5 });
      const active = subs.data.find(s => s.status === 'active' || s.status === 'trialing');
      if (active) {
        await applyWorkspaceSubscription(ws.id, active);
        updated += 1;
      } else if (ws.subscriptionId) {
        await downgradeWorkspaceToFree(ws.id);
        updated += 1;
      }
    } catch (err) {
      console.error(`[billing.reconcile] ws=${ws.id} failed:`, err);
    }
  }
  return { scanned: wss.length, updated };
}

export function startReconciler(intervalMs = 60 * 60 * 1000): () => void {
  const tick = () => {
    reconcileOnce()
      .then((r) => console.log(`[billing.reconcile] scanned=${r.scanned} updated=${r.updated}`))
      .catch((err) => console.error('[billing.reconcile] failed:', err));
  };
  tick();
  const handle = setInterval(tick, intervalMs);
  return () => clearInterval(handle);
}
```

- [ ] **Step 2: Wire into index.ts on serve()**

In the `if (import.meta.url === ...)` block, after `serve(...)`:

```ts
import { startReconciler } from './services/billing-reconciler.js';
// ...
serve({ fetch: app.fetch, port: env.PORT }, () => {
  if (env.STRIPE_SECRET_KEY) startReconciler();
});
```

- [ ] **Step 3: Reconciler test (mocks the Stripe client via service injection)**

Add a tiny module-mock in `billing-reconciler.test.ts`:

```ts
import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getDb, schema } from '../db/client.js';

vi.mock('./stripe.service.js', async (importOriginal) => {
  const actual = await importOriginal() as typeof import('./stripe.service.js');
  return {
    ...actual,
    isStripeConfigured: () => true,
    getStripe: () => ({
      subscriptions: { list: async (_args: any) => ({ data: [] }) },
    }) as any,
  };
});

import { reconcileOnce } from './billing-reconciler.js';

beforeAll(async () => { process.env.SESSION_SECRET = 'a'.repeat(48); await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('billing-reconciler', () => {
  it('scans workspaces with stripeCustomerId; downgrades when no active sub', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    await db.insert(schema.workspaces).values({
      slug: 'a', name: 'A', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
      stripeCustomerId: 'cus_123', stripeSubscriptionId: 'sub_existing', plan: 'pro',
    });
    const r = await reconcileOnce();
    expect(r.scanned).toBe(1);
    expect(r.updated).toBe(1);
    const [ws] = await db.select().from(schema.workspaces);
    expect(ws.plan).toBe('free');
    expect(ws.stripeSubscriptionId).toBeNull();
  });
});
```

- [ ] **Step 4: Run + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/services/billing-reconciler.ts cloud/api/src/services/billing-reconciler.test.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): hourly Stripe drop-detection reconciler"
```

---

### Task 18: Usage rollup endpoint (read-only in Slice 2)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/usage.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/index.ts`

Slice 2 has no usage producers wired yet — the engine will start emitting `usage_events` rows via a separate reporting endpoint in Slice 2.5. For now, the dashboard reads from `usage_daily`, which is empty until producers exist. We ship the read endpoint so the chart slot in the UI returns `[]` cleanly.

- [ ] **Step 1: routes/usage.ts**

```ts
import { Hono } from 'hono';
import { and, eq, gte } from 'drizzle-orm';
import { sessionMiddleware } from '../middleware/session.js';
import { getDb, schema } from '../db/client.js';
import * as ws from '../services/workspace.service.js';

export const usageRoutes = new Hono();
usageRoutes.use('*', sessionMiddleware);

usageRoutes.get('/:slug/daily', async (c) => {
  const u = c.get('user');
  const { workspace } = await ws.getBySlugForUser(u.userId, c.req.param('slug'));
  const since = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000);
  const rows = await getDb().select().from(schema.usageDaily)
    .where(and(eq(schema.usageDaily.workspaceId, workspace.id), gte(schema.usageDaily.day, since)));
  return c.json({ usage: rows });
});
```

- [ ] **Step 2: Mount + commit**

```ts
import { usageRoutes } from './routes/usage.js';
app.route('/api/usage', usageRoutes);
```

```bash
git add cloud/api/src/routes/usage.ts cloud/api/src/index.ts
git commit -m "feat(cloud/api): /api/usage/:slug/daily read endpoint"
```

---

### Task 19: Customer web — Next.js scaffold + global tokens

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/package.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/tsconfig.json`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/next.config.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/postcss.config.mjs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/layout.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/globals.css`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/page.tsx`

- [ ] **Step 1: package.json**

```json
{
  "name": "@kyma/cloud-web",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "next dev --port 3001",
    "build": "next build",
    "start": "next start --port 3001"
  },
  "dependencies": {
    "@kyma/shared": "workspace:*",
    "clsx": "^2.1.0",
    "lucide-react": "^0.460.0",
    "motion": "^12.0.0",
    "next": "^15.3.0",
    "react": "^19.1.0",
    "react-dom": "^19.1.0",
    "recharts": "^3.0.0",
    "tailwind-merge": "^3.0.0",
    "zustand": "^5.0.0"
  },
  "devDependencies": {
    "@tailwindcss/postcss": "^4.1.0",
    "@types/node": "^22.0.0",
    "@types/react": "^19.1.0",
    "@types/react-dom": "^19.1.0",
    "postcss": "^8.5.0",
    "tailwindcss": "^4.1.0",
    "typescript": "^5.7.0"
  }
}
```

- [ ] **Step 2: tsconfig.json + next.config.ts + postcss.config.mjs**

```json
{
  "compilerOptions": {
    "target": "ES2022", "module": "ESNext", "lib": ["DOM", "DOM.Iterable", "ES2022"],
    "moduleResolution": "Bundler", "jsx": "preserve", "strict": true,
    "esModuleInterop": true, "skipLibCheck": true, "resolveJsonModule": true,
    "incremental": true, "noEmit": true,
    "paths": { "@/*": ["./src/*"] },
    "plugins": [{ "name": "next" }]
  },
  "include": ["next-env.d.ts", "src/**/*", ".next/types/**/*.ts"],
  "exclude": ["node_modules"]
}
```

```ts
// next.config.ts
import type { NextConfig } from 'next';
const config: NextConfig = {
  output: 'standalone',
  transpilePackages: ['@kyma/shared'],
  experimental: { externalDir: true },
};
export default config;
```

```js
// postcss.config.mjs
export default { plugins: { '@tailwindcss/postcss': {} } };
```

- [ ] **Step 3: globals.css with kyma tokens (matches docs site)**

```css
@import "tailwindcss";

@layer base {
  :root {
    --kyma-font-mono: 'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    --kyma-font-body: 'IBM Plex Sans', system-ui, -apple-system, sans-serif;
    --kyma-bg: #ffffff;
    --kyma-bg-soft: #f7f7f6;
    --kyma-fg: #14171c;
    --kyma-fg-soft: #51555c;
    --kyma-muted: #767880;
    --kyma-rule: #2a2e36;
    --kyma-rule-soft: rgba(20, 23, 28, 0.10);
    --kyma-accent: #2d6f1f;
    --kyma-link: #2a5085;
  }
  .dark {
    --kyma-bg: #0d1015; --kyma-bg-soft: #14181f; --kyma-fg: #e8e6e0;
    --kyma-fg-soft: #b3b0a8; --kyma-muted: #818488; --kyma-rule: #2a2e36;
    --kyma-rule-soft: rgba(232, 230, 224, 0.10);
    --kyma-accent: #7ed957; --kyma-link: #7eaae6;
  }
  body {
    background: var(--kyma-bg); color: var(--kyma-fg);
    font-family: var(--kyma-font-body);
  }
  h1, h2, h3, h4 { font-family: var(--kyma-font-mono); letter-spacing: -0.005em; font-weight: 600; }
  code, pre, kbd { font-family: var(--kyma-font-mono); }
}
```

- [ ] **Step 4: layout.tsx + page.tsx**

```tsx
// src/app/layout.tsx
import './globals.css';
import type { Metadata } from 'next';

export const metadata: Metadata = { title: 'kyma cloud', description: 'Your data, queryable by any agent.' };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="" />
        <link
          href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600;700&family=IBM+Plex+Sans:wght@400;500;600;700&display=swap"
          rel="stylesheet"
        />
      </head>
      <body>{children}</body>
    </html>
  );
}
```

```tsx
// src/app/page.tsx
import { redirect } from 'next/navigation';
import { getCurrentUser } from '@/lib/auth-server';

export default async function Home() {
  const user = await getCurrentUser();
  redirect(user ? '/workspaces' : '/login');
}
```

- [ ] **Step 5: lib/auth-server.ts**

```ts
import 'server-only';
import { cookies } from 'next/headers';
import * as jose from 'jose';

const SESSION_COOKIE = 'kyma_session';
const ISS = 'kyma-cloud';

export async function getCurrentUser(): Promise<{ id: string; email: string } | null> {
  const c = await cookies();
  const jwt = c.get(SESSION_COOKIE)?.value;
  if (!jwt) return null;
  try {
    const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
    const { payload } = await jose.jwtVerify(jwt, secret, { issuer: ISS });
    return { id: payload.sub as string, email: payload.email as string };
  } catch { return null; }
}
```

- [ ] **Step 6: Dev test**

```bash
cd /Users/shaked/projects_new/agentcy/kyma/cloud
pnpm install
pnpm --filter @kyma/cloud-web dev &
curl -i http://localhost:3001/   # expect 307 Location: /login
kill %1
```

- [ ] **Step 7: Commit**

```bash
git add cloud/web
git commit -m "feat(cloud/web): Next.js 15 scaffold with kyma design tokens"
```

---

### Task 20: Login page (GitHub button + magic-link form + callback handler)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/lib/api.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/login/page.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/login/callback/page.tsx`

- [ ] **Step 1: lib/api.ts (browser-side)**

```ts
const API_URL = process.env.NEXT_PUBLIC_CLOUD_BASE_URL ?? '';

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_URL}${path}`, {
    ...init,
    credentials: 'include',
    headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
  });
  if (!res.ok) {
    const e = await res.json().catch(() => ({ error: { message: `HTTP ${res.status}` } }));
    throw new Error(e.error?.message ?? `HTTP ${res.status}`);
  }
  return res.json();
}

export const api = {
  startGithubAuth: () => { window.location.href = `${API_URL}/api/auth/github/start`; },
  requestMagicLink: (email: string) => call<{ ok: true }>('/api/auth/magic-link/request', {
    method: 'POST', body: JSON.stringify({ email }),
  }),
  exchangeMagicLink: (token: string) =>
    call<{ user: { id: string; email: string; name: string | null } }>(
      '/api/auth/magic-link/exchange',
      { method: 'POST', body: JSON.stringify({ token }) },
    ),
  me: () => call<{ user: { id: string; email: string; name: string | null } }>('/api/auth/me'),
  workspaces: {
    list: () => call<{ workspaces: any[] }>('/api/workspaces'),
    create: (name: string) => call<{ workspace: any }>('/api/workspaces', {
      method: 'POST', body: JSON.stringify({ name }),
    }),
    get: (slug: string) => call<{ workspace: any; role: string }>(`/api/workspaces/${slug}`),
    listTokens: (slug: string) => call<{ tokens: any[] }>(`/api/workspaces/${slug}/tokens`),
    mintToken: (slug: string, name?: string) => call<{
      token: string; prefix: string; id: string; mcpEndpoint: string;
    }>(`/api/workspaces/${slug}/tokens`, { method: 'POST', body: JSON.stringify({ name }) }),
    revokeToken: (slug: string, id: string) => call<{ ok: true }>(
      `/api/workspaces/${slug}/tokens/${id}/revoke`, { method: 'POST' },
    ),
  },
  billing: {
    checkout: (workspaceSlug: string, plan: 'pro' | 'team') => call<{ url: string }>(
      '/api/billing/checkout',
      { method: 'POST', body: JSON.stringify({ workspaceSlug, plan }) },
    ),
    portal: (workspaceSlug: string) => call<{ url: string }>(
      '/api/billing/portal', { method: 'POST', body: JSON.stringify({ workspaceSlug }) },
    ),
    subscription: (slug: string) => call<{
      plan: string; planActive: boolean; trialEndsAt: string | null;
      currentPeriodEnd: string | null; dunningState: string | null;
    }>(`/api/billing/${slug}/subscription`),
  },
};
```

- [ ] **Step 2: app/login/page.tsx**

```tsx
'use client';
import { useState } from 'react';
import { api } from '@/lib/api';

export default function LoginPage() {
  const [email, setEmail] = useState('');
  const [sent, setSent] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null); setPending(true);
    try { await api.requestMagicLink(email); setSent(true); }
    catch (err: any) { setError(err.message); }
    finally { setPending(false); }
  }

  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div className="w-full max-w-sm space-y-6">
        <div>
          <h1 className="text-2xl">kyma cloud</h1>
          <p className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
            Sign in to your workspace.
          </p>
        </div>

        <button
          onClick={() => api.startGithubAuth()}
          className="w-full h-10 rounded border font-medium"
          style={{ borderColor: 'var(--kyma-rule-soft)', background: 'var(--kyma-bg-soft)' }}
        >
          Continue with GitHub
        </button>

        <div className="flex items-center gap-3 text-xs" style={{ color: 'var(--kyma-muted)' }}>
          <div className="flex-1 h-px" style={{ background: 'var(--kyma-rule-soft)' }} />
          OR
          <div className="flex-1 h-px" style={{ background: 'var(--kyma-rule-soft)' }} />
        </div>

        {sent ? (
          <div className="text-sm">Check your inbox — we sent a magic link to <strong>{email}</strong>.</div>
        ) : (
          <form onSubmit={submit} className="space-y-3">
            <input
              type="email" required value={email} onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              className="w-full h-10 px-3 rounded border bg-transparent"
              style={{ borderColor: 'var(--kyma-rule-soft)' }}
            />
            <button
              type="submit" disabled={pending}
              className="w-full h-10 rounded font-medium text-white"
              style={{ background: 'var(--kyma-accent)' }}
            >
              {pending ? 'Sending…' : 'Email me a link'}
            </button>
            {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
          </form>
        )}
      </div>
    </main>
  );
}
```

- [ ] **Step 3: app/login/callback/page.tsx**

```tsx
'use client';
import { useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { api } from '@/lib/api';

export default function CallbackPage() {
  const params = useSearchParams();
  const router = useRouter();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const token = params.get('token');
    if (!token) { setError('Missing token'); return; }
    api.exchangeMagicLink(token)
      .then(() => router.replace('/workspaces'))
      .catch((e) => setError(e.message));
  }, [params, router]);

  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div>
        {error
          ? <div className="text-sm" style={{ color: '#dc2626' }}>Sign-in failed: {error}</div>
          : <div className="text-sm">Signing you in…</div>}
      </div>
    </main>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add cloud/web/src/lib cloud/web/src/app/login
git commit -m "feat(cloud/web): login page + GitHub start + magic-link form + callback exchange"
```

---

### Task 21: Workspace list + create page

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/(dashboard)/layout.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/(dashboard)/workspaces/page.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/(dashboard)/workspaces/new/page.tsx`

- [ ] **Step 1: dashboard layout (gated server-side)**

```tsx
// src/app/(dashboard)/layout.tsx
import { redirect } from 'next/navigation';
import { getCurrentUser } from '@/lib/auth-server';
import Link from 'next/link';

export default async function DashboardLayout({ children }: { children: React.ReactNode }) {
  const user = await getCurrentUser();
  if (!user) redirect('/login');
  return (
    <div className="min-h-screen">
      <header
        className="px-6 h-14 flex items-center justify-between border-b"
        style={{ borderColor: 'var(--kyma-rule-soft)' }}
      >
        <Link href="/workspaces" className="font-mono font-semibold">kyma cloud</Link>
        <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>{user.email}</div>
      </header>
      <main className="px-6 py-8 max-w-5xl mx-auto">{children}</main>
    </div>
  );
}
```

- [ ] **Step 2: workspaces/page.tsx**

```tsx
'use client';
import { useEffect, useState } from 'react';
import Link from 'next/link';
import { api } from '@/lib/api';

export default function WorkspacesPage() {
  const [list, setList] = useState<any[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => { api.workspaces.list().then((r) => setList(r.workspaces)).catch((e) => setError(e.message)); }, []);
  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!list) return <div>Loading…</div>;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-xl">Workspaces</h1>
        <Link
          href="/workspaces/new"
          className="h-9 px-3 rounded text-white text-sm flex items-center"
          style={{ background: 'var(--kyma-accent)' }}
        >
          New workspace
        </Link>
      </div>
      {list.length === 0 ? (
        <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>
          No workspaces yet. Create your first.
        </div>
      ) : (
        <ul className="space-y-2">
          {list.map((w) => (
            <li key={w.id} className="border rounded p-4" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
              <Link href={`/workspaces/${w.slug}`} className="font-medium">{w.name}</Link>
              <div className="text-xs mt-1" style={{ color: 'var(--kyma-muted)' }}>
                {w.slug} · {w.kind} · {w.plan}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
```

- [ ] **Step 3: workspaces/new/page.tsx**

```tsx
'use client';
import { useState } from 'react';
import { useRouter } from 'next/navigation';
import { api } from '@/lib/api';

export default function NewWorkspacePage() {
  const router = useRouter();
  const [name, setName] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setPending(true); setError(null);
    try {
      const { workspace } = await api.workspaces.create(name);
      router.replace(`/workspaces/${workspace.slug}`);
    } catch (err: any) { setError(err.message); setPending(false); }
  }

  return (
    <form onSubmit={submit} className="max-w-md space-y-4">
      <h1 className="text-xl">New workspace</h1>
      <input
        required value={name} onChange={(e) => setName(e.target.value)}
        placeholder="My workspace"
        className="w-full h-10 px-3 rounded border bg-transparent"
        style={{ borderColor: 'var(--kyma-rule-soft)' }}
      />
      <button
        type="submit" disabled={pending}
        className="h-10 px-4 rounded text-white"
        style={{ background: 'var(--kyma-accent)' }}
      >
        {pending ? 'Creating…' : 'Create'}
      </button>
      {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
    </form>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add cloud/web/src/app/\(dashboard\)
git commit -m "feat(cloud/web): workspace list + create pages"
```

---

### Task 22: Workspace detail page with MCP install widget

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/(dashboard)/workspaces/[slug]/page.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/components/mcp-install-widget.tsx`

- [ ] **Step 1: components/mcp-install-widget.tsx**

```tsx
'use client';
import { useState } from 'react';
import { api } from '@/lib/api';

export function McpInstallWidget({ slug, mcpEndpoint }: { slug: string; mcpEndpoint: string }) {
  const [token, setToken] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function mint() {
    setPending(true); setError(null);
    try { setToken((await api.workspaces.mintToken(slug, 'claude-skill')).token); }
    catch (err: any) { setError(err.message); }
    finally { setPending(false); }
  }

  return (
    <section className="border rounded p-4 space-y-3" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
      <h2 className="text-base">Connect Claude</h2>
      <ol className="list-decimal pl-5 text-sm space-y-2" style={{ color: 'var(--kyma-fg-soft)' }}>
        <li>Install the kyma Claude skill: <code>/skill install kyma</code> in Claude.</li>
        <li>Mint a workspace token below — copy it once; we never show it again.</li>
        <li>Paste the URL and token into the skill prompt and start asking questions.</li>
      </ol>

      <div className="space-y-2">
        <label className="text-xs uppercase tracking-wide" style={{ color: 'var(--kyma-muted)' }}>
          MCP endpoint
        </label>
        <input
          readOnly value={mcpEndpoint}
          onClick={(e) => (e.target as HTMLInputElement).select()}
          className="w-full h-10 px-3 rounded border font-mono text-sm bg-transparent"
          style={{ borderColor: 'var(--kyma-rule-soft)' }}
        />
      </div>

      {token ? (
        <div className="space-y-2">
          <label className="text-xs uppercase tracking-wide" style={{ color: 'var(--kyma-muted)' }}>
            Token (copy now — won't be shown again)
          </label>
          <input
            readOnly value={token}
            onClick={(e) => (e.target as HTMLInputElement).select()}
            className="w-full h-10 px-3 rounded border font-mono text-sm bg-transparent"
            style={{ borderColor: 'var(--kyma-rule-soft)' }}
          />
          <pre
            className="text-xs p-3 rounded overflow-x-auto"
            style={{ background: 'var(--kyma-bg-soft)' }}
          >{`{
  "mcpServers": {
    "kyma": {
      "transport": "http",
      "url": "${mcpEndpoint}",
      "headers": { "Authorization": "Bearer ${token}" }
    }
  }
}`}</pre>
        </div>
      ) : (
        <button
          onClick={mint} disabled={pending}
          className="h-10 px-4 rounded text-white text-sm"
          style={{ background: 'var(--kyma-accent)' }}
        >
          {pending ? 'Minting…' : 'Mint MCP token'}
        </button>
      )}
      {error && <div className="text-sm" style={{ color: '#dc2626' }}>{error}</div>}
    </section>
  );
}
```

- [ ] **Step 2: workspaces/[slug]/page.tsx**

```tsx
'use client';
import { useEffect, useState } from 'react';
import Link from 'next/link';
import { api } from '@/lib/api';
import { McpInstallWidget } from '@/components/mcp-install-widget';

export default function WorkspaceDetailPage({ params }: { params: { slug: string } }) {
  const [workspace, setWorkspace] = useState<any>(null);
  const [tokens, setTokens] = useState<any[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      api.workspaces.get(params.slug),
      api.workspaces.listTokens(params.slug),
    ]).then(([{ workspace }, { tokens }]) => { setWorkspace(workspace); setTokens(tokens); })
      .catch((e) => setError(e.message));
  }, [params.slug]);

  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!workspace) return <div>Loading…</div>;

  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <h1 className="text-xl">{workspace.name}</h1>
        <Link href={`/workspaces/${params.slug}/billing`} className="text-sm underline">Billing</Link>
      </div>

      <McpInstallWidget slug={params.slug} mcpEndpoint={workspace.mcpEndpoint} />

      <section className="space-y-3">
        <h2 className="text-base">Tokens</h2>
        {tokens.length === 0 ? (
          <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>No tokens yet.</div>
        ) : (
          <ul className="space-y-2">
            {tokens.map((t) => (
              <li key={t.id} className="border rounded p-3 text-sm flex items-center justify-between"
                  style={{ borderColor: 'var(--kyma-rule-soft)' }}>
                <span>
                  <code className="font-mono">{t.prefix}…</code>{' '}
                  <span style={{ color: 'var(--kyma-muted)' }}>· {t.scopes}</span>
                </span>
                {t.revokedAt
                  ? <span style={{ color: 'var(--kyma-muted)' }}>revoked</span>
                  : <button
                      onClick={async () => { await api.workspaces.revokeToken(params.slug, t.id); location.reload(); }}
                      className="text-xs underline" style={{ color: '#dc2626' }}
                    >revoke</button>}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add cloud/web/src/components cloud/web/src/app/\(dashboard\)/workspaces/\[slug\]
git commit -m "feat(cloud/web): workspace detail with MCP install widget + token list"
```

---

### Task 23: Billing page (plan selector + portal redirect + usage chart)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/app/(dashboard)/workspaces/[slug]/billing/page.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/src/components/usage-chart.tsx`

- [ ] **Step 1: components/usage-chart.tsx**

```tsx
'use client';
import { ResponsiveContainer, AreaChart, Area, XAxis, YAxis, Tooltip } from 'recharts';

interface DailyRow { day: string; kind: string; total: number; }

export function UsageChart({ rows }: { rows: DailyRow[] }) {
  const byDay = new Map<string, number>();
  for (const r of rows) byDay.set(r.day, (byDay.get(r.day) ?? 0) + r.total);
  const data = Array.from(byDay.entries()).map(([day, total]) => ({ day, total }))
    .sort((a, b) => a.day.localeCompare(b.day));
  if (!data.length) return <div className="text-sm" style={{ color: 'var(--kyma-muted)' }}>No usage yet.</div>;
  return (
    <div style={{ width: '100%', height: 200 }}>
      <ResponsiveContainer>
        <AreaChart data={data}>
          <XAxis dataKey="day" hide />
          <YAxis hide />
          <Tooltip />
          <Area dataKey="total" stroke="var(--kyma-accent)" fill="var(--kyma-accent)" fillOpacity={0.2} />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}
```

- [ ] **Step 2: billing/page.tsx**

```tsx
'use client';
import { useEffect, useState } from 'react';
import { api } from '@/lib/api';
import { PLANS, type PlanId } from '@kyma/shared';
import { UsageChart } from '@/components/usage-chart';

export default function BillingPage({ params }: { params: { slug: string } }) {
  const [sub, setSub] = useState<any>(null);
  const [usage, setUsage] = useState<any[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      api.billing.subscription(params.slug),
      fetch(`${process.env.NEXT_PUBLIC_CLOUD_BASE_URL}/api/usage/${params.slug}/daily`, { credentials: 'include' })
        .then(r => r.json() as Promise<{ usage: any[] }>),
    ]).then(([s, u]) => { setSub(s); setUsage(u.usage); }).catch((e) => setError(e.message));
  }, [params.slug]);

  async function checkout(plan: 'pro' | 'team') {
    setPending(true);
    try { window.location.href = (await api.billing.checkout(params.slug, plan)).url; }
    catch (err: any) { setError(err.message); setPending(false); }
  }
  async function portal() {
    setPending(true);
    try { window.location.href = (await api.billing.portal(params.slug)).url; }
    catch (err: any) { setError(err.message); setPending(false); }
  }

  if (error) return <div style={{ color: '#dc2626' }}>{error}</div>;
  if (!sub) return <div>Loading…</div>;

  return (
    <div className="space-y-8">
      <h1 className="text-xl">Billing</h1>

      <section className="border rounded p-4 space-y-2" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
        <div className="text-sm">Current plan: <strong>{sub.plan}</strong> · {sub.planActive ? 'active' : 'inactive'}</div>
        {sub.trialEndsAt && <div className="text-xs" style={{ color: 'var(--kyma-muted)' }}>Trial ends {sub.trialEndsAt}</div>}
        {sub.dunningState && sub.dunningState !== 'paid' && (
          <div className="text-xs" style={{ color: '#dc2626' }}>Payment issue: {sub.dunningState}</div>
        )}
        {sub.stripeCustomerId && (
          <button onClick={portal} disabled={pending} className="h-9 px-3 rounded text-sm border"
                  style={{ borderColor: 'var(--kyma-rule-soft)' }}>
            Manage billing
          </button>
        )}
      </section>

      <section className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {(['free', 'pro', 'team'] as PlanId[]).map((p) => (
          <div key={p} className="border rounded p-4" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
            <div className="font-mono text-sm uppercase tracking-wider" style={{ color: 'var(--kyma-muted)' }}>{p}</div>
            <div className="text-2xl my-2">${PLANS[p].pricePerMonth}/mo</div>
            <ul className="text-sm space-y-1 mb-3">
              {PLANS[p].features.map((f) => <li key={f}>· {f}</li>)}
            </ul>
            {p !== 'free' && (
              <button
                onClick={() => checkout(p as 'pro' | 'team')} disabled={pending}
                className="h-9 px-3 rounded text-sm text-white"
                style={{ background: 'var(--kyma-accent)' }}
              >
                {sub.plan === p ? 'Current' : `Switch to ${p}`}
              </button>
            )}
          </div>
        ))}
      </section>

      <section className="space-y-2">
        <h2 className="text-base">Usage (last 30 days)</h2>
        <UsageChart rows={usage} />
      </section>
    </div>
  );
}
```

- [ ] **Step 3: Commit**

```bash
git add cloud/web/src/app/\(dashboard\)/workspaces/\[slug\]/billing cloud/web/src/components/usage-chart.tsx
git commit -m "feat(cloud/web): billing page with checkout, portal, plan selector, usage chart"
```

---

### Task 24: Admin shell — Next.js scaffold with GitHub allowlist

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/package.json`, `tsconfig.json`, `next.config.ts`, `postcss.config.mjs`.
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/app/layout.tsx`, `src/app/globals.css`.
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/app/page.tsx` (login).
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/app/api/auth/github/start/route.ts`, `.../callback/route.ts`.
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/lib/admin-session.ts`.
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/lib/db.ts`.

- [ ] **Step 1: admin/package.json**

```json
{
  "name": "@kyma/cloud-admin",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "dev": "next dev --port 3002",
    "build": "next build",
    "start": "next start --port 3002"
  },
  "dependencies": {
    "@kyma/shared": "workspace:*",
    "drizzle-orm": "^0.38.0", "pg": "^8.13.0",
    "jose": "^6.0.0", "next": "^15.3.0",
    "react": "^19.1.0", "react-dom": "^19.1.0"
  },
  "devDependencies": {
    "@tailwindcss/postcss": "^4.1.0", "@types/node": "^22.0.0",
    "@types/pg": "^8.11.0", "@types/react": "^19.1.0",
    "@types/react-dom": "^19.1.0", "postcss": "^8.5.0",
    "tailwindcss": "^4.1.0", "typescript": "^5.7.0"
  }
}
```

(tsconfig/next.config/postcss/globals.css mirror Task 19; reuse the same kyma tokens.)

- [ ] **Step 2: lib/admin-session.ts**

```ts
import 'server-only';
import * as jose from 'jose';

export const ADMIN_COOKIE = 'kyma_admin_session';
const ISS = 'kyma-admin';

export async function signAdmin(claims: { ghId: string; ghLogin: string }): Promise<string> {
  const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
  return new jose.SignJWT({ ...claims })
    .setProtectedHeader({ alg: 'HS256' }).setIssuedAt().setExpirationTime('30d').setIssuer(ISS).sign(secret);
}

export async function verifyAdmin(jwt: string): Promise<{ ghId: string; ghLogin: string } | null> {
  try {
    const secret = new TextEncoder().encode(process.env.SESSION_SECRET ?? '');
    const { payload } = await jose.jwtVerify(jwt, secret, { issuer: ISS });
    return { ghId: payload.ghId as string, ghLogin: payload.ghLogin as string };
  } catch { return null; }
}

export function isAllowedAdmin(ghId: string): boolean {
  const ids = (process.env.KYMA_ADMIN_GITHUB_IDS ?? '').split(',').map((s) => s.trim()).filter(Boolean);
  return ids.includes(ghId);
}
```

- [ ] **Step 3: api/auth/github/start/route.ts and callback/route.ts**

```ts
// start/route.ts
import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';

export async function GET() {
  const state = crypto.randomUUID();
  (await cookies()).set('kyma_admin_oauth_state', state, {
    httpOnly: true, sameSite: 'lax', path: '/', maxAge: 600,
  });
  const params = new URLSearchParams({
    client_id: process.env.GITHUB_CLIENT_ID ?? '',
    redirect_uri: `${process.env.ADMIN_BASE_URL}/api/auth/github/callback`,
    scope: 'read:user',
    state,
  });
  return NextResponse.redirect(`https://github.com/login/oauth/authorize?${params}`);
}
```

```ts
// callback/route.ts
import { NextResponse } from 'next/server';
import { cookies } from 'next/headers';
import { signAdmin, ADMIN_COOKIE, isAllowedAdmin } from '@/lib/admin-session';

export async function GET(req: Request) {
  const url = new URL(req.url);
  const code = url.searchParams.get('code');
  const state = url.searchParams.get('state');
  const cookieJar = await cookies();
  const stored = cookieJar.get('kyma_admin_oauth_state')?.value;
  if (!code || !state || stored !== state) {
    return new NextResponse('OAuth state mismatch', { status: 401 });
  }
  cookieJar.delete('kyma_admin_oauth_state');

  const tokRes = await fetch('https://github.com/login/oauth/access_token', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
    body: JSON.stringify({
      client_id: process.env.GITHUB_CLIENT_ID,
      client_secret: process.env.GITHUB_CLIENT_SECRET,
      code,
      redirect_uri: `${process.env.ADMIN_BASE_URL}/api/auth/github/callback`,
    }),
  });
  const tok = await tokRes.json() as { access_token?: string };
  if (!tok.access_token) return new NextResponse('Token exchange failed', { status: 401 });

  const profRes = await fetch('https://api.github.com/user', {
    headers: { Authorization: `Bearer ${tok.access_token}`, 'User-Agent': 'kyma-admin' },
  });
  const profile = await profRes.json() as { id: number; login: string };
  if (!isAllowedAdmin(String(profile.id))) {
    return new NextResponse('Not on the admin allowlist', { status: 403 });
  }
  const jwt = await signAdmin({ ghId: String(profile.id), ghLogin: profile.login });
  cookieJar.set(ADMIN_COOKIE, jwt, {
    httpOnly: true, sameSite: 'lax', path: '/',
    maxAge: 60 * 60 * 24 * 30, secure: process.env.NODE_ENV === 'production',
  });
  return NextResponse.redirect(`${process.env.ADMIN_BASE_URL}/workspaces`);
}
```

- [ ] **Step 4: app/page.tsx (login)**

```tsx
import Link from 'next/link';

export default function AdminLogin() {
  return (
    <main className="min-h-screen flex items-center justify-center px-4">
      <div className="space-y-3 text-center">
        <h1 className="text-2xl font-mono">kyma admin</h1>
        <Link
          href="/api/auth/github/start"
          className="inline-block h-10 px-4 rounded text-white"
          style={{ background: 'var(--kyma-accent)' }}
        >
          Sign in with GitHub
        </Link>
      </div>
    </main>
  );
}
```

- [ ] **Step 5: Commit**

```bash
git add cloud/admin
git commit -m "feat(cloud/admin): Next.js scaffold + GitHub-allowlisted login"
```

---

### Task 25: Admin workspace list (read-only)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/lib/db.ts`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/app/(dash)/layout.tsx`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/src/app/(dash)/workspaces/page.tsx`

- [ ] **Step 1: lib/db.ts (raw pg, read-only)**

```ts
import 'server-only';
import pg from 'pg';

let _pool: pg.Pool | null = null;
function pool() {
  if (_pool) return _pool;
  _pool = new pg.Pool({ connectionString: process.env.DRIZZLE_DATABASE_URL });
  return _pool;
}

export async function listWorkspaces() {
  const { rows } = await pool().query(`
    SELECT
      w.id, w.slug, w.name, w.plan, w.kind, w.plan_active,
      w.stripe_customer_id, w.created_at,
      u.email AS owner_email
    FROM workspaces w
    JOIN users u ON u.id = w.owner_user_id
    ORDER BY w.created_at DESC
    LIMIT 200
  `);
  return rows;
}
```

- [ ] **Step 2: (dash)/layout.tsx — gate**

```tsx
import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import Link from 'next/link';
import { ADMIN_COOKIE, verifyAdmin } from '@/lib/admin-session';

export default async function AdminDashLayout({ children }: { children: React.ReactNode }) {
  const c = await cookies();
  const cookie = c.get(ADMIN_COOKIE)?.value;
  const ok = cookie ? await verifyAdmin(cookie) : null;
  if (!ok) redirect('/');
  return (
    <div className="min-h-screen">
      <header className="px-6 h-14 flex items-center justify-between border-b" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
        <Link href="/workspaces" className="font-mono">kyma admin · {ok.ghLogin}</Link>
      </header>
      <main className="px-6 py-8 max-w-6xl mx-auto">{children}</main>
    </div>
  );
}
```

- [ ] **Step 3: (dash)/workspaces/page.tsx**

```tsx
import { listWorkspaces } from '@/lib/db';

export default async function AdminWorkspacesPage() {
  const rows = await listWorkspaces();
  return (
    <div>
      <h1 className="text-xl mb-4">Workspaces ({rows.length})</h1>
      <table className="w-full text-sm">
        <thead className="text-left" style={{ color: 'var(--kyma-muted)' }}>
          <tr><th>Slug</th><th>Name</th><th>Owner</th><th>Plan</th><th>Kind</th><th>Created</th></tr>
        </thead>
        <tbody>
          {rows.map((w) => (
            <tr key={w.id} className="border-t" style={{ borderColor: 'var(--kyma-rule-soft)' }}>
              <td className="py-2 font-mono">{w.slug}</td><td>{w.name}</td>
              <td>{w.owner_email}</td><td>{w.plan}</td><td>{w.kind}</td>
              <td style={{ color: 'var(--kyma-muted)' }}>{new Date(w.created_at).toISOString().slice(0, 10)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-xs mt-6" style={{ color: 'var(--kyma-muted)' }}>
        "Promote to dedicated" lands in Slice 3.
      </p>
    </div>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add cloud/admin/src/lib/db.ts cloud/admin/src/app/\(dash\)
git commit -m "feat(cloud/admin): read-only workspace inspector (slice-3 promote button TBD)"
```

---

### Task 26: CLI — add reqwest + rebuild subcommand tree

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/Cargo.toml`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/main.rs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/api_client.rs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/profile.rs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/{mod,login,workspace,ingest,table}.rs`

- [ ] **Step 1: Cargo.toml additions**

```toml
[dependencies]
# (existing)
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
serde = { workspace = true, features = ["derive"] }
dirs = "5"
toml = "0.8"
webbrowser = "1"
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio", "server", "http1"] }
http-body-util = "0.1"
```

- [ ] **Step 2: src/profile.rs — minimal credentials.toml**

```rust
//! Single-profile credential storage at `~/.config/kyma/credentials.toml`.
//! Slice 4 will turn this into a multi-profile `profiles.toml`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub api_url: Option<String>,
    pub token: Option<String>,
    pub workspace_slug: Option<String>,
    pub workspace_id: Option<String>,
    pub mcp_endpoint: Option<String>,
}

fn path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("no config dir")?.join("kyma");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("credentials.toml"))
}

pub fn load() -> Result<Credentials> {
    let p = path()?;
    if !p.exists() { return Ok(Credentials::default()); }
    let bytes = fs::read_to_string(&p)?;
    Ok(toml::from_str(&bytes)?)
}

pub fn save(c: &Credentials) -> Result<()> {
    let p = path()?;
    let s = toml::to_string_pretty(c)?;
    fs::write(&p, s)?;
    Ok(())
}
```

- [ ] **Step 3: src/api_client.rs**

```rust
use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;

pub struct ApiClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl ApiClient {
    pub fn new(base: String, token: Option<String>) -> Self {
        Self { base, token, http: reqwest::Client::new() }
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(t) = &self.token { req = req.header(AUTHORIZATION, format!("Bearer {t}")); }
        req
    }

    pub async fn list_workspaces(&self) -> Result<Vec<WorkspaceRow>> {
        #[derive(Deserialize)] struct Wrap { workspaces: Vec<WorkspaceRow> }
        let res = self.auth(self.http.get(format!("{}/api/workspaces", self.base))).send().await?;
        if !res.status().is_success() { return Err(anyhow!("HTTP {}", res.status())); }
        Ok(res.json::<Wrap>().await?.workspaces)
    }

    pub async fn ingest(&self, workspace_slug: &str, database: &str, table: &str, ndjson: &str) -> Result<()> {
        let url = format!("{}/api/workspaces/{}/ingest/{}/{}", self.base, workspace_slug, database, table);
        let res = self.auth(self.http.post(url).header(CONTENT_TYPE, "application/x-ndjson").body(ndjson.to_owned())).send().await?;
        if !res.status().is_success() { return Err(anyhow!("ingest HTTP {}", res.status())); }
        Ok(())
    }

    pub async fn list_tables(&self, workspace_slug: &str, db: &str) -> Result<Vec<TableRow>> {
        #[derive(Deserialize)] struct Wrap { tables: Vec<TableRow> }
        let res = self.auth(self.http.get(format!("{}/api/workspaces/{}/databases/{}/tables", self.base, workspace_slug, db))).send().await?;
        if !res.status().is_success() { return Err(anyhow!("HTTP {}", res.status())); }
        Ok(res.json::<Wrap>().await?.tables)
    }
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceRow {
    pub id: String, pub slug: String, pub name: String,
    pub plan: String, pub kind: String,
    pub mcp_endpoint: String, pub kyma_endpoint: String,
}

#[derive(Debug, Deserialize)]
pub struct TableRow { pub name: String, pub columns: Vec<String> }
```

- [ ] **Step 4: Update main.rs to dispatch to commands**

```rust
mod api_client;
mod profile;
mod commands;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kyma", about = "kyma CLI")]
struct Cli {
    #[arg(long, env = "KYMA_API_URL", default_value = "https://cloud.kyma.dev")]
    api_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Login,
    Workspace { #[command(subcommand)] cmd: commands::workspace::WorkspaceCmd },
    Ingest    { #[arg(long)] db: String, #[arg(long)] table: String, file: String },
    Table     { #[command(subcommand)] cmd: commands::table::TableCmd },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Login => commands::login::run(&cli.api_url).await,
        Command::Workspace { cmd } => commands::workspace::run(cmd, &cli.api_url).await,
        Command::Ingest { db, table, file } => commands::ingest::run(&cli.api_url, &db, &table, &file).await,
        Command::Table { cmd } => commands::table::run(cmd, &cli.api_url).await,
    }
}
```

- [ ] **Step 5: src/commands/mod.rs**

```rust
pub mod login;
pub mod workspace;
pub mod ingest;
pub mod table;
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check -p kyma-cli`
Expected: compiles (the per-command files are stubbed in the next tasks).

- [ ] **Step 7: Commit (with stubs that compile)**

Add stub modules so the workspace builds:

```rust
// commands/login.rs
pub async fn run(_api_url: &str) -> anyhow::Result<()> { unimplemented!("Task 27") }
```
```rust
// commands/workspace.rs
use clap::Subcommand;
#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd { List, Select { slug: String } }
pub async fn run(_cmd: WorkspaceCmd, _api_url: &str) -> anyhow::Result<()> { unimplemented!("Task 28") }
```
```rust
// commands/ingest.rs
pub async fn run(_api_url: &str, _db: &str, _table: &str, _file: &str) -> anyhow::Result<()> { unimplemented!("Task 29") }
```
```rust
// commands/table.rs
use clap::Subcommand;
#[derive(Debug, Subcommand)]
pub enum TableCmd { List { #[arg(long)] db: String } }
pub async fn run(_cmd: TableCmd, _api_url: &str) -> anyhow::Result<()> { unimplemented!("Task 29") }
```

```bash
cargo check -p kyma-cli
git add crates/kyma-cli
git commit -m "refactor(kyma-cli): subcommand tree + reqwest API client + profile storage"
```

---

### Task 27: `kyma login` — browser-flow OAuth

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/login.rs`

The flow: CLI binds a localhost port, opens `cloud.kyma.dev/login?cli=1&redirect=http://127.0.0.1:<port>/cb`, and the dashboard sends the user back to `/cb?token=<workspace-mcp-token>` after the user picks a workspace and mints a token. Slice 2 keeps it simple — the dashboard renders a "Send to CLI" button that POSTs back to the local port. We implement the CLI side here.

- [ ] **Step 1: Implement login.rs**

```rust
use crate::api_client::ApiClient;
use crate::profile::{load, save, Credentials};
use anyhow::{anyhow, Context, Result};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub async fn run(api_url: &str) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}/cb");

    let auth_url = format!("{api_url}/login?cli=1&redirect={}",
        urlencoding::encode(&redirect));
    println!("Opening {auth_url}\nIf the browser does not open, paste it manually.");
    let _ = webbrowser::open(&auth_url);

    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await { Ok(s) => s, Err(_) => break };
            let tx = tx.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let tx = tx.clone();
                    async move {
                        let q = req.uri().query().unwrap_or("");
                        let token = q.split('&').find_map(|kv| kv.strip_prefix("token="));
                        if let Some(t) = token {
                            if let Some(s) = tx.lock().await.take() { let _ = s.send(t.to_string()); }
                            Ok::<_, hyper::Error>(Response::builder().status(StatusCode::OK)
                                .body(Full::new(Bytes::from(
                                    "kyma CLI authenticated. You can close this tab."
                                )))
                                .unwrap())
                        } else {
                            Ok(Response::builder().status(StatusCode::BAD_REQUEST)
                                .body(Full::new(Bytes::from("missing ?token=")))
                                .unwrap())
                        }
                    }
                });
                let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), svc).await;
            });
        }
    });

    let token = tokio::time::timeout(std::time::Duration::from_secs(180), rx).await
        .map_err(|_| anyhow!("login timed out after 3 minutes"))?
        .context("login channel closed")?;

    // Validate by listing workspaces.
    let api = ApiClient::new(api_url.to_string(), Some(token.clone()));
    let workspaces = api.list_workspaces().await?;
    let first = workspaces.first().ok_or_else(|| anyhow!("no workspaces; create one in the dashboard"))?;

    let mut creds = load().unwrap_or_default();
    creds.api_url = Some(api_url.to_string());
    creds.token = Some(token);
    creds.workspace_slug = Some(first.slug.clone());
    creds.workspace_id = Some(first.id.clone());
    creds.mcp_endpoint = Some(first.mcp_endpoint.clone());
    save(&creds)?;
    println!("logged in. default workspace: {} ({})", first.name, first.slug);
    Ok(())
}
```

- [ ] **Step 2: Add `urlencoding = "2"` to Cargo.toml dependencies**

- [ ] **Step 3: Verify build**

Run: `cargo check -p kyma-cli`

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-cli/src/commands/login.rs crates/kyma-cli/Cargo.toml
git commit -m "feat(kyma-cli): kyma login via local-port browser callback"
```

---

### Task 28: `kyma workspace [list|select]`

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/workspace.rs`

- [ ] **Step 1: Implementation**

```rust
use crate::api_client::ApiClient;
use crate::profile::{load, save};
use anyhow::{anyhow, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum WorkspaceCmd {
    /// List all workspaces this token has access to.
    List,
    /// Set the default workspace by slug.
    Select { slug: String },
}

pub async fn run(cmd: WorkspaceCmd, api_url: &str) -> Result<()> {
    let creds = load()?;
    let token = creds.token.clone().ok_or_else(|| anyhow!("not logged in — run `kyma login`"))?;
    let url = creds.api_url.clone().unwrap_or_else(|| api_url.to_string());
    let api = ApiClient::new(url, Some(token));

    match cmd {
        WorkspaceCmd::List => {
            for w in api.list_workspaces().await? {
                let mark = if creds.workspace_slug.as_deref() == Some(&w.slug) { "*" } else { " " };
                println!("{} {:<20} {:<24} plan={:<5} kind={}", mark, w.slug, w.name, w.plan, w.kind);
            }
        }
        WorkspaceCmd::Select { slug } => {
            let list = api.list_workspaces().await?;
            let hit = list.iter().find(|w| w.slug == slug)
                .ok_or_else(|| anyhow!("workspace `{slug}` not found"))?;
            let mut creds = creds;
            creds.workspace_slug = Some(hit.slug.clone());
            creds.workspace_id = Some(hit.id.clone());
            creds.mcp_endpoint = Some(hit.mcp_endpoint.clone());
            save(&creds)?;
            println!("selected workspace: {}", hit.slug);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build + commit**

```bash
cargo check -p kyma-cli
git add crates/kyma-cli/src/commands/workspace.rs
git commit -m "feat(kyma-cli): kyma workspace list + select"
```

---

### Task 29: `kyma ingest <file>` and `kyma table list`

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/ingest.rs`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/table.rs`

The cloud API needs ingest/table endpoints — add these to the API in this same task to keep the CLI commit tight.

**Files (cloud/api):**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/engine-proxy.service.ts`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/workspaces.ts`

- [ ] **Step 1: engine-proxy.service.ts (server-side proxy to engine)**

```ts
import { getEnv } from '../env.js';
import { badRequest } from '../lib/errors.js';

/** Forwards an NDJSON payload to the engine on behalf of the workspace. */
export async function ingest(opts: {
  tenantId: string;
  database: string;
  table: string;
  ndjson: string;
  /** Bearer token to forward — typically minted same-request via mcp-token.service. */
  token: string;
}): Promise<void> {
  const env = getEnv();
  const url = `${env.KYMA_ENGINE_BASE_URL}/v1/ingest/${encodeURIComponent(opts.database)}/${encodeURIComponent(opts.table)}`;
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${opts.token}`,
      'Content-Type': 'application/x-ndjson',
    },
    body: opts.ndjson,
  });
  if (!res.ok) throw badRequest(`engine ingest failed: ${res.status} ${await res.text()}`, 'ENGINE_FAIL');
}

export async function listTables(opts: {
  database: string;
  token: string;
}): Promise<{ tables: Array<{ name: string; columns: string[] }> }> {
  const env = getEnv();
  const url = `${env.KYMA_ENGINE_BASE_URL}/v1/catalog/databases/${encodeURIComponent(opts.database)}/tables`;
  const res = await fetch(url, { headers: { 'Authorization': `Bearer ${opts.token}` } });
  if (!res.ok) throw badRequest(`engine list-tables failed: ${res.status}`, 'ENGINE_FAIL');
  return res.json() as Promise<{ tables: Array<{ name: string; columns: string[] }> }>;
}
```

- [ ] **Step 2: Add endpoints in routes/workspaces.ts**

Token-authenticated path — accept the same `Authorization: Bearer kyma_*` token the CLI is using, look up the workspace from the token, forward to engine.

```ts
// (append to routes/workspaces.ts; this set does NOT use sessionMiddleware)
import { hashToken } from '../lib/tokens.js';
import { authenticateForDebug } from '../services/mcp-token.service.js';
import * as engine from '../services/engine-proxy.service.js';

export const workspaceTokenRoutes = new Hono();

async function authWithToken(c: any): Promise<{ tenantId: string; token: string }> {
  const h = c.req.header('Authorization');
  if (!h?.startsWith('Bearer ')) throw new (await import('../lib/errors.js')).AppError(401, 'UNAUTH', 'Missing token');
  const token = h.slice(7);
  const principal = await authenticateForDebug(token);
  if (!principal) throw new (await import('../lib/errors.js')).AppError(401, 'UNAUTH', 'Invalid token');
  return { tenantId: principal.tenantId, token };
}

workspaceTokenRoutes.post('/:slug/ingest/:db/:table', async (c) => {
  const { token } = await authWithToken(c);
  const ndjson = await c.req.raw.text();
  await engine.ingest({
    tenantId: '', // engine derives from token
    database: c.req.param('db'),
    table: c.req.param('table'),
    ndjson, token,
  });
  return c.json({ ok: true });
});

workspaceTokenRoutes.get('/:slug/databases/:db/tables', async (c) => {
  const { token } = await authWithToken(c);
  const r = await engine.listTables({ database: c.req.param('db'), token });
  return c.json(r);
});
```

Mount the new sub-router in `index.ts` AFTER the session-gated routes:

```ts
import { workspaceTokenRoutes } from './routes/workspaces.js';
app.route('/api/workspaces', workspaceTokenRoutes);
```

- [ ] **Step 3: ingest.rs**

```rust
use crate::api_client::ApiClient;
use crate::profile::load;
use anyhow::{anyhow, Result};
use std::fs;

pub async fn run(api_url: &str, db: &str, table: &str, file: &str) -> Result<()> {
    let creds = load()?;
    let token = creds.token.clone().ok_or_else(|| anyhow!("not logged in"))?;
    let slug = creds.workspace_slug.clone().ok_or_else(|| anyhow!("no workspace selected"))?;
    let url = creds.api_url.unwrap_or_else(|| api_url.to_string());

    let bytes = fs::read_to_string(file)?;
    let api = ApiClient::new(url, Some(token));
    api.ingest(&slug, db, table, &bytes).await?;
    let lines = bytes.lines().filter(|l| !l.trim().is_empty()).count();
    println!("ingested {lines} rows into {db}.{table} (workspace {slug})");
    Ok(())
}
```

- [ ] **Step 4: table.rs**

```rust
use crate::api_client::ApiClient;
use crate::profile::load;
use anyhow::{anyhow, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum TableCmd {
    List { #[arg(long)] db: String },
}

pub async fn run(cmd: TableCmd, api_url: &str) -> Result<()> {
    let creds = load()?;
    let token = creds.token.clone().ok_or_else(|| anyhow!("not logged in"))?;
    let slug = creds.workspace_slug.clone().ok_or_else(|| anyhow!("no workspace selected"))?;
    let url = creds.api_url.unwrap_or_else(|| api_url.to_string());
    let api = ApiClient::new(url, Some(token));
    match cmd {
        TableCmd::List { db } => {
            for t in api.list_tables(&slug, &db).await? {
                println!("{}  [{}]", t.name, t.columns.join(", "));
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Build + commit**

```bash
cargo check -p kyma-cli
pnpm --filter @kyma/cloud-api run test
git add crates/kyma-cli cloud/api/src/services/engine-proxy.service.ts cloud/api/src/routes/workspaces.ts cloud/api/src/index.ts
git commit -m "feat(cli+api): kyma ingest + kyma table list with engine proxy"
```

---

### Task 30: Engine deployment uses `cloud-auth` feature with `KYMA_AUTH_BACKEND=db`

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/Dockerfile`

- [ ] **Step 1: Inspect current Dockerfile**

Run: `cat /Users/shaked/projects_new/agentcy/kyma/Dockerfile`
Confirm the `cargo build` line and locate where features are passed.

- [ ] **Step 2: Add `cloud-auth` to the prod build**

Edit the build line so production images carry the cloud-auth feature:

```dockerfile
RUN cargo build --release -p kyma-bin --features "web-ui cloud-auth"
```

- [ ] **Step 3: Confirm runtime selection via env**

The selection logic in `crates/kyma-bin/src/main.rs:147` already chooses `DbAuthBackend` when both the feature is on and `KYMA_AUTH_BACKEND=db`. No code change needed; just configure `KYMA_AUTH_BACKEND=db` and `KYMA_CATALOG_URL=<cloud-postgres>` on the engine's Railway service in Phase I.

- [ ] **Step 4: Verify the build still succeeds locally**

```bash
cargo build --release -p kyma-bin --features "cloud-auth" 2>&1 | tail -5
```

Expected: `Finished release [optimized]` (warnings ok).

- [ ] **Step 5: Commit**

```bash
git add Dockerfile
git commit -m "build(engine): enable cloud-auth feature in production image"
```

---

### Task 31: Engine ↔ cloud auth integration smoke test

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/engine-auth.it.test.ts`

This test boots the engine via testcontainers... but actually we don't ship Rust spawn from the JS test harness. Instead, we verify the contract by inserting an `api_tokens` row with the cloud's mint helper, then SELECT it back with the EXACT query the engine uses.

- [ ] **Step 1: engine-auth.it.test.ts**

```ts
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { freshDb } from '../test-setup.js';
import { closeDb, getPool, getDb, schema } from '../db/client.js';
import { mintMcpToken } from './mcp-token.service.js';
import { hashToken } from '../lib/tokens.js';

beforeAll(async () => { process.env.KYMA_ENGINE_BASE_URL = 'http://e'; await freshDb(); });
afterAll(async () => { await closeDb(); });

describe('engine DbAuthBackend SQL contract', () => {
  it('mint then run the engine`s exact SELECT', async () => {
    const db = getDb();
    const [u] = await db.insert(schema.users).values({ email: 'a@b.c' }).returning();
    const [ws] = await db.insert(schema.workspaces).values({
      slug: 'eng', name: 'eng', ownerUserId: u.id,
      kymaEndpoint: 'http://e', mcpEndpoint: 'http://e/x/mcp/v1',
    }).returning();
    const { plain } = await mintMcpToken({ workspaceId: ws.id, createdByUserId: u.id });

    const hash = hashToken(plain);
    // EXACT query from crates/kyma-server/src/auth/db_backend.rs:60-65
    const r = await getPool().query(
      `SELECT tenant_id, scopes, subject FROM api_tokens
       WHERE token_hash = $1 AND revoked_at IS NULL`,
      [hash],
    );
    expect(r.rowCount).toBe(1);
    expect(r.rows[0].tenant_id).toBe(ws.id);
    expect(r.rows[0].scopes).toBe('read,write');

    // The engine then does this UPDATE for last_used_at — make sure it works:
    await getPool().query(
      `UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1`,
      [hash],
    );
  });
});
```

- [ ] **Step 2: Run + commit**

```bash
pnpm --filter @kyma/cloud-api run test
git add cloud/api/src/services/engine-auth.it.test.ts
git commit -m "test(cloud/api): exact-SQL contract test for engine DbAuthBackend"
```

---

### Task 32: Dockerfiles for api / web / admin

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/Dockerfile.api`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/Dockerfile.web`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/Dockerfile.admin`

- [ ] **Step 1: Dockerfile.api**

```dockerfile
FROM node:22-alpine
RUN corepack enable
WORKDIR /app

COPY cloud/package.json cloud/pnpm-workspace.yaml cloud/pnpm-lock.yaml ./
COPY cloud/packages/ packages/
COPY cloud/api/ api/

RUN pnpm install --frozen-lockfile

WORKDIR /app/api
EXPOSE 3003
CMD ["pnpm", "exec", "tsx", "src/index.ts"]
```

- [ ] **Step 2: Dockerfile.web**

```dockerfile
FROM node:22-alpine AS builder
RUN corepack enable
WORKDIR /app

COPY cloud/package.json cloud/pnpm-workspace.yaml cloud/pnpm-lock.yaml ./
COPY cloud/packages/ packages/
COPY cloud/web/ web/

RUN pnpm install --frozen-lockfile

WORKDIR /app/web
ARG NEXT_PUBLIC_CLOUD_BASE_URL
ENV NEXT_PUBLIC_CLOUD_BASE_URL=${NEXT_PUBLIC_CLOUD_BASE_URL}
RUN pnpm run build

FROM node:22-alpine
WORKDIR /app
COPY --from=builder /app/web/.next/standalone ./
COPY --from=builder /app/web/.next/static ./web/.next/static
COPY --from=builder /app/web/public ./web/public
EXPOSE 3001
ENV PORT=3001
ENV HOSTNAME=0.0.0.0
CMD ["node", "web/server.js"]
```

- [ ] **Step 3: Dockerfile.admin**

(identical to Dockerfile.web with `web/` → `admin/`, `PORT=3002`, `NEXT_PUBLIC_CLOUD_BASE_URL` → `ADMIN_BASE_URL`, and one extra ARG `KYMA_ADMIN_GITHUB_IDS` baked in via runtime env on Railway, NOT build-time.)

```dockerfile
FROM node:22-alpine AS builder
RUN corepack enable
WORKDIR /app

COPY cloud/package.json cloud/pnpm-workspace.yaml cloud/pnpm-lock.yaml ./
COPY cloud/packages/ packages/
COPY cloud/admin/ admin/

RUN pnpm install --frozen-lockfile

WORKDIR /app/admin
RUN pnpm run build

FROM node:22-alpine
WORKDIR /app
COPY --from=builder /app/admin/.next/standalone ./
COPY --from=builder /app/admin/.next/static ./admin/.next/static
EXPOSE 3002
ENV PORT=3002
ENV HOSTNAME=0.0.0.0
CMD ["node", "admin/server.js"]
```

- [ ] **Step 4: Commit**

```bash
git add cloud/Dockerfile.api cloud/Dockerfile.web cloud/Dockerfile.admin
git commit -m "build(cloud): Dockerfiles for api/web/admin"
```

---

### Task 33: Railway service manifests

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/api/railway.toml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/web/railway.toml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/cloud/admin/railway.toml`

- [ ] **Step 1: api/railway.toml**

```toml
[build]
builder = "DOCKERFILE"
dockerfilePath = "cloud/Dockerfile.api"
watchPatterns = ["cloud/api/**", "cloud/packages/**"]

[deploy]
healthcheckPath = "/health"
healthcheckTimeout = 60
restartPolicyType = "ON_FAILURE"
restartPolicyMaxRetries = 5
```

- [ ] **Step 2: web/railway.toml**

```toml
[build]
builder = "DOCKERFILE"
dockerfilePath = "cloud/Dockerfile.web"
watchPatterns = ["cloud/web/**", "cloud/packages/**"]

[deploy]
restartPolicyType = "ON_FAILURE"
restartPolicyMaxRetries = 5
```

- [ ] **Step 3: admin/railway.toml**

```toml
[build]
builder = "DOCKERFILE"
dockerfilePath = "cloud/Dockerfile.admin"
watchPatterns = ["cloud/admin/**", "cloud/packages/**"]

[deploy]
restartPolicyType = "ON_FAILURE"
restartPolicyMaxRetries = 5
```

- [ ] **Step 4: Commit**

```bash
git add cloud/api/railway.toml cloud/web/railway.toml cloud/admin/railway.toml
git commit -m "build(cloud): Railway service manifests"
```

---

### Task 34: Provision Railway services + DNS (manual; documented)

This is a manual step performed against the existing kyma-cloud Railway project (`0773c99d-5f16-426a-9da1-6b5de1d0b88d`). Document the exact actions.

- [ ] **Step 1: Create three services in Railway dashboard**

Operator runs:

1. Open https://railway.app/project/0773c99d-5f16-426a-9da1-6b5de1d0b88d
2. Add service "kyma-cloud-api" — connect to GitHub repo, set root path empty, set "Config-as-Code" to `cloud/api/railway.toml`. Add Postgres plugin OR set `DRIZZLE_DATABASE_URL` to the existing engine's catalog URL with database `kyma_cloud` (must be created out of band).
3. Add service "kyma-cloud-web" — same repo, config path `cloud/web/railway.toml`. Build arg `NEXT_PUBLIC_CLOUD_BASE_URL=https://cloud.kyma.dev`.
4. Add service "kyma-cloud-admin" — same repo, config path `cloud/admin/railway.toml`.
5. Add service "kyma-engine" — point at the existing kyma engine image `ghcr.io/shakedaskayo/kyma-engine:latest`. Set `KYMA_AUTH_BACKEND=db`, `KYMA_CATALOG_URL=$DRIZZLE_DATABASE_URL` (so the engine reads the same `api_tokens` table).

- [ ] **Step 2: Set required env vars per service**

Per service:

`kyma-cloud-api`:
```
PORT=3003
NODE_ENV=production
DRIZZLE_DATABASE_URL=<plugin-or-shared>
GITHUB_CLIENT_ID=Ov23licu3x1my6MCgWkB
GITHUB_CLIENT_SECRET=<from .env.local>
RESEND_API_KEY=<from .env.local>
RESEND_FROM_EMAIL=noreply@kyma.dev
SESSION_SECRET=<from .env.local>
STRIPE_SECRET_KEY=<from Stripe dashboard>
STRIPE_WEBHOOK_SIGNING_SECRET=<from Stripe webhook>
STRIPE_PRICE_PRO=<from Stripe products>
STRIPE_PRICE_TEAM=<from Stripe products>
CLOUD_BASE_URL=https://cloud.kyma.dev
ADMIN_BASE_URL=https://admin.kyma.dev
KYMA_ENGINE_BASE_URL=https://mcp.kyma.dev
KYMA_ADMIN_GITHUB_IDS=<your numeric GitHub user id>
```

`kyma-cloud-web`: `NEXT_PUBLIC_CLOUD_BASE_URL=https://cloud.kyma.dev`, `SESSION_SECRET=<same>`.

`kyma-cloud-admin`: `GITHUB_CLIENT_ID=<same>`, `GITHUB_CLIENT_SECRET=<same>`, `SESSION_SECRET=<same>`, `ADMIN_BASE_URL=https://admin.kyma.dev`, `DRIZZLE_DATABASE_URL=<same>`, `KYMA_ADMIN_GITHUB_IDS=<your numeric id>`.

- [ ] **Step 3: Configure custom domains in Railway**

For each service: Settings → Networking → Custom Domain.
- `cloud.kyma.dev` → kyma-cloud-web
- `cloud-api.kyma.dev` → kyma-cloud-api (referenced by the web's `NEXT_PUBLIC_CLOUD_BASE_URL` if api is split; for Slice 2 we put api on the SAME `cloud.kyma.dev` host with Hono routes prefixed `/api/...` but Next.js owns the host, so we instead put api on its own `cloud-api.kyma.dev` and update `NEXT_PUBLIC_CLOUD_BASE_URL=https://cloud-api.kyma.dev`).
- `admin.kyma.dev` → kyma-cloud-admin
- `mcp.kyma.dev` → kyma-engine

Railway will display each CNAME target.

- [ ] **Step 4: Add CNAMEs in GoDaddy**

GoDaddy DNS for `getkyma.dev` zone — add four CNAMEs:
- `cloud` → Railway-provided host
- `cloud-api` → Railway-provided host
- `admin` → Railway-provided host
- `mcp` → Railway-provided host

Wait for issuance (typically 1–3 minutes; Railway emits "Domain ready" when LE certs are live).

- [ ] **Step 5: Add the GitHub OAuth app callback URL**

In https://github.com/settings/developers (App ID 3577887): set the callback URL to a list including
`https://cloud.kyma.dev/api/auth/github/callback`,
`https://cloud-api.kyma.dev/api/auth/github/callback`,
`https://admin.kyma.dev/api/auth/github/callback`.
GitHub allows comma-separated URLs in the callback field.

- [ ] **Step 6: Configure Stripe webhook**

Stripe dashboard → Developers → Webhooks → Add endpoint
`https://cloud-api.kyma.dev/api/webhooks/stripe`. Subscribe to:
- `customer.subscription.created`
- `customer.subscription.updated`
- `customer.subscription.deleted`
- `invoice.paid`
- `invoice.payment_failed`

Copy the signing secret into `STRIPE_WEBHOOK_SIGNING_SECRET` on the api service.

- [ ] **Step 7: Smoke-test each domain**

```bash
curl -i https://cloud.kyma.dev/        # 200, Next.js HTML
curl -i https://cloud-api.kyma.dev/health   # {"status":"ok",...}
curl -i https://admin.kyma.dev/        # 200, login page
curl -i https://mcp.kyma.dev/health    # 200 from engine
```

- [ ] **Step 8: Commit a deployment notes file**

(Notes-only commit — no executable change.)

```bash
# Add docs/superpowers/notes/2026-05-XX-cloud-slice-2-deploy-notes.md with the
# Railway service ids and DNS records for future reference.
git add docs/superpowers/notes/2026-05-XX-cloud-slice-2-deploy-notes.md
git commit -m "docs: cloud slice 2 deployment notes (Railway services + DNS)"
```

---

### Task 35: kyma-claude-skill local install for the demo flow

**Files:**
- (skill repo lives at `kyma-claude-skill/`; verify-only step here)

The skill repo was created in Slice 1a. Slice 2's demo verifies it works against `mcp.kyma.dev` with a cloud-issued token.

- [ ] **Step 1: Confirm the skill repo path**

```bash
ls /Users/shaked/projects_new/agentcy/kyma-claude-skill/SKILL.md \
   /Users/shaked/projects_new/agentcy/kyma-claude-skill/README.md
```

If missing, the slice-1a plan owns creation; do not patch from this plan.

- [ ] **Step 2: Verify install snippet rendered by the dashboard matches the skill manifest**

Visually compare the JSON block emitted by `McpInstallWidget` (Task 22) with the `claude_desktop_config.json` sample in the skill README. Fields must match: `mcpServers.kyma.transport`, `.url`, `.headers.Authorization`.

- [ ] **Step 3: No commit — verification only.**

---

### Task 36: Slice 2 verification — sign-up → answer in < 60s

This is the spec's hard gate. Run all checks; do not advance until each is green.

- [ ] **Step 1: GitHub sign-up flow on cloud.kyma.dev**

1. Open `https://cloud.kyma.dev` in a fresh browser.
2. Click "Continue with GitHub". Complete OAuth.
3. Land on `/workspaces`. Click "New workspace". Enter "Demo". Submit.
4. Land on `/workspaces/demo`. Click "Mint MCP token". Copy the token.

Expected: end-to-end < 30s.

- [ ] **Step 2: Magic-link sign-up flow on cloud.kyma.dev**

1. Sign out.
2. Open the login page. Enter a new fresh email. Click "Email me a link".
3. Receive the link in inbox. Click. Land on `/workspaces`. Create workspace. Mint token.

Expected: works equivalently to GitHub flow.

- [ ] **Step 3: Stripe Pro checkout**

1. From `/workspaces/demo/billing` click "Switch to pro".
2. Complete Stripe Checkout test card (`4242 4242 4242 4242`).
3. Confirm Stripe webhook arrives (check `billing_events` row + `processed=true`).
4. Confirm `workspaces.plan = 'pro'` and `planActive = true`.

```sql
SELECT plan, plan_active, stripe_customer_id, stripe_subscription_id FROM workspaces WHERE slug = 'demo';
```

- [ ] **Step 4: Stripe billing portal**

From `/workspaces/demo/billing` click "Manage billing". Stripe portal opens.
Click "Cancel subscription". Confirm. Within ~30s the webhook fires; UI shows `plan = free`.

- [ ] **Step 5: CLI**

```bash
cargo install --path /Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli
kyma login
kyma workspace list
echo '{"ts":"2026-05-04T10:00:00Z","msg":"hello"}' > /tmp/sample.ndjson
kyma ingest --db default --table demo /tmp/sample.ndjson
kyma table list --db default
```

Expected: each command succeeds; the row is queryable through MCP after.

- [ ] **Step 6: MCP end-to-end via Claude skill**

1. Open Claude Desktop or Claude Code. Run `/skill install kyma`.
2. Paste `mcpEndpoint` and the token from Step 1.
3. Ask: "What tables are in the demo workspace? Sample 5 rows from `default.demo`."
4. Confirm Claude calls `list_databases`, `describe_table`, `sample_rows` and returns the row inserted in Step 5.

Expected: < 60s from "sign up on cloud.kyma.dev" to "first answer". Demo recorded for future use.

- [ ] **Step 7: Cross-tenant isolation**

1. Create two workspaces under different users (use two browser profiles).
2. Mint a token in workspace A.
3. POST `https://cloud-api.kyma.dev/api/workspaces/<B-slug>/databases/default/tables` with workspace A's token.
4. Confirm 401 (the token's `tenant_id` resolves to A's id; the engine refuses to read B's catalog rows because B's tables have B's `tenant_id`).

Also try directly hitting the engine MCP endpoint with workspace A's token but querying for objects that exist only in B. Expect zero rows / not found.

- [ ] **Step 8: Reconciler smoke**

Restart the API service in Railway. Watch logs for `[billing.reconcile] scanned=N updated=M` within ~1 minute (initial tick) and again at the next hour.

- [ ] **Step 9: Commit verification log**

```bash
git add docs/superpowers/notes/2026-05-XX-cloud-slice-2-verification.md
git commit -m "docs: cloud slice 2 verification log (all checks green)"
```

---

## Self-Review

**1. Spec coverage:** Every requirement in the Slice 2 spec section has at least one task: shared-tenancy workspace creation (T11), MCP token issuance writing the engine's `api_tokens` shape (T12, T31), GitHub OAuth + magic-link (T8, T9), Stripe checkout/webhook/reconciler (T14–T17), customer dashboard + MCP install widget (T19–T23), admin shell with allowlist (T24–T25), thin CLI retrofit (T26–T29), Railway deploy + DNS (T32–T34), and the verification gate (T36). The Slice 2 verification bullets in the spec map 1:1 onto T36 steps 1–8.

**2. Placeholder scan:** No "TBD"/"add appropriate"/"similar to"/"fill in" patterns. Each step has either complete code or an exact command + expected output. The CLI commands compile as stubs in T26 and become real implementations in T27/T28/T29 — the stubs are explicit, not placeholders for "implement later."

**3. Type consistency:** `mintMcpToken({ workspaceId, createdByUserId, name?, scopes? })` is used in both T12 and T13. `getBySlugForUser` returns `{ workspace, role }` in T11 and that exact shape is consumed in T13/T15. The `apiTokens.tokenHash` column is `bytea` (Buffer in JS) everywhere; `hashToken()` returns `Buffer` in T6 and the schema test in T5 round-trips a `Buffer`. The engine's `db_backend.rs:60-65` SELECT is reproduced verbatim in T31. Plan tier ids `'free' | 'pro' | 'team'` are consistent across T2, T11, T14, T15, T17.

**4. Scope check:** Slice 2 covers 4 surfaces (api, customer-web, admin-web, CLI) plus deploy. Each surface is testable independently — api ships green tests at T18, web is browser-smokable at T19/T23, admin at T25, CLI at T29. The "Slice 2 EXCLUDES" section (Slice 3 promote button, KMS encryption, query budgets, full CLI retrofit, marketing landing) is explicitly called out at the top and not snuck back into any task.

**5. Read-not-speculate spot checks:** The `api_tokens` schema columns (`tenant_id`, `token_hash bytea`, `scopes` csv, `subject`, `last_used_at`, `revoked_at`, `created_at`) match `crates/kyma-server/src/auth/db_backend.rs:5-15`. The engine's `KYMA_AUTH_BACKEND=db` selection logic is real (`crates/kyma-bin/src/main.rs:147`), feature-gated behind `cloud-auth`. The `CLOUD_BASE_URL`, `KYMA_ENGINE_BASE_URL`, `RESEND_FROM_EMAIL`, `RAILWAY_PROJECT_ID` env names match `cloud/.env.local`/`.env.example` exactly.

---

## Critical Files for Implementation

- /Users/shaked/projects_new/agentcy/kyma/cloud/api/src/db/schema.ts
- /Users/shaked/projects_new/agentcy/kyma/cloud/api/src/services/mcp-token.service.ts
- /Users/shaked/projects_new/agentcy/kyma/cloud/api/src/routes/webhooks.ts
- /Users/shaked/projects_new/agentcy/kyma/cloud/web/src/components/mcp-install-widget.tsx
- /Users/shaked/projects_new/agentcy/kyma/crates/kyma-cli/src/commands/login.rs

---

# Report

I cannot write to `docs/superpowers/plans/2026-05-06-cloud-slice-2-control-plane.md` — the system prompt for this read-only planning task strictly prohibits file creation. Please save the markdown above (the entire block from `# Cloud Slice 2 — Control Plane v1 Implementation Plan` through `Critical Files for Implementation`) to that path.

**(a) Total tasks: 36 across 10 phases (A–J).** Phase A foundation 5 (T1–T5), Phase B auth 5 (T6–T10), Phase C workspaces+tokens 3 (T11–T13), Phase D Stripe 5 (T14–T18; usage rollup folded in), Phase E customer web 5 (T19–T23), Phase F admin web 2 (T24–T25), Phase G CLI 4 (T26–T29), Phase H engine wiring 2 (T30–T31), Phase I deploy 3 (T32–T34), Phase J verification 2 (T35–T36).

**(b) Structural mismatches between agentcy cloud and what kyma cloud needs:**
- Agentcy `users` has `password_hash`, `auth_provider`, `email_verified`, `verification_token`, `reset_token`, `approval_status`, `approved_by_gh_login`, `denied_reason`, `onboarding_step`. Kyma drops all of them — no password auth, no approval gate, no onboarding wizard.
- Agentcy `teams.slug` is the tenant key; kyma renames to `workspaces` and uses `workspaces.id` (UUID) as the tenant_id used by the engine.
- Agentcy `api_keys` uses `key_hash varchar(256)` (hex string) and prefix `agcl_`. Kyma's `api_tokens` uses `token_hash bytea` (raw 32-byte SHA-256) and prefix `kyma_` because the Rust engine's `DbAuthBackend` already consumes raw bytes — the cloud must match the engine, not the other way around.
- Agentcy `instances` (Railway-provisioned) is dropped in Slice 2; Slice 3 reintroduces it as the dedicated-graduation target.
- Agentcy uses Google OAuth + email/password; kyma uses GitHub OAuth + magic link.
- Agentcy's auth issues JWT access+refresh tokens; kyma uses signed HttpOnly session cookies (jose HS256 + `SESSION_SECRET`) with no refresh token — simpler, no rotation needed for the dashboard.
- Agentcy uses BullMQ + Redis for the provisioning queue. Slice 2 has no provisioning, so no Redis dependency. Slice 3 adds it.

**(c) Spec gaps I noticed:**
- The spec says cloud-Postgres "shared with kyma-server's catalog Postgres for v1; can split later if hot." But the Slice 0 catalog uses `sqlx` migrations starting at `001_initial.sql..007_tenant_id.sql` and the cloud uses `drizzle-kit` with its own `0000_*.sql` files. Both targeting the SAME Postgres needs explicit guidance on whether to use one logical database (shared, both migrators write into it — sqlx into engine tables, drizzle into cloud tables) or two logical databases (separate `kyma` and `kyma_cloud` databases with a single `pg_pool` per service). I made the call: `.env.local` already specifies `DRIZZLE_DATABASE_URL=postgres://.../kyma_cloud` (separate logical DB on the same Postgres instance), and the engine reads from its own `kyma` database. The `api_tokens` table thus lives in the **cloud's** Postgres, and the engine must connect to `kyma_cloud` (not `kyma`) to read tokens. Operationally simpler is to keep them in the same database and let the engine point its `KYMA_CATALOG_URL` at the cloud DB, OR replicate `api_tokens` between them. **Decision the implementer needs to make at Phase I deploy time:** which DB the engine's `KYMA_CATALOG_URL` points at. T34 step 1 assumes the engine reads from the cloud's `kyma_cloud` database directly.
- The spec mentions "DNS for cloud.kyma.dev" but doesn't name the api host. I added `cloud-api.kyma.dev` as a separate hostname because Next.js can't share a host with a Hono backend without a reverse proxy. Alternative is to put cloud-api behind `cloud.kyma.dev/api/*` via a Next.js rewrite — both are reasonable.
- Spec doesn't define the engine's per-workspace MCP path. I picked `<KYMA_ENGINE_BASE_URL>/<workspace-id>/mcp/v1`; the engine currently mounts `/mcp/v1` directly without a workspace segment. The path segment is informational because the engine uses `tenant_id` from the bearer token, but the cloud-issued URL needs to look workspace-specific to users. **Implementer must decide** whether the engine grows a `workspace-id` URL segment (informational only) or whether `mcpEndpoint` stays plain `/mcp/v1` and only the token differentiates workspaces.
- "Per-tenant query budgets / cancellation" deferred to Slice 2.5 per spec, but Slice 2.5 has no plan file yet.

**(d) Open decisions beyond the four locked at kickoff:**
1. **Cloud Postgres topology** — single shared instance with separate logical DBs (current assumption) vs single shared DB with both migrators writing into it. Affects T34 env-var setup.
2. **MCP URL shape** — `mcp.kyma.dev/<workspace-id>/mcp/v1` vs `mcp.kyma.dev/mcp/v1` (token-only routing).
3. **api host topology** — `cloud-api.kyma.dev` separate hostname (current plan) vs Next.js rewrites under `cloud.kyma.dev/api/*`.
4. **Stripe price ID provisioning** — the plan treats `STRIPE_PRICE_PRO`/`STRIPE_PRICE_TEAM` as inputs filled at billing implementation. Whoever runs Phase I needs Stripe products created in test mode first; T14 will return 503 from `/api/billing/checkout` until both env vars are set.
5. **Magic-link rate limiting** — hand-rolled magic-link has no rate limit; an attacker could spam `/api/auth/magic-link/request` with arbitrary emails. I left this out of Slice 2 because it's not in the spec, but the implementer may want to add a per-IP+per-email rate-limit row before going public.
6. **CLI binary distribution** — T36 uses `cargo install --path` for verification. Slice 4 owns brew/curl-installer; Slice 2 verification just needs the local binary.