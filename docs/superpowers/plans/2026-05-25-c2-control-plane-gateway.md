# C2 — Control Plane + Edge Implementation Plan (REVISED)

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **REVISION 2026-05-25:** Original C2 built a Rust `kyma-gateway` that did API-key auth, tenant resolution, and X-Database injection. That was largely **already built in the engine** — branch `feature/cloud-slice-0-engine-tenancy` adds a pluggable `AuthBackend` with a `DbAuthBackend` that reads a shared Postgres `api_tokens` table, resolves token → `tenant_id` → `Role`, and the engine already segments catalog rows and S3 extent paths by `tenant_id` (migration 007 + `TelemetryFormat::with_tenant`). So the engine authenticates and isolates tenants itself. This revision: (1) the **control plane writes `api_tokens`** (the engine reads them); (2) the engine runs with the `cloud-auth` feature; (3) the "gateway" collapses to a thin **edge** doing only what the engine does NOT — public TLS/routing, per-tenant **rate-limit + usage metering**, and **optional scoped-STS** as storage defense-in-depth. Aligns to the pre-existing master spec `docs/superpowers/specs/2026-05-02-kyma-cloud-platform-design.md` (Slice 0 + Slice 2).

**Goal:** Stand up the cloud control plane (`cloud/api`) that owns workspaces, users, and `api_tokens`, deploy the engine with `cloud-auth` so those tokens authenticate and resolve to tenants natively, and add a thin edge for per-tenant rate-limiting + usage metering (and optional scoped-STS). After C2, a workspace can be created, a token minted, and a request bearing that token reaches the engine, authenticates via `DbAuthBackend`, and is hard-isolated to that workspace's `tenant_id` at catalog + storage layers — provably unable to read another workspace's data.

