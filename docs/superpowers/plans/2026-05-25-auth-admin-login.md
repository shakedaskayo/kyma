# Proper Auth: Admin-User Login (OIDC later) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Replace the "paste a bearer token in Settings" UX with a real **login screen** backed by a **users table + an admin user**. Login issues a **session token** that the existing bearer-token middleware validates unchanged. OIDC (Okta/Keycloak) is a later phase — design the seam.

**Architecture (reuse, don't rebuild):** Kyma already has `AuthBackend`/`Principal`/`Role(Read<Write<Admin)` + bearer middleware (`crates/kyma-server/src/auth/`). A login = INSERT an `api_tokens` row (random ≥128-bit token; store `SHA-256(token)`); subsequent requests authenticate via the same SHA-256 lookup. Add: a `users` table (argon2 password), an `api_tokens` migration (it only exists as a doc-comment/test fixture today), a **`SessionAuthBackend`** that validates `api_tokens` *and* falls back to env tokens (so static `KYMA_AUTH_TOKENS` API tokens keep working), unauthenticated `POST /v1/auth/login`, authenticated `/v1/auth/me` + `/logout`, admin seeding from env, and a web `/login` route. Passwords use **argon2** (db_backend.rs explicitly warns SHA-256 is only for high-entropy tokens, not passwords).

**Scope:** v1 = a single seeded admin user + username/password login. Multi-user management UI + OIDC are later phases (seams designed). Connectors work is paused (P1-engine-A committed).

**Reference (verified):** `crates/kyma-server/src/auth/{backend,db_backend,env_backend,middleware,mod}.rs`; `crates/kyma-bin/src/main.rs` (backend selection ~148-178, router merge ~369-401); migration pattern `crates/kyma-catalog/migrations/006_dashboards.sql`/`008_graphs.sql` + `Catalog` trait `*_in_tenant` pattern (`crates/kyma-core/src/catalog.rs`); web guard `web/src/routes/_app.tsx`, top-level `settings.tsx`, `web/src/sdk/session.ts`.

**Working dir:** worktree `…/.claude/worktrees/feature+graph-layer`. Docker for tests.

---

## Phasing
- **A1 — engine**: `009_auth.sql` (users + api_tokens), Catalog user/session methods, argon2, login/logout/me routes (login unauthenticated), `SessionAuthBackend` (db sessions + env fallback), admin seeding, main wiring.
- **A2 — web**: `/login` route (server URL + username + password), session store gains `user{username,role}`, guard → `/login`, Settings becomes account (logged-in-as + logout + server info), paste-token demoted to an "API token (advanced)" option.
- **A3 — later (seams only)**: user management UI + roles; OIDC (`/v1/auth/oidc/start|callback` → issue the same session) + Settings OIDC provider config.

---

## A1 — Engine

### Migration `crates/kyma-catalog/migrations/009_auth.sql`
```sql
CREATE TABLE users (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id     uuid NOT NULL,
  username      text NOT NULL,
  password_hash text NOT NULL,          -- argon2 PHC string
  role          text NOT NULL,          -- 'admin' | 'write' | 'read'
  created_at    timestamptz NOT NULL DEFAULT now(),
  updated_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (tenant_id, username)
);
CREATE INDEX users_tenant_idx ON users (tenant_id);

-- api_tokens: documented in db_backend.rs but never migrated. Add it now;
-- both login sessions AND static API tokens live here.
CREATE TABLE api_tokens (
  id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tenant_id     uuid NOT NULL,
  token_hash    bytea NOT NULL UNIQUE,  -- SHA-256(presented token)
  scopes        text NOT NULL,          -- 'admin'|'write'|'read'
  subject       text,                   -- user id/username or token label
  kind          text NOT NULL DEFAULT 'session',  -- 'session' | 'api'
  expires_at    timestamptz,
  last_used_at  timestamptz,
  revoked_at    timestamptz,
  created_at    timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX api_tokens_tenant_idx ON api_tokens (tenant_id);
```

### Catalog (`kyma-core/src/catalog.rs` trait + `kyma-catalog/src/lib.rs` impl)
Mirror the dashboards/graphs `*_in_tenant` pattern. Add types `User { id, username, role, ... }`, `SessionToken`. Methods:
- `create_user_in_tenant(tenant, username, password_hash, role) -> User`
- `get_user_by_username_in_tenant(tenant, username) -> Option<UserWithHash>` (returns the hash for verification — keep `password_hash` off the public `User` type; a separate internal struct)
- `count_users_in_tenant(tenant) -> usize` (for "is auth configured / seed needed")
- `insert_api_token_in_tenant(tenant, token_hash: &[u8], scopes, subject, kind, expires_at) -> ()`
- `lookup_api_token(token_hash) -> Option<TokenPrincipal { tenant, role, subject, expired }>` (the read path the backend uses; honors `revoked_at`/`expires_at`)
- `revoke_api_token(token_hash) -> bool`
(Default-tenant wrappers delegate to `_in_tenant`, per convention.)

### Password + token crypto
- Add `argon2` + `rand` to `crates/kyma-server` (or a small `kyma-server/src/auth/passwords.rs`): `hash_password(pw) -> String` (argon2id PHC), `verify_password(pw, phc) -> bool`.
- Session token: 32 random bytes → base64url (≥128-bit). Store `Sha256::digest(token)` (reuse db_backend's `hash_token`). Default session expiry e.g. 30 days (`expires_at`).

### `SessionAuthBackend` (`crates/kyma-server/src/auth/session_backend.rs`, NOT feature-gated)
`struct SessionAuthBackend { pool: PgPool, env: EnvAuthBackend }`. `authenticate(token)`:
1. Try `env.authenticate(token)` (in-memory `KYMA_AUTH_TOKENS`) → if Ok, return (preserves static API tokens).
2. Else `catalog.lookup_api_token(SHA256(token))` → if found, not revoked, not expired → `Principal { tenant, role, subject }`; else `UnknownToken`.
`enabled()` = `env.enabled() || users_exist` (cache a bool at startup, or always-true once login is on). Honors the existing auth-disabled passthrough when neither users nor env tokens exist.

### Auth routes (`crates/kyma-server/src/auth_handler.rs`)
- **Unauthenticated** `auth_login_router(catalog)`:
  - `POST /v1/auth/login {username, password}` → look up user, `verify_password`, on success `insert_api_token(kind:"session", scopes:role, subject:username, expires_at)` → `{ token, user:{username, role}, expires_at }`. Rate-limit/constant-time-ish; generic error on failure (no user-enumeration).
- **Authenticated** routes (mounted on a `Role::Read`-wrapped router, so the session token is validated):
  - `GET /v1/auth/me` → from the request `Principal` extension, return `{ username:subject, role }`.
  - `POST /v1/auth/logout` → `revoke_api_token(SHA256(presented token))`.
- Mount: `app.merge(auth_login_router)` **without** the auth layer (sibling to `health_router` at main.rs router-merge); mount me/logout on an authenticated router.

### main.rs wiring (`crates/kyma-bin/src/main.rs`)
- After catalog connect: **seed admin** — if `KYMA_ADMIN_USER` + `KYMA_ADMIN_PASSWORD` set and `count_users()==0`, `create_user(admin, argon2(pw), Admin)`. (Log a one-time generated password if only `KYMA_ADMIN_USER` set — or require both; pick: require both, else skip seeding.)
- **Backend selection**: when users exist or `KYMA_AUTH_BACKEND=session`, use `SessionAuthBackend::new(pool, EnvAuthBackend::from_env())`. Keep env-only path for pure token deployments. (Self-hosted with a seeded admin → session backend automatically.)
- Merge the unauthenticated `auth_login_router`.
- Deps: `argon2`, `rand`, `base64` (root + kyma-server `Cargo.toml`).

### Tests
- `passwords.rs`: hash→verify round-trip; wrong password fails.
- catalog: create_user + get_by_username + insert/lookup/revoke api_token (testcontainers, 009 migration).
- `auth_handler` (feature `test-support`): login success → token works on a protected route; bad password → 401; logout → token rejected; expired/revoked rejected.
- `session_backend`: env token still works; db session works; unknown → UnknownToken.

---

## A2 — Web

### `web/src/sdk/auth.ts`
`login({endpoint, username, password}) -> {token, user, expires_at}` (POST /v1/auth/login, NO bearer); `me({endpoint, token})`; `logout({endpoint, token})`. Local `headers`/`handleResponse` like the other sdk modules.

### Session store (`web/src/sdk/session.ts`)
Add `user: { username, role } | null`. `set`/`reset` updated. Keep `token` (now the session token). `isConfigured()` stays endpoint-only for the auth-disabled dev path; add `isAuthenticated() = Boolean(token)` for the guard. (Reconcile with the recent blank-token fix: auth-disabled engines need no token, so the guard allows through when the server reports auth disabled — see below.)

### `/login` route (`web/src/routes/login.tsx`, top-level, sibling to settings — NOT under `_app`)
Form: **Server URL** (default `http://localhost:8080`), **Username**, **Password** → `login()` → store `{endpoint, token, user, database}` → `navigate({to: next ?? "/explore"})`. Error toast on failure. A small "connect without login (unauthenticated dev server)" affordance that sets endpoint only (for auth-disabled engines) — preserves the dev flow.

### Guard (`web/src/routes/_app.tsx`)
`beforeLoad`: if not authenticated (no token) and the server isn't auth-disabled, `redirect({to:"/login", search:{next}})`. (For auth-disabled dev, endpoint-only suffices — keep that path working.)

### Settings (`web/src/routes/settings.tsx`) → account
- Show **logged in as `<username>` (`<role>`)** + **Log out** (calls `logout()` + `session.reset()` → `/login`).
- Keep **Server URL** (read-only/info once logged in).
- Demote the bearer-token field to an **"API token (advanced)"** collapsible (for connecting with a static token instead of login) — not the primary path.
- Nav/shell: show the logged-in user + a logout affordance (`web/src/app/shell.tsx`).

### Tests
- vitest `sdk/auth.ts` (login posts no bearer; me/logout send bearer; error mapping).

---

## Critical files
- Engine: `crates/kyma-catalog/migrations/009_auth.sql`; `crates/kyma-core/src/catalog.rs` + `crates/kyma-catalog/src/lib.rs` (user/token methods); `crates/kyma-server/src/auth/{session_backend.rs,passwords.rs}` + `auth_handler.rs` + `mod.rs`; `crates/kyma-bin/src/main.rs` (seed + backend select + mount). Reuse `db_backend.rs` `hash_token`, `backend.rs` `Principal/Role`, `middleware.rs`.
- Web: `web/src/sdk/auth.ts`, `web/src/sdk/session.ts`, `web/src/routes/login.tsx`, `_app.tsx`, `settings.tsx`, `web/src/app/shell.tsx`.

## Verification (end-to-end)
1. Unit: `cargo test -p kyma-server --features test-support auth` + catalog user/token tests; web vitest `sdk/auth`.
2. Live: start the stack with `KYMA_ADMIN_USER=admin KYMA_ADMIN_PASSWORD=…`; engine seeds the admin. Open the web → redirected to `/login` → log in as admin → lands on Explore; protected endpoints work with the issued session token; `/v1/auth/me` returns the user; **Log out** → back to `/login`, old token rejected (401). A static `KYMA_AUTH_TOKENS` token still authenticates the API. Auth-disabled engine (no users, empty tokens) → "connect without login" still works.

## Risks
1. **Don't lock yourself out / don't break auth-disabled dev.** Keep the env-token + auth-disabled passthrough intact; the session backend is additive. The blank-token web fix must coexist (auth-disabled → no login needed).
2. **Password storage** must be argon2 (NOT the SHA-256 token path). Generic login errors (no user enumeration). Session tokens high-entropy + expiring + revocable.
3. **Unauthenticated login route** must be merged WITHOUT the auth layer (structural exemption, like `health_router`) — verify it's not accidentally under a protected router.
4. **Migration adds `api_tokens`** that prod cloud deploys may already create out-of-band — use `CREATE TABLE` (fail-if-exists is fine for self-hosted; coordinate before merging if cloud has it). Confirm no conflict with the `cloud-auth` path.
