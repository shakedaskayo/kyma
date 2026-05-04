# kyma cloud — end-to-end SaaS platform

## Context

kyma the engine is shippable today (live at https://getkyma.dev with full docs).
What's missing is the path from "self-hosted Rust binary" to "managed product
that a user can sign up for, see value in 60 seconds, and pay for at scale."

This plan covers building **kyma cloud** at `cloud.kyma.dev` with:

1. A real product surface — sign up with GitHub or magic link, get a workspace,
   see data flowing, pay via Stripe.
2. **Per-workspace MCP endpoints** at `mcp.kyma.dev/<workspace-id>` so any
   agent (Claude, Cursor, custom) can connect with one bearer token. A
   `kyma-claude-skill` repo lets users install the connector with a `/skill
   install` command and start querying their data instantly.
3. **Two tenancy modes** — shared by default (Free/Pro on a single kyma
   cluster, isolated by `tenant_id`); admin-promoted graduation to a
   dedicated Railway-provisioned cluster for Team/Enterprise.
4. A retrofitted **`kyma-cli`** as the developer-friendly entrypoint —
   `kyma login`, `kyma ingest`, `kyma connector add` — talking to the cloud
   over HTTPS instead of Postgres-direct.
5. **Stripe** plan tiers (Free / Pro / Team) + credits for overages, mirroring
   agentcy cloud's data model so the implementation is largely cribbing from
   working code.

The goal is "user clicks a link, sees their data answered by an agent, in
under 60 seconds." Everything else is infrastructure to make that demoable
at scale.

The architecture mirrors **agentcy cloud** (`/Users/shaked/projects_new/agentcy/cloud/`),
which is the proven multi-tenant Railway-provisioning reference. We diverge
on three points: (a) we add a real engine-side tenancy retrofit before
launching, since kyma has none today; (b) MCP endpoint is a first-class
product surface from day one; (c) the dedicated-tenant graduation is
admin-initiated, not customer-self-serve.

---

## Architecture

**Monorepo layout** (extending the existing kyma repo at
`/Users/shaked/projects_new/agentcy/kyma`):

```
kyma/                                  # existing repo
├── crates/                            # engine
│   ├── kyma-mcp/                      # NEW — JSON-RPC MCP server (Slice 1)
│   ├── kyma-server/                   # RETROFIT — pluggable auth, tenant_id (Slice 0)
│   ├── kyma-catalog/                  # RETROFIT — tenant_id columns (Slice 0)
│   ├── kyma-storage/                  # RETROFIT — per-tenant prefix (Slice 0)
│   └── kyma-cli/                      # RETROFIT — HTTPS API mode (Slice 2/4)
├── cloud/                             # NEW — control plane
│   ├── api/                           # Hono + Drizzle + Postgres (TypeScript)
│   ├── web/                           # Next.js 15 customer frontend
│   ├── admin/                         # Next.js 15 admin dashboard
│   ├── packages/shared/               # shared types
│   ├── docker-compose.yml             # Postgres for local dev
│   └── railway.toml                   # control-plane Railway service
├── docs/                              # existing docs site at getkyma.dev
└── web/                               # existing self-hosted operator UI
```

**Separate repo:**
```
kyma-claude-skill/                     # SKILL.md manifest + README
```

**DNS topology:**

| Hostname               | Serves                              | Backend                      |
| ---------------------- | ----------------------------------- | ---------------------------- |
| `getkyma.dev`          | Marketing + docs (already live)     | VitePress static, Railway    |
| `cloud.kyma.dev`       | Customer dashboard + admin API      | `cloud/web` + `cloud/api`    |
| `admin.kyma.dev`       | Platform-admin dashboard            | `cloud/admin`                |
| `mcp.kyma.dev/<wsid>`  | Per-workspace MCP endpoint (HTTP+SSE) | `kyma-server` `/mcp/v1` route, behind Cloudflare for rate limiting |
| `app-<wsid>.kyma.dev`  | (Future, dedicated tenants)         | per-tenant `kyma-server`     |

All under the existing `getkyma.dev` apex registered at GoDaddy. Railway auto-provisions Let's Encrypt certs for each subdomain (we already proved this with `getkyma.dev` itself).

**Engine-side tenancy retrofit** (Slice 0):

- Catalog: add `tenant_id UUID NOT NULL` to `databases`, `tables`, `snapshots`, `manifests`, `extents`, `connectors`, `connector_cursors`, `dashboards`, `agent_runs`. Backfill = single tenant `00000000-0000-0000-0000-000000000000` for existing self-hosted; new column is `NOT NULL` after the backfill migration.
- Storage: object-store paths become `<KYMA_PATH_PREFIX>/<tenant_id>/extents/<extent_id>.kyma`. Keeps load-bearing for Slice 3 graduation (extent copy is just an S3 prefix copy).
- Auth: new `AuthBackend` trait in `kyma-server`. Two impls: `EnvAuthBackend` (existing `KYMA_AUTH_TOKENS` behavior, single tenant), `DbAuthBackend` (cloud only — token rows in Postgres, scoped to `tenant_id`).
- Per-tenant query cancellation/budgets: deferred to Slice 2.5; we ship Slice 0 with hard catalog/auth scoping but no metering yet.

**MCP server** (Slice 1):

- New `kyma-mcp` crate. JSON-RPC 2.0 framing over Streamable HTTP (the modern transport, supersedes SSE-only). Wire spec: https://modelcontextprotocol.io/.
- Mounted at `/mcp/v1` on `kyma-server`. Auth via `Authorization: Bearer <token>` — Slice 1a uses the existing `EnvAuthBackend`; Slice 2 swaps to `DbAuthBackend` for cloud-issued tokens.
- Wraps existing tool implementations from `crates/kyma-server/src/agent/tools.rs` (8 tools: `list_databases`, `describe_table`, `run_sql`, `run_kql`, `sample_rows`, `explore_schema`, `find_references_to`, `graph_traverse`). The MCP server is purely a JSON-RPC adapter; it does not duplicate tool logic.
- `kyma-claude-skill` repo ships a `SKILL.md` declaring the MCP server, with instructions to paste the workspace URL + bearer token at install time.

**Cloud control plane** (Slice 2):

Stack: Node 22 + TypeScript + Hono 4 + Drizzle ORM + Postgres + Resend (transactional email) + Stripe + GitHub OAuth + magic link via `lucia-auth` or `@auth/core`.

Tables (mirroring `cloud/api/src/db/schema.ts` from agentcy cloud):
- `users` — github_id, email, name, avatar_url
- `workspaces` — slug, owner_user_id, plan, kind (`shared` | `dedicated`), kyma_endpoint, mcp_endpoint, stripe_customer_id, stripe_subscription_id, plan_active, trial_ends_at, dedicated_railway_project_id (nullable)
- `workspace_members` — workspace_id, user_id, role (`owner` | `admin` | `member`)
- `api_tokens` — workspace_id, token_hash, scopes, last_used_at, revoked_at — these are *also* the MCP bearer tokens, validated by `kyma-server` `DbAuthBackend`
- `magic_links` — email, token_hash, expires_at, consumed_at
- `sessions` — user_id, token_hash, expires_at
- `billing_events` — Stripe webhook log (raw)
- `credits` — workspace_id, amount, remaining, source, expires_at
- `usage_events` — workspace_id, kind (compute_seconds, ingest_bytes, agent_calls, mcp_calls), value, occurred_at
- `usage_daily` — workspace_id, day, kind, total (hourly aggregator job)
- `platform_admin_audit` — admin_user_id, action, target, payload, occurred_at

The cloud control plane writes `api_tokens` rows; `kyma-server`'s `DbAuthBackend` reads them via direct Postgres connection (cloud-plane Postgres is shared with kyma-server's catalog Postgres for v1; can split later if hot).