**Architecture:** `cloud/api` (TypeScript + Hono + Drizzle + Postgres, mirroring the existing spec's stack and the agentcy-cloud reference) owns the control-plane schema — `users`, `workspaces`, `workspace_members`, `api_tokens`, `usage_events`, plus Stripe tables (C4). It writes `api_tokens` rows (`token_hash = SHA-256(token)`, `tenant_id`, `scopes`). The engine, built with cargo feature `cloud-auth`, runs `DbAuthBackend` pointed at that same Postgres; every request's bearer token resolves to a `Principal { tenant, role }`, and the existing tenant-aware catalog + tenant-segmented extent paths enforce isolation. The **edge** (Cloudflare in production per the existing spec; a thin proxy in dev) does only cross-cutting concerns the engine doesn't: rate-limit per token/workspace and capture `usage_events`. **Optional:** mint per-request scoped STS creds for S3 as defense-in-depth — kept as an opt-in hardening step, since the engine already prefix-isolates by `tenant_id`.

**Tech Stack:** `cloud/api`: Node 22 + TypeScript + Hono 4 + Drizzle ORM + Postgres 16 (the control-plane DB; the existing spec shares it with the engine catalog Postgres for v1, splittable later). Engine: existing Rust build with `--features cloud-auth`. Edge: Cloudflare (prod) / lightweight proxy (dev). Verification: bash + curl + the engine's own `tenant_isolation_it` + `auth_backends_it` tests as the isolation guarantee, plus a cross-workspace denial script.

**Prerequisites:**
- C1 complete with PROCEED verdict.
- Engine image built from a commit including Slice 0 (`feature/cloud-slice-0-engine-tenancy`: migration 007, `AuthBackend`/`DbAuthBackend`, tenant-segmented paths). **Task 0 confirms this.**

**Engine-dependency note (resolved, not assumed):** Slice 0 IS the "A4" the original C2 hedged about. It exists on a branch with a passing cross-tenant isolation gate test. C2 therefore *consumes* it. Task 0 verifies it's in the deployed build; the rest of C2 builds the control plane that feeds it `api_tokens` and the edge that meters it.

**File Structure (created across this plan):**

```
cloud/api/                                  # control plane (Hono + Drizzle)
  src/db/schema.ts                          users, workspaces, members, api_tokens, usage_events
  src/db/migrate.ts
  src/routes/workspaces.ts                  workspace CRUD + provisioning
  src/routes/tokens.ts                      token mint + revoke (writes api_tokens)
  src/lib/tokens.ts                         token gen + SHA-256 hash (matches DbAuthBackend)
  src/lib/provision.ts                      transactional workspace provisioning
  src/server.ts
  test/provision.test.ts  test/tokens.test.ts
cloud/edge/                                 # thin edge (dev); Cloudflare config (prod)
  src/rate_limit.ts  src/meter.ts  src/proxy.ts
  src/sts.ts                                OPTIONAL scoped-STS hardening
infra/envs/dev/main.tf                      + control-plane Postgres, engine cloud-auth env,
                                            edge service; - C1 static IAM user (if STS used)
scripts/cloud/
  c2-provision-workspace.sh                 create workspace + token via cloud/api
  c2-isolation-test.sh                      prove workspace A cannot read workspace B
docs/cloud/
  c2-isolation-report.md                    THE DELIVERABLE: isolation evidence + threat model
```

---

## Task 0: Confirm Slice 0 in the deployed engine; lock the api_tokens contract

**Objective:** Verify the engine already does tenant auth + isolation, and pin the exact `api_tokens` shape the control plane must write.

**Files:**
- Create: `docs/cloud/c2-engine-contract.md`

- [ ] **Step 1: Read the engine's `DbAuthBackend` contract** (source of truth for the table shape)

Run:
```bash
git show feature/cloud-slice-0-engine-tenancy:crates/kyma-server/src/auth/db_backend.rs | sed -n '1,40p'
git show feature/cloud-slice-0-engine-tenancy:crates/kyma-catalog/migrations/007_tenant_id.sql | head -60
```
Record the exact `api_tokens` columns the engine expects: `id, tenant_id (uuid), token_hash (bytea = SHA-256(token)), scopes (csv: admin|write|read), subject, last_used_at, revoked_at, created_at`.

- [ ] **Step 2: Confirm the engine build flags** — engine must run with `--features cloud-auth` and `DbAuthBackend` pointed at the control-plane Postgres. Confirm `kyma-bin` selects the db backend from env (commit `40887172`).

- [ ] **Step 3: Write `docs/cloud/c2-engine-contract.md`** — the frozen `api_tokens` schema, the hashing rule (raw SHA-256, ≥128-bit CSPRNG token), the `tenant_id` semantics, and the statement: "control plane writes; engine reads; never diverge this schema."

- [ ] **Step 4: Commit**

```bash
git add docs/cloud/c2-engine-contract.md
git commit -m "docs(c2): freeze api_tokens contract shared with engine DbAuthBackend"
```

---

## Task 1: Control-plane Postgres + schema (Drizzle)

**Objective:** The control-plane DB with the workspace/token model, matching the existing spec and the engine's `api_tokens` contract.

**Files:**
- Modify: `infra/envs/dev/main.tf` (control-plane Postgres — Supabase A or the shared catalog Postgres per existing spec's v1 choice)
- Create: `cloud/api/src/db/schema.ts`

- [ ] **Step 1: Decide the DB topology** — the existing spec shares the control-plane Postgres with the engine catalog Postgres for v1 (so `DbAuthBackend` reads `api_tokens` via a direct connection, no cross-DB hop). Record this in `c2-engine-contract.md`; it simplifies the `DbAuthBackend` wiring. (My original two-Supabase split remains the post-v1 option if it gets hot.)

- [ ] **Step 2: Write `cloud/api/src/db/schema.ts`** (Drizzle) with: `users`, `workspaces` (slug, owner_user_id, plan, kind `shared|dedicated`, tenant_id, kyma_endpoint, mcp_endpoint, stripe_* nullable), `workspace_members`, **`api_tokens` exactly matching the engine contract from Task 0**, `usage_events` (workspace_id, kind, value, occurred_at). Forward-only migrations.

- [ ] **Step 3: Apply + verify**

Run:
```bash
pnpm -C cloud/api db:push
psql "$CONTROL_PLANE_DB_URL" -c "\d api_tokens"
```
Expected: `api_tokens` columns match the engine's `DbAuthBackend` exactly (token_hash bytea, tenant_id uuid, scopes text, revoked_at).

- [ ] **Step 4: Commit**

```bash
git add cloud/api/src/db/schema.ts infra/envs/dev/main.tf
git commit -m "feat(cloud/api): control-plane schema; api_tokens matches engine contract"
```

---

## Task 2: Token mint + hash (must match DbAuthBackend exactly)

**Objective:** Generate ≥128-bit tokens and store `SHA-256(token)` as the engine expects.

**Files:**
- Create: `cloud/api/src/lib/tokens.ts`, `cloud/api/test/tokens.test.ts`

- [ ] **Step 1: Failing test** — `generateToken()` yields a ≥128-bit token; `hashToken(t)` equals SHA-256 bytes; a token hashed here matches what the engine's `hash_token` would compute (assert against a known vector copied from `db_backend.rs`).

- [ ] **Step 2: Run — expect failure.**

- [ ] **Step 3: Implement `tokens.ts`** — `randomBytes(32)` → base64url token with a `kyma_` prefix; `hashToken` = `createHash("sha256").update(token).digest()` returning bytes for the `bytea` column. **Critical:** the hash must be byte-identical to the engine's (raw SHA-256 of the presented bearer string, no salt) or auth silently fails.

- [ ] **Step 4: Run — expect pass.**

- [ ] **Step 5: Commit**

```bash
git add cloud/api/src/lib/tokens.ts cloud/api/test/tokens.test.ts
git commit -m "feat(cloud/api): token mint + SHA-256 hash matching engine DbAuthBackend"
```

---

## Task 3: Transactional workspace provisioning

**Objective:** Create user/workspace + allocate `tenant_id`, atomically and idempotently. **No Terraform, no AWS resource per signup** (existing spec: "creating a workspace = inserting rows + minting a token").

**Files:**
- Create: `cloud/api/src/lib/provision.ts`, `cloud/api/test/provision.test.ts`

- [ ] **Step 1: Failing test** — `provisionWorkspace({userId, name, idempotencyKey})` inserts a workspace with a fresh `tenant_id` (uuid), `plan=free`, `kind=shared`; a retry with the same key returns the same workspace; distinct names get distinct tenant_ids.

- [ ] **Step 2: Run — expect failure.**

- [ ] **Step 3: Implement `provision.ts`** — single transaction: insert workspace (uuid `tenant_id`), `workspace_members` (owner), set `kyma_endpoint` + `mcp_endpoint`. Idempotency via an `idempotency_keys` table. The engine needs no notification — it discovers the tenant lazily when the first token authenticates and the first ingest auto-creates the database under that `tenant_id`.

- [ ] **Step 4: Run — expect pass.**

- [ ] **Step 5: Commit**

```bash
git add cloud/api/src/lib/provision.ts cloud/api/test/provision.test.ts
git commit -m "feat(cloud/api): transactional idempotent workspace provisioning"
```

---

## Task 4: cloud/api routes + deploy

**Objective:** Expose workspace + token operations over HTTP; deploy to Railway.

**Files:**
- Create: `cloud/api/src/routes/workspaces.ts`, `src/routes/tokens.ts`, `src/server.ts`
- Modify: `infra/envs/dev/main.tf` (cloud/api Railway service)

- [ ] **Step 1: `POST /workspaces`** (auth: user session) → `provisionWorkspace`, returns workspace + endpoints. **`POST /workspaces/:id/tokens`** → mint token, insert `api_tokens` row (hash + tenant_id + scopes), return the full token **once**. **`DELETE /tokens/:id`** → set `revoked_at`.

- [ ] **Step 2: `server.ts`** mounts routes; verifies user session (GitHub OAuth / magic link per existing spec); `/health`.

- [ ] **Step 3: Deploy** `cloud/api` Railway service (env: `CONTROL_PLANE_DB_URL`). Apply via CI.

- [ ] **Step 4: Verify**

Run:
```bash
API="https://$(terraform -chdir=infra/envs/dev output -raw cloud_api_url)"
curl -fsS "$API/health"
# (with a session) create workspace + mint token, capture the token once
```
Expected: workspace JSON with endpoints; token minted; an `api_tokens` row exists.

- [ ] **Step 5: Commit**

```bash
git add cloud/api/src/routes cloud/api/src/server.ts infra/envs/dev/main.tf
git commit -m "feat(cloud/api): workspace + token routes; deploy to railway"
```

---

## Task 5: Point the engine at cloud-auth; verify token → tenant

**Objective:** The engine authenticates control-plane-issued tokens and isolates by tenant — using its own `DbAuthBackend`, no gateway auth code.

**Files:**
- Modify: `infra/envs/dev/main.tf` (engine env: enable cloud-auth, point at control-plane Postgres)

- [ ] **Step 1: Rebuild/redeploy the engine with `--features cloud-auth`** and env selecting `DbAuthBackend` against `CONTROL_PLANE_DB_URL`. Remove the C1 single shared `KYMA_AUTH_TOKENS` env (real per-tenant auth replaces it).

- [ ] **Step 2: Verify token → tenant end to end**

Run:
```bash
ENGINE="https://$(terraform -chdir=infra/envs/dev output -raw engine_url)"
# ingest + query using the workspace token (build auth header from $WS_TOKEN at call time)
scripts/cloud/c2-provision-workspace.sh   # prints WS_TOKEN
ENGINE="$ENGINE" WS_TOKEN=*** scripts/cloud/c2-deploy-check.sh
```
Expected: ingest + query succeed under the workspace's `tenant_id`; the row lands under that tenant's S3 prefix (`<prefix>/<tenant_id>/...`).

- [ ] **Step 3: Commit**

```bash
git add infra/envs/dev/main.tf scripts/cloud/c2-provision-workspace.sh
git commit -m "feat(infra): engine runs cloud-auth (DbAuthBackend) against control-plane tokens"
```

---

## Task 6: Edge — rate-limit + usage metering

**Objective:** Add the cross-cutting concerns the engine does NOT do: per-tenant rate-limit and usage capture. (Auth + tenant + X-Database are the engine's job now.)

**Files:**
- Create: `cloud/edge/src/rate_limit.ts`, `src/meter.ts`, `src/proxy.ts`
- Modify: `infra/envs/dev/main.tf` (edge service / Cloudflare config)

- [ ] **Step 1: Rate-limit** — per-token/workspace token-bucket keyed on the bearer token prefix; over-limit → 429. In production this is Cloudflare rate-limiting rules per the existing spec; in dev, a thin proxy.

- [ ] **Step 2: Metering** — capture `usage_events` (`ingest_bytes`, `mcp_calls`, `query_*`) from request/response sizes + engine response headers (`X-Kyma-Rows`); async batch-insert into the control-plane `usage_events`. Never block or fail a customer request on a metering write; log drops.

- [ ] **Step 3: Deploy edge** in front of the engine; route `/v1/*` and `/mcp/v1`. Verify a flood trips 429 and `usage_events` rows accrue.

- [ ] **Step 4: Commit**

```bash
git add cloud/edge infra/envs/dev/main.tf
git commit -m "feat(cloud/edge): per-tenant rate-limit + usage metering"
```

---

## Task 7 (OPTIONAL): Scoped-STS storage hardening

**Objective:** Defense-in-depth — even though the engine prefix-isolates by `tenant_id`, optionally mint per-request STS creds scoped to the tenant prefix so a storage-layer bug can't cross tenants. Skip if the engine's single shared S3 credential + tenant-path isolation is deemed sufficient for v1.

**Files:**
- Create: `cloud/edge/src/sts.ts`
- Modify: `infra/envs/dev/main.tf` (wire `iam-scoped-role` from C0; remove C1 static IAM user)

- [ ] **Step 1: Decide** — record in `c2-isolation-report.md` whether v1 uses (a) engine shared cred + tenant-path isolation (simpler, matches existing spec), or (b) per-request scoped STS (stronger, my original design). Default to (a) for v1 unless the threat model demands (b).

- [ ] **Step 2 (if b):** implement `session_policy_for(tenant_id, bucket)` pinning S3 to `<bucket>/<tenant_id>/*`; mint via `AssumeRole` on the C0 `iam-scoped-role`; deliver per-request creds to the engine. Add the direct-S3 cross-prefix `AccessDenied` test to Task 8.

- [ ] **Step 3: Commit** (only if implemented)

```bash
git add cloud/edge/src/sts.ts infra/envs/dev/main.tf
git commit -m "feat(cloud/edge): optional per-request scoped-STS storage hardening"
```

---

## Task 8: Cross-workspace isolation test (the deliverable)

**Objective:** Prove workspace A cannot read workspace B's data — leveraging the engine's own isolation, verified end to end.

**Files:**
- Create: `scripts/cloud/c2-isolation-test.sh`, `docs/cloud/c2-isolation-report.md`

- [ ] **Step 1: Write `scripts/cloud/c2-isolation-test.sh`** — provision workspaces A and B; mint a token each; ingest distinct sentinels; assert A's token returns only A's rows and never B's (and vice-versa), via both `/v1/query` and `/mcp/v1`. If Task 7(b) was implemented, also assert A's STS creds get `AccessDenied` on B's S3 prefix.

- [ ] **Step 2: Run the engine's own isolation tests as the foundation**

Run:
```bash
cargo test -p kyma-catalog tenant_isolation
cargo test -p kyma-server auth_backends
GW_OR_ENGINE_URL=... API=... bash scripts/cloud/c2-isolation-test.sh
```
Expected: engine gate tests pass; end-to-end cross-workspace denial holds.

- [ ] **Step 3: Write `docs/cloud/c2-isolation-report.md`**

````markdown
# C2 — Tenant Isolation Report

**Date:** <fill>  **Engine build:** <commit incl. Slice 0>

## Isolation boundaries
1. **Engine `DbAuthBackend`** resolves token -> tenant_id -> Role (no client-controlled tenant).
2. **Catalog** rows scoped by tenant_id (migration 007; unique constraints are (tenant_id, name)).
3. **Storage** extent paths carry `<tenant_id>/` segment (TelemetryFormat::with_tenant).
4. **(Optional) Scoped STS** pins S3 to <bucket>/<tenant_id>/* — defense in depth, if Task 7(b).

## Evidence
| Test | Layer | Result |
|---|---|---|
| Engine `tenant_isolation_it` | catalog | |
| Engine `auth_backends_it` | auth | |
| A token returns only A rows (/v1/query) | query | |
| A token returns only A rows (/mcp/v1) | mcp | |
| A STS creds denied on B prefix (if Task 7b) | storage | |
| Rate-limit breach -> 429 | edge | |

## Known limitations (honest)
- Shared engine process holds multiple tenants' data in memory; isolation is at
  catalog/storage/auth, not process memory. Hard isolation = dedicated cluster
  (Slice 3 graduation / C6).
- If Task 7(a) chosen: a storage-layer bug could theoretically cross prefixes
  since the engine uses one S3 credential. Mitigated by tenant-path isolation +
  the engine's gate test; (b) closes this fully.

## Verdict
<PROCEED to C3 / blockers>
````

- [ ] **Step 4: Commit**

```bash
git add scripts/cloud/c2-isolation-test.sh docs/cloud/c2-isolation-report.md
git commit -m "test(c2): cross-workspace isolation evidence + report"
```

---

## Task 9: Phase exit checklist

- [ ] `cloud/api` deployed: creates workspaces, allocates tenant_id, mints `api_tokens` (hashed to engine spec).
- [ ] Provisioning transactional + idempotent; no Terraform/AWS resource per signup.
- [ ] Engine runs `cloud-auth` (`DbAuthBackend`) against control-plane tokens; token → tenant verified.
- [ ] Edge does rate-limit + usage metering (NOT auth/tenant — that's the engine).
- [ ] Cross-workspace isolation proven via engine gate tests + end-to-end script; report written.
- [ ] `usage_events` populated (feeds C4 billing).
- [ ] STS hardening decision recorded (a or b).

- [ ] **Mark C2 complete in the master design**

```bash
git add docs/superpowers/specs/2026-05-25-kyma-cloud-platform-design.md
git commit -m "docs(cloud): mark C2 complete (consumes engine DbAuthBackend + tenancy)"
```

---

## Notes for the implementer

- **The engine authenticates and isolates tenants — not a gateway.** Slice 0 shipped `DbAuthBackend` + tenant-scoped catalog + tenant-segmented storage. C2 does NOT reimplement auth/tenant resolution. If you write token-to-tenant logic in the edge, stop — the engine already does it.
- **`api_tokens` hashing must be byte-identical to the engine.** Raw SHA-256 of the presented token, no salt (the engine's `db_backend.rs` documents why: server-issued, ≥128-bit tokens). A mismatch fails auth silently. Test against a vector copied from the engine source.
- **Share the Postgres for v1.** The existing spec shares control-plane Postgres with the engine catalog so `DbAuthBackend` reads `api_tokens` with no cross-DB hop. Don't split into two databases until it's measurably hot.
- **The edge is thin and optional-heavy.** Its only must-haves are rate-limit + metering. Auth, tenant, X-Database, storage isolation are all the engine's. Scoped-STS is opt-in hardening, not a v1 requirement.
- **Names match the existing spec.** `workspaces` (not "projects"), `api_tokens` (not "api_keys"), `tenant_id` (uuid). Aligning to `2026-05-02-kyma-cloud-platform-design.md` keeps one architecture, not two.
- **Metering is best-effort but auditable.** Never fail a customer request on a `usage_events` write; buffer + batch; log drops (C4 billing integrity depends on it).
