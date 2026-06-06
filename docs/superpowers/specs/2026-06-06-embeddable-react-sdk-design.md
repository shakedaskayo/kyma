# Embeddable React SDK — Design

**Date:** 2026-06-06
**Status:** Approved (sections 1–2 explicitly; section 3 finalized under autonomous-completion directive)
**Owner:** Shaked / Claude

## Goal

Let anyone embed Kyma's UI views (Graph, Discover, Query, Dashboards, Agent chat) into their own web application by installing an npm package, pointing it at a Kyma server URL (cluster), and rendering React components — with deep customization (theme tokens → props/callbacks → headless hooks) and production-grade auth for every audience:

1. OSS self-hosters embedding into internal tools
2. SaaS products embedding Kyma-powered views for **their** end-users (multi-tenant)
3. Kyma Cloud customers
4. AI-agent product builders (memory/graph visualizations in agent UIs)

## Non-goals (v1)

- iframe/no-code embed routes (can be layered on later; would consume this SDK internally)
- Web-components/framework-agnostic wrappers (React only)
- Supabase-specific auth integration (deferred; generic OIDC covers Supabase-as-issuer)
- Per-node/per-row data-level ACLs (scoping is per-database + role)

## Architecture (Approach B — package extraction, app dogfoods the SDK)

```
kyma/
├── packages/
│   ├── client/                      # @kyma-ai/client — framework-agnostic TS API client
│   │   └── src/                     # extracted from web/src/sdk/ (minus session.ts)
│   └── react/                       # @kyma-ai/react — embeddable component library
│       └── src/
│           ├── provider/            # KymaProvider, context, theme injection, portal container
│           ├── hooks/               # headless layer (useKymaGraph, useKymaQuery, …)
│           ├── graph/  query/  discover/  dashboards/  agent/   # the five views
│           └── theme/               # KymaTheme type, kymaDark/kymaLight presets, token map
├── web/                             # the Kyma app — consumes the packages via workspace:*
└── examples/embed-demo/             # standalone Vite host app (integration reference)
```

**Why B:** single source of truth — the Kyma app itself imports `@kyma-ai/react`, so embed-API regressions break the app build immediately. Rejected: dual-build from `web/` (mega-bundle, coupling leaks), iframe embeds (fails "highly tuneable").

### Package: `@kyma-ai/client`

- Pure TypeScript, zero React deps; works in browsers, Node, edge (the embed-token-minting story needs server-side usage).
- **Replaces global `installAuthFetch()`** (unacceptable in an embedded SDK — it patches host-app fetch) with an instance factory:

```ts
const client = createKymaClient({
  endpoint: "https://kyma.acme.internal",
  auth: { token } | { getToken: () => Promise<string> },
  database?: "prod",
  fetch?: customFetch,          // DI for tests/edge runtimes
});
client.graph.getOverview(…); client.query.execute(…); client.discover.search(…); // namespaced
```

- Auth behavior: bearer attached per-request; on 401 with `getToken`, single-flight re-mint + one retry; JWT `exp` (if parseable) triggers proactive refresh ~60s before expiry. Native opaque Kyma tokens pass through unchanged.
- Errors: `KymaApiError { status, code, message, requestId? }`; `KymaAuthError` subtype for 401-after-refresh so hosts can react (e.g., re-login).
- Streaming preserved: discover NDJSON async generators, agent SSE; all accept `AbortSignal`.
- `web/src/sdk/session.ts` (Zustand session persistence, Tauri store) **stays in `web/`** — app-specific.

### Package: `@kyma-ai/react`

- `react`/`react-dom` ≥18 as **peer deps**. Heavy view deps isolated per subpath export so bundlers tree-shake:
  - `@kyma-ai/react` → provider, hooks, theme (lightweight)
  - `@kyma-ai/react/graph` → react-force-graph-2d
  - `@kyma-ai/react/query` → monaco-editor
  - `@kyma-ai/react/discover` → (light)
  - `@kyma-ai/react/dashboards` → echarts, react-grid-layout
  - `@kyma-ai/react/agent` → ai-sdk
  - `@kyma-ai/react/styles.css` (+ per-subpath CSS) — compiled, prefixed
