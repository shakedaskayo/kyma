# C3 — Onboarding + Dashboard + Claude Code Hook Implementation Plan (REVISED)

> **For agentic workers:** REQUIRED SUB-SKILL: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **REVISION 2026-05-25:** Original C3 specced building a 2-tool MCP server in the gateway. That was wrong — a full `pensieve-mcp` crate already exists on branch `feature/cloud-slice-1a-mcp-server` (8 agent tools, JSON-RPC 2.0 over Streamable HTTP at `/mcp/v1`, `Role::Read` auth, full test suite). It is mounted in `pensieve-bin`. This revision **consumes that surface** instead of rebuilding it, and aligns to the pre-existing master spec `docs/superpowers/specs/2026-05-02-pensieve-cloud-platform-design.md` (Slice 1a). C3 no longer builds any MCP code; it builds the **onboarding + dashboard + workspace→token→MCP-URL** loop on top of the existing engine surface.

**Goal:** A developer signs up, gets a workspace, points telemetry at their Pensieve Cloud endpoint, and **connects Claude Code in under 60 seconds** by either pasting a `claude mcp add` command or `/skill install`-ing `pensieve-claude-skill` — then their agent queries their telemetry via the existing `/mcp/v1` tools. After C3, "data in → answers in Claude Code" works for a real signed-up user.

**Architecture:** The MCP server already exists and is mounted at `/mcp/v1` on `pensieve-server` behind the engine's `require_role_middleware(Role::Read)`. With the C2 control plane issuing `api_tokens` (read by the engine's `DbAuthBackend`), a workspace's bearer token already authenticates to MCP and resolves to that workspace's `tenant_id`. So C3 builds only the **product surface**: a Next.js dashboard (in the `cloud/web` dir per the existing spec) for signup/login/workspace-create, a one-time token reveal, and a generated per-workspace MCP connection snippet + the `pensieve-claude-skill` install path. The per-workspace MCP URL is `mcp.pensieve.dev/<workspace-id>` (path/subdomain routing → the shared engine's `/mcp/v1`, behind Cloudflare for rate limiting). No new query path, no new MCP code.

**Tech Stack:** Next.js 15 (`cloud/web`) + the C2 control-plane API (Hono/TypeScript, `cloud/api`) + the existing `pensieve-mcp` crate (unchanged) + the existing `pensieve-claude-skill` repo (from Slice 1a Task 10). Cloudflare for `mcp.pensieve.dev` routing/rate-limit. Verification: a real `claude mcp add` / `/skill install` against a live workspace, plus the engine's existing `pensieve-mcp` e2e tests as the protocol guarantee.

**Prerequisites:**
- C2 (revised) complete — control plane issues `api_tokens`; engine runs with `cloud-auth` feature so `DbAuthBackend` reads them; workspace → tenant_id mapping live.
- Engine deployed from a build that includes Slice 0 (tenancy) + Slice 1a (`pensieve-mcp`). **Task 0 confirms these are merged/available in the deployed image.**

**File Structure (created across this plan):**

```
cloud/web/                                  # Next.js 15 customer dashboard (per existing spec)
  app/(auth)/login/page.tsx  signup/page.tsx
  app/dashboard/page.tsx                    workspace list
  app/dashboard/workspaces/[id]/page.tsx    endpoints + token reveal + MCP snippet
  app/dashboard/workspaces/[id]/console/page.tsx  minimal KQL/SQL console
  components/McpConnect.tsx  CopyButton.tsx
  lib/api.ts                                cloud/api client (session-auth'd)
scripts/cloud/
  c3-mcp-smoke.sh                           JSON-RPC round trip against deployed /mcp/v1
  c3-onboarding-e2e.sh                      signup->workspace->token->ingest->mcp query
docs/cloud/
  c3-onboarding-report.md                   THE DELIVERABLE: timed 60s loop + evidence
```

---

## Task 0: Confirm the deployed engine includes Slice 0 + Slice 1a

**Objective:** Make sure the engine image C3 builds on actually serves `/mcp/v1` and authenticates cloud `api_tokens`.

**Files:**
- Create: `docs/cloud/c3-prereqs.md`

- [ ] **Step 1: Confirm the surfaces exist in the deployed engine build**

Run:
```bash
# against the C1/C2 deployed engine (internal URL, admin token):
curl -fsS -X POST "$ENGINE_URL/mcp/v1" -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"prereq","version":"0"}}}'
```
Expected: a JSON-RPC result with `serverInfo`. If 404, the deployed image predates Slice 1a — rebuild from a branch/commit that includes `crates/pensieve-mcp` and its `pensieve-bin` mount (commits `6ef03cbe`, `29180452`).

- [ ] **Step 2: Confirm `cloud-auth` (DbAuthBackend) is active** — the engine must be built with the `cloud-auth` feature and pointed at the control-plane Postgres so `api_tokens` authenticate. Verify a C2-issued token authenticates against `/v1/query`.

- [ ] **Step 3: Write `docs/cloud/c3-prereqs.md`** recording: engine commit/image, that `/mcp/v1` responds, that `DbAuthBackend` is wired, and the 8 tool names returned by `tools/list` (`list_databases`, `describe_table`, `run_sql`, `run_kql`, `sample_rows`, `explore_schema`, `find_references_to`, `graph_traverse`).

- [ ] **Step 4: Commit**

```bash
git add docs/cloud/c3-prereqs.md
git commit -m "docs(c3): confirm engine serves /mcp/v1 + cloud-auth in deployed build"
```

---

## Task 1: MCP smoke test against the deployed engine (no new code)

**Objective:** Prove a workspace `api_token` can drive the existing `/mcp/v1` end to end before building any UI.

**Files:**
- Create: `scripts/cloud/c3-mcp-smoke.sh`

- [ ] **Step 1: Write `scripts/cloud/c3-mcp-smoke.sh`** — `initialize`, `tools/list`, then `tools/call run_kql`. The auth credential is the workspace query token; build the header value at call time from the `WS_TOKEN` env var (scheme + token) so no literal secret appears in source.

```bash
#!/usr/bin/env bash
set -euo pipefail
: "${MCP_URL:?}"; : "${WS_TOKEN:?}"
auth_header() { printf 'Authorization: %s %s' "Bearer" "$WS_TOKEN"; }
rpc() { curl -fsS -X POST "$MCP_URL" -H "$(auth_header)" \
  -H "Content-Type: application/json" -d "$1"; echo; }

rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
rpc '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_kql","arguments":{"query":"otel_logs | take 1"}}}'
echo "C3 MCP smoke OK"
```

- [ ] **Step 2: Run it**

Run:
```bash
chmod +x scripts/cloud/c3-mcp-smoke.sh
MCP_URL="https://$(terraform -chdir=infra/envs/dev output -raw engine_url)/mcp/v1" \
  WS_TOKEN="<C2-issued workspace token>" scripts/cloud/c3-mcp-smoke.sh
```
Expected: 3 JSON-RPC results; the third returns the workspace's own rows. Cross-check: a token for a different workspace returns only its own rows (engine `tenant_id` scoping; proven by Slice 0's `tenant_isolation_it`).