Provisioning is dead simple in v1 — creating a workspace = inserting rows + minting an MCP token. No Railway provisioning, no Redis, no BullMQ. The shared kyma cluster already exists.

**Admin + dedicated graduation** (Slice 3):

Admin dashboard at `admin.kyma.dev` — GitHub OAuth gated to allowlisted GitHub user IDs (env var). Provides:
- User/workspace browsing
- Manual approve/deny if we add an approval gate later (not in v1)
- "Promote to dedicated" workflow per workspace

Graduation flow (admin-initiated):
1. Mark workspace `migrating`. `kyma-server` rejects new ingest for this workspace.
2. **Drain in-flight writes.** Wait for all uncommitted batches to flush.
3. **Catalog snapshot.** `pg_dump --table=...` filtered by `WHERE tenant_id = $X` from shared catalog. Captures `databases`, `tables`, `snapshots`, `manifests`, `extents` rows.
4. **Provision dedicated infra** via Railway GraphQL API (mirror agentcy cloud's `provisioner.ts`): new Railway project, new Postgres, new MinIO, new `kyma-server`, new `kyma-otlp` proxy.
5. **Copy extents** from `s3://shared-bucket/kyma/<tenant_id>/...` to `s3://dedicated-bucket/kyma/...`. Parallel S3 copy, resumable, immutable extents make this safe. Slow step — could be hours for a large tenant.
6. **Restore catalog** into the dedicated Postgres, rewriting object-store paths to the new bucket.
7. **Connector cursor migration.** Per-connector cursor state moves with the catalog dump. Re-attach replication slots (Postgres CDC) / change-stream tokens (Mongo) on the dedicated kyma so streaming connectors don't replay or drop events.
8. **Cutover atomically.** Update `workspaces.kind = 'dedicated'`, `kyma_endpoint`, `mcp_endpoint` in cloud control plane. Existing API tokens keep working (they identify by workspace, not cluster).
9. **Verification window**: 7-day retention of shared-cluster data with row-count + checksum diff. Then GC.

This whole flow is admin-initiated, not customer-self-serve, in v1.

**CLI retrofit** (thin slice in Slice 2, full work in Slice 4):

Slice 2 thin slice: `kyma login`, `kyma workspace [list|select]`, `kyma ingest <file>`, `kyma table list`. Talks to `cloud.kyma.dev/api/...` over HTTPS.

Slice 4 full work:
- `~/.config/kyma/profiles.toml` for multi-workspace selection
- `kyma connector [add|list|sync]` against the workspace's connector admin API
- `kyma mcp` — local stdio MCP that proxies to the workspace's HTTP MCP endpoint (lets users plug local agents into a remote workspace)
- `cargo install kyma-cli` + Homebrew tap + `curl | sh` installer
- Backwards compat: existing `--catalog-url` Postgres-direct mode kept for self-hosted users

**Stripe billing** (Slice 2):

Plan tiers in `cloud/packages/shared/src/plans.ts`:
- Free — 1 workspace, 1 GiB ingest/month, 50 MCP calls/day
- Pro — 5 workspaces, 50 GiB/month, 5K MCP calls/day, $49/month
- Team — 20 workspaces, 1 TiB/month, unlimited MCP, $299/month + per-seat
- Enterprise — dedicated cluster, custom pricing

Webhook flow:
- Stripe → `cloud/api/webhooks/stripe` — record raw event in `billing_events`, reconcile `workspaces.plan` + `plan_active` in a transaction.
- Hourly reconciler job catches dropped webhooks (a real production bug source per the plan agent's review).

Credits: `credits` table mirrors agentcy cloud's design. Earned via promo (signup credit), purchased (in-app top-up), or refunded (failed dedicated provisioning). Consumed when usage exceeds plan limits.

---

## Slice plan

The plan agent's reorder is what we ship. Slices 0 and 1a run in parallel.

| Slice | Scope | Effort | Status |
|-------|-------|-------:|--------|
| **0**  | Engine prep: tenant_id retrofit + storage prefix + pluggable AuthBackend | 3w | gates Slice 2 GA |
| **1a** | MCP server crate + skill repo, self-hosted token model | 1.5w | parallel with Slice 0 |
| **2**  | Cloud control plane v1: shared tenant, GitHub+magic-link, Stripe, MCP token issuance, thin CLI | 5–6w | first paying customer |
| **3**  | Admin + dedicated graduation (snapshot + extent-copy + atomic cutover) | 3w | enterprise tier |
| **4**  | Full CLI retrofit (profiles, brew, stdio MCP) | 2w | DX polish |
| **5**  | Marketing landing for `cloud.kyma.dev`, onboarding polish, sample data, "60-second demo" flow | 1.5w | launch |

**Total: ~16-17 weeks**, single engineer with Claude pair. First paying customer end of Slice 2 (~week 9–10).

---

## Critical files to modify or create

**Engine retrofit (Slice 0):**
- `crates/kyma-catalog/migrations/0011_tenant_id.sql` — additive migration adding `tenant_id` to all relevant tables, backfill to default UUID, then NOT NULL.
- `crates/kyma-catalog/src/lib.rs` — every `WHERE name = ?` lookup becomes `WHERE tenant_id = ? AND name = ?`. This is the long pole.
- `crates/kyma-storage/src/lib.rs` — object-store path templates updated to include `<tenant_id>` segment.
- `crates/kyma-server/src/auth.rs` — refactor into `AuthBackend` trait + `EnvAuthBackend` impl. New `DbAuthBackend` lives in `kyma-server` but only enabled via Cargo feature `cloud-auth`.
- `crates/kyma-server/src/lib.rs` — middleware extracts `tenant_id` from validated token; injects into request extensions; downstream handlers take it.

**MCP server (Slice 1a):**
- `crates/kyma-mcp/Cargo.toml` — new crate, depends on `kyma-server::tools`.
- `crates/kyma-mcp/src/lib.rs` — JSON-RPC 2.0 framing, Streamable HTTP transport, tool dispatch table.
- `crates/kyma-mcp/src/tools.rs` — thin wrappers around `crates/kyma-server/src/agent/tools.rs` (the 8 existing tools).
- `crates/kyma-server/src/lib.rs` — mount `kyma_mcp::router()` at `/mcp/v1`.

**Claude skill (Slice 1a):**
- `kyma-claude-skill/SKILL.md` — manifest declaring the MCP server, install instructions.
- `kyma-claude-skill/README.md` — user-facing setup guide.

**Cloud control plane (Slice 2):**

Files to create. Heavily cribbed from agentcy cloud — copy structure, retype as needed:
- `cloud/api/src/db/schema.ts` — Drizzle schema (mirror `/Users/shaked/projects_new/agentcy/cloud/api/src/db/schema.ts`).
- `cloud/api/src/services/auth.service.ts` — GitHub OAuth + magic link, JWT minting (mirror agentcy `auth.service.ts` minus email/password).
- `cloud/api/src/services/stripe.service.ts` — plan management, webhook handling.
- `cloud/api/src/services/workspace.service.ts` — workspace CRUD, MCP token minting.
- `cloud/api/src/routes/auth.ts`, `cloud/api/src/routes/workspaces.ts`, `cloud/api/src/routes/billing.ts`, `cloud/api/src/routes/webhooks.ts`, `cloud/api/src/routes/usage.ts`.
- `cloud/web/src/app/(auth)/login/page.tsx` — GitHub button + magic link form.
- `cloud/web/src/app/(dashboard)/workspaces/page.tsx` — workspace list.
- `cloud/web/src/app/(dashboard)/workspaces/[slug]/page.tsx` — workspace detail with MCP setup, connector list, usage chart, billing.
- `cloud/web/src/app/(dashboard)/billing/page.tsx` — plan selector + Stripe customer portal link.
- `cloud/admin/src/app/page.tsx` — admin dashboard (GitHub-OAuth gated).
- `cloud/admin/src/app/workspaces/[id]/page.tsx` — workspace inspector + "promote to dedicated" button (Slice 3 wiring).
- `cloud/packages/shared/src/types.ts`, `cloud/packages/shared/src/plans.ts`.
- `cloud/Dockerfile.api`, `cloud/Dockerfile.web`, `cloud/Dockerfile.admin`, `cloud/railway.toml`.
- `cloud/docker-compose.yml` — local Postgres for dev.

**CLI retrofit (Slice 2 thin slice + Slice 4):**
- `crates/kyma-cli/src/main.rs` — clap subcommand tree.
- `crates/kyma-cli/src/api_client.rs` — HTTPS client for `cloud.kyma.dev/api`.
- `crates/kyma-cli/src/profile.rs` — `~/.config/kyma/profiles.toml` handling.
- `crates/kyma-cli/src/commands/login.rs`, `ingest.rs`, `connector.rs`, `mcp.rs` (Slice 4).

**DNS (handled live, not via PR):**
- GoDaddy DNS for `getkyma.dev` (already configured for the docs site)
- Add CNAMEs: `cloud.kyma.dev` → Railway, `admin.kyma.dev` → Railway, `mcp.kyma.dev` → Cloudflare → Railway
- Cloudflare in front of `mcp.kyma.dev` for rate limiting per bearer token

---

## Existing code to reuse

- **Agentcy cloud schema and provisioner** (`/Users/shaked/projects_new/agentcy/cloud/api/src/db/schema.ts`, `services/provisioner.ts`, `services/railway.service.ts`). Direct copy targets for Slice 3's Railway provisioning.
- **Agentcy cloud frontend shell** (`/Users/shaked/projects_new/agentcy/cloud/web/src/components/`). Tailwind 4.1 + custom UI primitives — copy the design tokens and component shells, replace the brand identity.
- **kyma agent tools** (`crates/kyma-server/src/agent/tools.rs`) — the 8 tool implementations are the entire MCP surface.
- **kyma docs site identity** (`docs/site/.vitepress/theme/`) — JetBrains Mono + IBM Plex Sans + phosphor-green palette + the kyma K-mark logo. The cloud frontend should use the *same* design tokens for visual continuity between docs and dashboard.
- **GoDaddy DNS automation pattern** — we already used the GoDaddy DNS UI via Claude-in-Chrome for `getkyma.dev`. Same approach for `cloud.kyma.dev` and `mcp.kyma.dev`.

---

## Verification

Per slice. Each slice has a hard gate — no ship until green.

**Slice 0 verification:**
- All catalog tests green (`cargo test -p kyma-catalog`).
- New integration test: insert two databases with same name under different `tenant_id`s; assert lookups never cross.
- `cargo test -p kyma-server --features cloud-auth` exercises both `EnvAuthBackend` and `DbAuthBackend`.
- Smoke: deploy retrofitted kyma to a fresh Railway project, run the existing docs-site smoke (every page returns 200, sample queries work) — proves no regression for self-hosted.

**Slice 1a verification:**
- MCP wire-protocol conformance: install `kyma-claude-skill` in a local Claude Code, run a workspace query end-to-end. Specifically `list_databases`, `run_kql "otel_logs | take 5"`, and `graph_traverse` (the marquee tool).
- `cargo test -p kyma-mcp` covers JSON-RPC framing edge cases (parse errors, version mismatches, batch requests).
- Manual: connect from Cursor too — proves MCP isn't Claude-specific.

**Slice 2 verification:**
- Sign up via GitHub OAuth on `cloud.kyma.dev`. Get a workspace. Mint an MCP token. Install the Claude skill. Ask a question. Get an answer. End-to-end in <60 seconds — that's the demo.
- Sign up via magic link from a fresh email. Same flow.
- Stripe Checkout for Pro plan. Webhook updates `workspaces.plan`. Confirm via dashboard.
- Stripe billing portal works (cancel, change plan, update card).
- `kyma login` from a fresh terminal authenticates and stores credentials.
- `kyma ingest sample.ndjson` lands data in the workspace; `kyma table list` shows it.
- Browser-driven test of cross-tenant isolation: workspace A's token cannot read workspace B's data via either MCP or HTTP API.
- Stripe webhook reconciler job runs hourly and catches any dropped event.

**Slice 3 verification:**
- Promote a test workspace from shared to dedicated end-to-end. Time the cutover. Verify zero data loss via row-count diff.
- Connector cursor continuity: a Postgres connector mid-stream during graduation continues without replaying or dropping events on the dedicated cluster.
- Admin audit log captures every promotion action.

**Slice 4 verification:**
- `cargo install kyma-cli` from a fresh machine works.
- `kyma mcp` starts a local stdio server that proxies to the cloud workspace; Claude Desktop can connect to it.
- `kyma connector add postgres --workspace prod` creates a Postgres connector against the cloud workspace's connector admin API.

**Slice 5 verification:**
- Lighthouse score on `cloud.kyma.dev` landing ≥ 90 on Performance + Accessibility + SEO.
- New-user funnel measured end-to-end: time from "click Get Started on getkyma.dev" to "first answer from MCP-connected agent" averages under 60s across 10 runs.

---

## Open decisions to lock in during execution (not blockers)

These can be answered when the relevant slice starts:

1. Magic-link library: `lucia-auth` vs `@auth/core` vs hand-rolled. Lock in at Slice 2 kickoff.
2. Per-tenant connector secret encryption: per-workspace KMS keys (recommended by plan agent) vs single platform key. Lock in at Slice 0 kickoff since the schema reflects this choice.
3. Cloudflare account: existing AgentcyLabs account or new dedicated kyma account. Locks in MCP rate-limit ownership.
4. Stripe webhook signature secret: hard-coded vs Stripe's `2024-...` API version pinning. Standard Stripe operational decision; plan agent flagged this as a real source of bugs.