- Build: Vite library mode, ESM + `.d.ts` (vite-plugin-dts). Versioning/publish via Changesets; plain pnpm workspaces (no turborepo).
- Naming: `@kyma-ai/*` is the working scope ("kyma" collides with SAP's Kyma project on npm). **Verify scope availability before first publish**; fallback `@agentcylabs/kyma-*`.

## Customization contract (three layers)

### L1 — Provider & theme

```tsx
<KymaProvider
  endpoint="https://kyma.acme.internal"
  auth={{ token }}                              // or { getToken }
  database="prod"                               // default, overridable per component
  theme={kymaDark}                              // preset | Partial<KymaTheme> | "inherit"
  queryClient={hostQueryClient}                 // optional; otherwise isolated internal client
  onError={(err) => …}
>
```

- Creates the `KymaClient`, holds it in context; brings its own React Query `QueryClient` (isolated cache, keys namespaced `["kyma", endpoint, database, …]`) unless one is injected.
- Renders root element with class `kyma-root` carrying all `--kyma-*` CSS vars, **plus a portal container** (same class/vars) that all Radix-based popovers/dialogs/tooltips target via context — portals to `document.body` would otherwise escape theming.

### L2 — Components

All five views as self-contained components; every component accepts `database`, `className`/`style`, `fallback` (error boundary), and view-specific props/callbacks:

- `<KymaGraph graphs height layout toolbar sidebar minimap focusQuery onNodeClick onSelectionChange renderNodeTooltip />`
- `<KymaQueryEditor language defaultQuery showSchemaBrowser showResults timeRange readOnly onResults renderResultCell />`
- `<KymaDiscover sources defaultQuery onRowClick />`
- `<KymaDashboard dashboardId editable timeRange />`
- `<KymaAgentChat placeholder systemContext onMessage renderMessage />`

Render-prop escape hatches start minimal and named (`renderNodeTooltip`, `renderResultCell`, `renderMessage`); additive growth is non-breaking.

### L3 — Headless hooks

Same code paths the L2 components are built on (no drift possible):

- `useKymaGraph({graphs})` → `{nodes, edges, stats, expandNode, searchNodes, isLoading}`
- `useKymaQuery({language})` → `{execute, columns, rows, arrow, isRunning, error}` (Arrow-backed + row convenience)
- `useKymaDiscover()` → `{frames, search, isStreaming, abort}`
- `useKymaDashboards()`, `useKymaAgent()`
- `useKymaClient()`, `useKymaCapabilities()`

## Auth — generic OIDC (server work)

**Decision: no Supabase-specific integration; generic OIDC.** Hosts bring their own IdP (Auth0, Clerk, Keycloak, Cognito, Supabase-as-issuer). Kyma validates JWTs directly; for self-hosters without an IdP, existing native Kyma tokens keep working unchanged through `auth={{ token }}`.

### Server: `OidcAuthBackend` (new, implements existing `AuthBackend` trait)

- Config (env/CLI, following existing pattern in `crates/kyma-bin/src/main.rs`):
  - `KYMA_OIDC_ISSUERS` — comma-separated issuer URLs (each resolved via `/.well-known/openid-configuration`)
  - `KYMA_OIDC_AUDIENCE` — required `aud`
  - `KYMA_OIDC_ROLE_CLAIM` (default `kyma_role`; values `admin|write|read`; missing → `read`)
  - `KYMA_OIDC_SUBJECT_CLAIM` (default `sub`)
  - `KYMA_OIDC_DATABASES_CLAIM` (default `kyma_databases`; array of allowed database names; missing → all)
- JWKS fetched per issuer, cached with TTL + refresh-on-unknown-kid; RS256/ES256.
- Validates signature, `exp`/`nbf`, `iss`, `aud` → `Principal { tenant, role, subject, allowed_databases }`.
- **Chaining:** token that parses as a JWT (three dot-separated segments) → OIDC validation; otherwise falls through to the existing session/env backends. Wired in the backend-selection block of `main.rs`.
- New dep: `jsonwebtoken` (workspace).

### Per-database scoping

`Principal` gains `allowed_databases: Option<Vec<String>>` (None = unrestricted; preserves behavior for all existing backends). Enforcement via a shared helper called at every point a request's target database is resolved (query, explore/discover, graph, catalog/schema, dashboards data paths). Violations → 403 with a typed error body.

### CORS

`KYMA_CORS_ALLOWED_ORIGINS` — comma-separated allow-list; replaces `with_permissive_cors` mirror behavior when set. Unset → current permissive behavior (dev default), with a startup warning when auth is enabled.

## Theming & CSS isolation

- Library Tailwind build: `prefix: "ky-"`, `preflight: false`; selectors scoped under `.kyma-root`. Host styles and Kyma styles cannot collide in either direction.
- All design tokens become `--kyma-*` CSS vars (mapped from the existing HSL token set in `web/src/styles/globals.css`), set inline on the provider root from a typed `KymaTheme` object.
- Presets: `kymaDark`, `kymaLight`; `theme="inherit"` maps a documented set of host CSS vars onto `--kyma-*`.
- Fonts default to `inherit` (never force-load fonts into a host app); `--kyma-font-sans`/`--kyma-font-mono` overridable.
- Non-CSS renderers (Monaco theme, ECharts theme, force-graph canvas colors) derive from the same token object at runtime.
- `web/` app: keeps its global stylesheet for shell chrome but renders views through the same provider/tokens (dogfooding the theme contract).

## Data flow

- React Query for request/cache lifecycle; streaming endpoints (discover NDJSON, agent SSE) managed by hooks with `AbortController` tied to unmount.
- Query results: Apache Arrow tables exposed directly plus row-array convenience accessors.
- Capabilities: `useKymaCapabilities()` reads `/v1/capabilities` (cached); components degrade gracefully — e.g., `KymaAgentChat` renders a "not available on this server" state instead of erroring when the capability is absent. Dev-mode console warning on server/SDK version mismatch.

## Error handling

- Typed errors from client (`KymaApiError`, `KymaAuthError`).
- Each L2 component wraps itself in an error boundary; `fallback` prop overrides the default styled inline error card (with retry button).
- Provider-level `onError` observes all errors (telemetry hook for hosts).
- Unreachable-cluster state: distinct inline presentation with retry; never an unhandled throw into the host app.

## Testing

- `@kyma-ai/client`: vitest + msw — auth single-flight/refresh/retry, NDJSON & SSE parsing, error mapping, abort propagation.
- `@kyma-ai/react`: vitest + Testing Library — hooks against msw; smoke render per component; error-boundary and capability-degradation states. Storybook stories per component (knobs for theme/props) double as visual review surface.
- Server (Rust): integration tests following `crates/kyma-server/tests/` patterns — mock OIDC issuer (axum test server serving discovery + JWKS), JWT validation paths (valid/expired/wrong-aud/wrong-iss/unknown-kid), backend chaining (JWT → OIDC, opaque → session/env), per-database scope enforcement (403), CORS allow-list behavior.
- Integration gates in CI: `web/` app builds against the packages (dogfood), `examples/embed-demo` builds + typechecks.

## Deliverables

1. `@kyma-ai/client` + `@kyma-ai/react` packages (built, typed, tree-shakable subpaths)
2. `web/` refactored to consume both packages (`workspace:*`)
3. Server: `OidcAuthBackend`, per-database scoping, CORS allow-list (+ tests)
4. `examples/embed-demo` — standalone Vite host app embedding all five components against a real server (includes a minimal token-minting Node snippet for the multi-tenant pattern)
5. Storybook for `@kyma-ai/react`
6. Docs: embedding guide (install, provider, auth patterns incl. OIDC claim reference, theming reference, per-component props), in `docs/`
7. npm publish pipeline: Changesets + CI workflow (build, test, publish on version PR merge)

## Compatibility & versioning

- Semver via Changesets; packages start at `0.1.0`.
- SDK ↔ server compatibility surfaced through `/v1/capabilities` (feature detection over version pinning).
- Public API = everything exported from package roots/subpaths; render-prop additions are minor versions, removals are major.

## Risks & mitigations

- **Extraction churn in `web/`** — biggest one-time cost; mitigated by moving `web/src/sdk/` mostly as-is (its `{endpoint, token}`-args style already fits the client factory) and keeping app-only state (session persistence, tabs, routing) out of the packages.
- **CSS prefix migration** — components moving into the package must switch to prefixed classes; mechanical, verified by the dogfooding build.
- **Portal theming** — solved by provider-owned portal container; must be enforced for every Radix portal in extracted components.
- **npm scope availability** — verify `@kyma-ai` before publish; fallback documented.