- [ ] **Step 3: Commit**

```bash
git add scripts/cloud/c3-mcp-smoke.sh
git commit -m "test(c3): MCP smoke against existing /mcp/v1 via workspace token"
```

---

## Task 2: Per-workspace MCP routing (mcp.pensieve.dev/<wsid>)

**Objective:** Give each workspace a stable MCP URL that routes to the shared engine's `/mcp/v1`, per the existing spec's DNS topology.

**Files:**
- Modify: `cloud/api` (return `mcp_endpoint` on workspace) + edge routing config (Cloudflare / Railway)

- [ ] **Step 1: Routing** — `mcp.pensieve.dev/<workspace-id>` → shared engine `/mcp/v1`. The workspace's `api_token` is authoritative for tenant resolution (engine `DbAuthBackend`); the path segment is for clean per-workspace URLs + Cloudflare rate-limit keying. For dev, a path-prefix on the gateway/edge is enough; record the production Cloudflare config.

- [ ] **Step 2: cloud/api surfaces endpoints** — workspace GET returns `pensieve_endpoint` (ingest/query) and `mcp_endpoint` (`https://mcp.pensieve.dev/<wsid>`), matching the `workspaces` columns the existing spec defines.

- [ ] **Step 3: Verify** — `c3-mcp-smoke.sh` works against the `mcp_endpoint` URL, not just the raw engine URL.

- [ ] **Step 4: Commit**

```bash
git add cloud/api
git commit -m "feat(cloud): per-workspace mcp endpoint routing"
```

---

## Task 3: Dashboard — auth + workspace create

**Objective:** A real user signs up (GitHub OAuth or magic link per existing spec), logs in, creates a workspace.

