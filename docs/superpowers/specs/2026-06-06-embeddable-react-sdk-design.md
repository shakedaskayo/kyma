# Embeddable React SDK — Design

**Date:** 2026-06-06
**Status:** Approved (sections 1–2 explicitly; section 3 finalized under autonomous-completion directive)
**Owner:** Shaked / Claude

## Goal

Let anyone embed Pensieve's UI views (Graph, Discover, Query, Dashboards, Agent chat) into their own web application by installing an npm package, pointing it at a Pensieve server URL (cluster), and rendering React components — with deep customization (theme tokens → props/callbacks → headless hooks) and production-grade auth for every audience:

1. OSS self-hosters embedding into internal tools
2. SaaS products embedding Pensieve-powered views for **their** end-users (multi-tenant)
3. Pensieve Cloud customers
4. AI-agent product builders (memory/graph visualizations in agent UIs)

## Non-goals (v1)

- iframe/no-code embed routes (can be layered on later; would consume this SDK internally)
- Web-components/framework-agnostic wrappers (React only)
- Supabase-specific auth integration (deferred; generic OIDC covers Supabase-as-issuer)
- Per-node/per-row data-level ACLs (scoping is per-database + role)

## Architecture (Approach B — package extraction, app dogfoods the SDK)

```
pensieve/
├── packages/
│   ├── client/                      # @pensieve-ai/client — framework-agnostic TS API client
│   │   └── src/                     # extracted from web/src/sdk/ (minus session.ts)
│   └── react/                       # @pensieve-ai/react — embeddable component library
│       └── src/
│           ├── provider/            # PensieveProvider, context, theme injection, portal container
│           ├── hooks/               # headless layer (usePensieveGraph, usePensieveQuery, …)
│           ├── graph/  query/  discover/  dashboards/  agent/   # the five views
│           └── theme/               # PensieveTheme type, pensieveDark/pensieveLight presets, token map
├── web/                             # the Pensieve app — consumes the packages via workspace:*
└── examples/embed-demo/             # standalone Vite host app (integration reference)
```

**Why B:** single source of truth — the Pensieve app itself imports `@pensieve-ai/react`, so embed-API regressions break the app build immediately. Rejected: dual-build from `web/` (mega-bundle, coupling leaks), iframe embeds (fails "highly tuneable").

### Package: `@pensieve-ai/client`

- Pure TypeScript, zero React deps; works in browsers, Node, edge (the embed-token-minting story needs server-side usage).
- **Replaces global `installAuthFetch()`** (unacceptable in an embedded SDK — it patches host-app fetch) with an instance factory:

```ts
const client = createPensieveClient({
  endpoint: "https://pensieve.acme.internal",
  auth: { token } | { getToken: () => Promise<string> },
  database?: "prod",
  fetch?: customFetch,          // DI for tests/edge runtimes
});
client.graph.getOverview(…); client.query.execute(…); client.discover.search(…); // namespaced
```

- Auth behavior: bearer attached per-request; on 401 with `getToken`, single-flight re-mint + one retry; JWT `exp` (if parseable) triggers proactive refresh ~60s before expiry. Native opaque Pensieve tokens pass through unchanged.
- Errors: `PensieveApiError { status, code, message, requestId? }`; `PensieveAuthError` subtype for 401-after-refresh so hosts can react (e.g., re-login).
- Streaming preserved: discover NDJSON async generators, agent SSE; all accept `AbortSignal`.
- `web/src/sdk/session.ts` (Zustand session persistence, Tauri store) **stays in `web/`** — app-specific.

### Package: `@pensieve-ai/react`

- `react`/`react-dom` ≥18 as **peer deps**. Heavy view deps isolated per subpath export so bundlers tree-shake:
  - `@pensieve-ai/react` → provider, hooks, theme (lightweight)
  - `@pensieve-ai/react/graph` → react-force-graph-2d
  - `@pensieve-ai/react/query` → monaco-editor
  - `@pensieve-ai/react/discover` → (light)
  - `@pensieve-ai/react/dashboards` → echarts, react-grid-layout
  - `@pensieve-ai/react/agent` → ai-sdk
  - `@pensieve-ai/react/styles.css` (+ per-subpath CSS) — compiled, prefixed