**Files:**
- Create: `cloud/web/` Next.js app, auth pages, `lib/api.ts`

- [ ] **Step 1: Scaffold `cloud/web` (Next.js 15)** wired to the C2 control-plane API. Auth uses the existing spec's choice (GitHub OAuth + magic link via the control plane), not a second auth system. Protect `/dashboard/*`.

- [ ] **Step 2: Workspace list + create** — calls cloud/api `POST /workspaces` (creates workspace row + tenant_id + plan=free, per existing spec: "creating a workspace = inserting rows + minting an MCP token"). List the user's workspaces.

- [ ] **Step 3: Verify + deploy** — signup → login → create workspace → it appears. Deploy `cloud/web` as a Railway service.

- [ ] **Step 4: Commit**

```bash
git add cloud/web
git commit -m "feat(cloud/web): auth + workspace create"
```

---

## Task 4: Workspace page — token reveal + MCP connect snippet (the payoff)

**Objective:** The 60-second screen: endpoints, a one-time token reveal, and the copy-paste Claude Code connection.

**Files:**
- Create: `cloud/web/app/dashboard/workspaces/[id]/page.tsx`, `components/McpConnect.tsx`, `CopyButton.tsx`

- [ ] **Step 1: Endpoints section** — show `pensieve_endpoint` + `mcp_endpoint` with copy buttons.