- Build: Vite library mode, ESM + `.d.ts` (vite-plugin-dts). Versioning/publish via Changesets; plain pnpm workspaces (no turborepo).
- Naming: `@pensieve-ai/*` is the working scope ("pensieve" collides with SAP's Pensieve project on npm). **Verify scope availability before first publish**; fallback `@agentcylabs/pensieve-*`.

## Customization contract (three layers)

### L1 — Provider & theme

```tsx
<PensieveProvider
  endpoint="https://pensieve.acme.internal"
  auth={{ token }}                              // or { getToken }
  database="prod"                               // default, overridable per component
  theme={pensieveDark}                              // preset | Partial<PensieveTheme> | "inherit"
  queryClient={hostQueryClient}                 // optional; otherwise isolated internal client
  onError={(err) => …}
>
```

- Creates the `PensieveClient`, holds it in context; brings its own React Query `QueryClient` (isolated cache, keys namespaced `["pensieve", endpoint, database, …]`) unless one is injected.
- Renders root element with class `pensieve-root` carrying all `--pensieve-*` CSS vars, **plus a portal container** (same class/vars) that all Radix-based popovers/dialogs/tooltips target via context — portals to `document.body` would otherwise escape theming.

### L2 — Components

All five views as self-contained components; every component accepts `database`, `className`/`style`, `fallback` (error boundary), and view-specific props/callbacks:

- `<PensieveGraph graphs height layout toolbar sidebar minimap focusQuery onNodeClick onSelectionChange renderNodeTooltip />`
- `<PensieveQueryEditor language defaultQuery showSchemaBrowser showResults timeRange readOnly onResults renderResultCell />`
- `<PensieveDiscover sources defaultQuery onRowClick />`
- `<PensieveDashboard dashboardId editable timeRange />`
- `<PensieveAgentChat placeholder systemContext onMessage renderMessage />`

Render-prop escape hatches start minimal and named (`renderNodeTooltip`, `renderResultCell`, `renderMessage`); additive growth is non-breaking.

### L3 — Headless hooks

Same code paths the L2 components are built on (no drift possible):

- `usePensieveGraph({graphs})` → `{nodes, edges, stats, expandNode, searchNodes, isLoading}`
- `usePensieveQuery({language})` → `{execute, columns, rows, arrow, isRunning, error}` (Arrow-backed + row convenience)
- `usePensieveDiscover()` → `{frames, search, isStreaming, abort}`
- `usePensieveDashboards()`, `usePensieveAgent()`
- `usePensieveClient()`, `usePensieveCapabilities()`

## Auth — generic OIDC (server work)

**Decision: no Supabase-specific integration; generic OIDC.** Hosts bring their own IdP (Auth0, Clerk, Keycloak, Cognito, Supabase-as-issuer). Pensieve validates JWTs directly; for self-hosters without an IdP, existing native Pensieve tokens keep working unchanged through `auth={{ token }}`.

### Server: `OidcAuthBackend` (new, implements existing `AuthBackend` trait)

- Config (env/CLI, following existing pattern in `crates/pensieve-bin/src/main.rs`):
  - `PENSIEVE_OIDC_ISSUERS` — comma-separated issuer URLs (each resolved via `/.well-known/openid-configuration`)
  - `PENSIEVE_OIDC_AUDIENCE` — required `aud`
  - `PENSIEVE_OIDC_ROLE_CLAIM` (default `pensieve_role`; values `admin|write|read`; missing → `read`)
  - `PENSIEVE_OIDC_SUBJECT_CLAIM` (default `sub`)
  - `PENSIEVE_OIDC_DATABASES_CLAIM` (default `pensieve_databases`; array of allowed database names; missing → all)
- JWKS fetched per issuer, cached with TTL + refresh-on-unknown-kid; RS256/ES256.
- Validates signature, `exp`/`nbf`, `iss`, `aud` → `Principal { tenant, role, subject, allowed_databases }`.
- **Chaining:** token that parses as a JWT (three dot-separated segments) → OIDC validation; otherwise falls through to the existing session/env backends. Wired in the backend-selection block of `main.rs`.
- New dep: `jsonwebtoken` (workspace).

### Per-database scoping

`Principal` gains `allowed_databases: Option<Vec<String>>` (None = unrestricted; preserves behavior for all existing backends). Enforcement via a shared helper called at every point a request's target database is resolved (query, explore/discover, graph, catalog/schema, dashboards data paths). Violations → 403 with a typed error body.

### CORS

`PENSIEVE_CORS_ALLOWED_ORIGINS` — comma-separated allow-list; replaces `with_permissive_cors` mirror behavior when set. Unset → current permissive behavior (dev default), with a startup warning when auth is enabled.

## Theming & CSS isolation

- Library Tailwind build: `prefix: "pv-"`, `preflight: false`; selectors scoped under `.pensieve-root`. Host styles and Pensieve styles cannot collide in either direction.
- All design tokens become `--pensieve-*` CSS vars (mapped from the existing HSL token set in `web/src/styles/globals.css`), set inline on the provider root from a typed `PensieveTheme` object.
- Presets: `pensieveDark`, `pensieveLight`; `theme="inherit"` maps a documented set of host CSS vars onto `--pensieve-*`.
- Fonts default to `inherit` (never force-load fonts into a host app); `--pensieve-font-sans`/`--pensieve-font-mono` overridable.
- Non-CSS renderers (Monaco theme, ECharts theme, force-graph canvas colors) derive from the same token object at runtime.
- `web/` app: keeps its global stylesheet for shell chrome but renders views through the same provider/tokens (dogfooding the theme contract).

## Data flow

- React Query for request/cache lifecycle; streaming endpoints (discover NDJSON, agent SSE) managed by hooks with `AbortController` tied to unmount.
- Query results: Apache Arrow tables exposed directly plus row-array convenience accessors.
- Capabilities: `usePensieveCapabilities()` reads `/v1/capabilities` (cached); components degrade gracefully — e.g., `PensieveAgentChat` renders a "not available on this server" state instead of erroring when the capability is absent. Dev-mode console warning on server/SDK version mismatch.

## Error handling

- Typed errors from client (`PensieveApiError`, `PensieveAuthError`).
- Each L2 component wraps itself in an error boundary; `fallback` prop overrides the default styled inline error card (with retry button).
- Provider-level `onError` observes all errors (telemetry hook for hosts).
- Unreachable-cluster state: distinct inline presentation with retry; never an unhandled throw into the host app.

## Testing

- `@pensieve-ai/client`: vitest + msw — auth single-flight/refresh/retry, NDJSON & SSE parsing, error mapping, abort propagation.
- `@pensieve-ai/react`: vitest + Testing Library — hooks against msw; smoke render per component; error-boundary and capability-degradation states. Storybook stories per component (knobs for theme/props) double as visual review surface.
- Server (Rust): integration tests following `crates/pensieve-server/tests/` patterns — mock OIDC issuer (axum test server serving discovery + JWKS), JWT validation paths (valid/expired/wrong-aud/wrong-iss/unknown-kid), backend chaining (JWT → OIDC, opaque → session/env), per-database scope enforcement (403), CORS allow-list behavior.
- Integration gates in CI: `web/` app builds against the packages (dogfood), `examples/embed-demo` builds + typechecks.

## Deliverables

1. `@pensieve-ai/client` + `@pensieve-ai/react` packages (built, typed, tree-shakable subpaths)
2. `web/` refactored to consume both packages (`workspace:*`)
3. Server: `OidcAuthBackend`, per-database scoping, CORS allow-list (+ tests)
4. `examples/embed-demo` — standalone Vite host app embedding all five components against a real server (includes a minimal token-minting Node snippet for the multi-tenant pattern)
5. Storybook for `@pensieve-ai/react`
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
- **npm scope availability** — verify `@pensieve-ai` before publish; fallback documented.