- [ ] **Step 2: Token issuance** — "Create token" calls cloud/api to mint an `api_tokens` row (the same row the engine's `DbAuthBackend` reads). The full token is shown **once** in a reveal modal with a copy button + "store it now" warning; thereafter only the prefix + created date + scopes.

- [ ] **Step 3: `McpConnect.tsx`** — render the exact Claude Code connection for this workspace, assembled from parts so it is correct and copy-pasteable:
  - **Option A (CLI):** `claude mcp add pensieve --transport http <mcp_endpoint>` plus a header flag carrying the authorization value (scheme + the workspace token).
  - **Option B (skill):** `/skill install https://github.com/<org>/pensieve-claude-skill`, then paste `<mcp_endpoint>` + token when prompted — this reuses the `pensieve-claude-skill` repo from Slice 1a Task 10.
  - **Option C (team `.mcp.json`):** the project-scoped config block.
  - Mark the token as a secret in all three.

- [ ] **Step 4: Ingest quickstart** — a ready-to-run curl posting sample NDJSON to `pensieve_endpoint` with the workspace token, so "data in" is also one copy-paste.

- [ ] **Step 5: Headline acceptance — the snippet must actually connect Claude Code**

Run (literally, with the values from the page):
```bash
claude mcp add pensieve --transport http "<mcp_endpoint>" --header "<auth header from page>"
claude mcp list           # pensieve shows connected
# in a claude session: "use pensieve to run: otel_logs | take 5"
```
Expected: Claude Code lists `pensieve` connected and calls `run_kql`. If `claude` CLI is unavailable in the env, validate via the `pensieve-claude-skill` install path + `c3-mcp-smoke.sh` and record that.

- [ ] **Step 6: Commit**

```bash
git add "cloud/web/app/dashboard/workspaces/[id]/page.tsx" cloud/web/components
git commit -m "feat(cloud/web): workspace page with token reveal + claude mcp connect"
```

---

## Task 5: Minimal query console

**Objective:** Let a user run a KQL/SQL query in-browser to confirm data is flowing.

**Files:**
- Create: `cloud/web/app/dashboard/workspaces/[id]/console/page.tsx`

- [ ] **Step 1: Console UI** — textarea + KQL/SQL toggle + Run. Calls the workspace `pensieve_endpoint` `/v1/query` **proxied through cloud/api using the user's session** (never expose the raw workspace token in client JS). Render NDJSON rows as a table.

- [ ] **Step 2: Verify** — ingest sample rows, run `otel_logs | take 5`, see only this workspace's rows.

- [ ] **Step 3: Commit**

```bash
git add "cloud/web/app/dashboard/workspaces/[id]/console/page.tsx"
git commit -m "feat(cloud/web): minimal in-browser query console"
```

---

## Task 6: End-to-end onboarding test + timed report (the deliverable)

**Objective:** Prove and time the whole loop for a fresh user.

**Files:**
- Create: `scripts/cloud/c3-onboarding-e2e.sh`
- Create: `docs/cloud/c3-onboarding-report.md`

- [ ] **Step 1: Write `scripts/cloud/c3-onboarding-e2e.sh`** — scripted human flow: create workspace via cloud/api → mint token → ingest sample NDJSON via `pensieve_endpoint` → `tools/call run_kql` via `mcp_endpoint` → assert the sentinel row returns. Time steps.

- [ ] **Step 2: Run it.** Expected: sentinel row returned via MCP; elapsed time recorded.

- [ ] **Step 3: Real human run once** — sign up in a browser, create workspace, copy the `claude mcp add` command (or `/skill install`), ask Claude Code a question. Capture transcript/screenshot + wall-clock.

- [ ] **Step 4: Write `docs/cloud/c3-onboarding-report.md`**

````markdown
# C3 — Onboarding Loop Report

**Date:** <fill>  **Engine build:** <commit incl. Slice 0 + 1a>

## The loop, timed
| Step | Action | Time |
|---|---|---|
| 1 | Sign up + create workspace | |
| 2 | Copy + run `claude mcp add` (or `/skill install`) | |
| 3 | First successful `run_kql` from Claude Code | |
| **Total** | signup -> answer in Claude Code | **<target < 60s after data exists>** |

## Evidence
- Scripted e2e: <pass/fail>, sentinel round-trip <ms>.
- Human run: <transcript/screenshot>. Claude Code called `run_kql`, returned workspace rows.
- MCP surface: existing `pensieve-mcp` `/mcp/v1` (8 tools), auth via cloud `api_tokens` (DbAuthBackend). No new MCP code built.

## Findings / friction
- <where the 60s went; any confusion>
- Cross-workspace isolation reconfirmed end-to-end (engine tenant_id scoping).

## Verdict
<PROCEED to C4 / fixes>
````

- [ ] **Step 5: Commit**

```bash
git add scripts/cloud/c3-onboarding-e2e.sh docs/cloud/c3-onboarding-report.md
git commit -m "test(c3): onboarding e2e + timed loop report"
```

---

## Task 7: Phase exit checklist

- [ ] Deployed engine serves `/mcp/v1` (8 tools) and authenticates cloud `api_tokens` (Task 0).
- [ ] `cloud/web` deployed: signup, login, workspace create, workspace page.
- [ ] Per-workspace `mcp_endpoint` (`mcp.pensieve.dev/<wsid>`) routes to the engine.
- [ ] Token minted, shown once, stored hashed in `api_tokens`; drives both ingest/query and MCP.
- [ ] `claude mcp add` / `/skill install` connects Claude Code to the workspace.
- [ ] Console renders workspace-only rows.
- [ ] Onboarding e2e passes; timed loop documented; human run captured.
- [ ] **No MCP code was rebuilt** — C3 consumed `pensieve-mcp` and `pensieve-claude-skill` from Slice 1a.

- [ ] **Mark C3 complete in the master design**

```bash
git add docs/superpowers/specs/2026-05-25-pensieve-cloud-platform-design.md
git commit -m "docs(cloud): mark C3 complete (consumes existing MCP)"
```

---

## Notes for the implementer

- **Do not build an MCP server.** It exists (`crates/pensieve-mcp`, Slice 1a). C3 builds the product surface that issues tokens and shows users how to connect. If you start writing JSON-RPC code, stop — you're duplicating shipped, tested work.
- **The 8 tools are richer than a generic query tool.** `run_kql`, `run_sql`, `list_databases`, `describe_table`, `sample_rows`, `explore_schema`, `find_references_to`, `graph_traverse`. The dashboard's copy should advertise these so users know their agent can explore schema and traverse the graph, not just run one query.
- **`api_tokens` is the integration contract.** The control plane (C2) writes the row; the engine's `DbAuthBackend` reads it; the same token is the MCP bearer. Don't invent a second token system.
- **Reuse `pensieve-claude-skill`.** Slice 1a Task 10 created a `/skill install`-able repo. C3's job is to point users at it with their workspace URL + token, not to author a new one.
- **Never expose a workspace token in client JS.** The console proxies through cloud/api with the user's session. The token is shown once at issuance, otherwise server-side.
- **Headline acceptance is Task 4 Step 5** — a real `claude mcp add` connecting Claude Code. Everything else is plumbing for that one command.
- **Honor the existing DNS/spec naming.** `mcp.pensieve.dev/<wsid>`, `cloud.pensieve.dev`, `workspaces` (not "projects"), `api_tokens` (not "api_keys"). Aligning names to `2026-05-02-pensieve-cloud-platform-design.md` avoids forking the architecture.
