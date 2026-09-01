# pensieve-embed-demo

Reference host app for the `@pensieve-ai/react` embeddable SDK. Demonstrates all
five Pensieve views (Graph, Query, Discover, Dashboards, Agent) plus the headless
hooks layer (L3), theme switching, and the multi-tenant mint-server auth pattern.

## Prerequisites

- Node 20+ / pnpm 9+
- A running Pensieve server (default `http://localhost:8080`)
- A valid Pensieve bearer token
- Server started with CORS open to the Vite dev port:

```
PENSIEVE_CORS_ALLOWED_ORIGINS=http://localhost:5173 pensieve serve
```

## Quick start

```bash
# 1. Install all workspace deps (from repo root)
pnpm install

# 2. Build the SDK packages first (workspace dep resolves to dist/)
pnpm --filter @pensieve-ai/client build
pnpm --filter @pensieve-ai/react build

# 3. Start the demo app
pnpm --filter pensieve-embed-demo dev
# → http://localhost:5173
```

Open the app, enter your endpoint + token, click **Connect**.

## Mint-server pattern (multi-tenant auth)

The "Use mint server" toggle demonstrates the production auth pattern: the host
app's backend mints a short-lived token so the raw credential is never exposed
to the browser.

In a separate terminal:

```bash
PENSIEVE_TOKEN=<your-token> pnpm --filter pensieve-embed-demo mint-token
# → http://localhost:8788/api/pensieve-token
```

Enable "Use mint server" in the UI and connect. The SDK will call
`GET http://localhost:8788/api/pensieve-token` before each request.

### OIDC environment variables (server-side)

| Variable | Default | Description |
|---|---|---|
| `PENSIEVE_OIDC_ISSUERS` | — | Comma-separated list of trusted OIDC issuer URLs |
| `PENSIEVE_OIDC_AUDIENCE` | `pensieve` | Expected `aud` claim value |
| `PENSIEVE_OIDC_ROLE_CLAIM` | `pensieve_role` | JWT claim carrying `admin`/`editor`/`viewer` |
| `PENSIEVE_OIDC_SUBJECT_CLAIM` | `sub` | JWT claim used as the audit identity |
| `PENSIEVE_OIDC_DATABASES_CLAIM` | `pensieve_databases` | JWT claim containing allowed database list (omit for full access) |

## Scoped-token caveats

When a token is scoped to specific databases (`pensieve_databases` non-empty):

- **Agent / Ask Pensieve** — returns 403 (fail-closed; agent needs cross-database access).
- **MCP endpoint** — same 403.
- **Arrow Flight** — same 403 (server-to-server only).
- **Live tail (SSE)** — not yet supported in embeds; planned for a future SDK version.

Use a full-access token (no `pensieve_databases` claim) when testing the **Agent** tab.

## Views

| Tab | Component | Key props exercised |
|---|---|---|
| Graph | `PensieveGraph` | `discover="all-databases"`, `onNodeClick`, `onSelectionChange` |
| Query | `PensieveQueryEditor` | `defaultQuery`, `showSchemaBrowser`, `onResults`, `onQueryChange` |
| Discover | `PensieveDiscover` | `defaultQuery`, `onRowOpen`, `onExportKql`, `onSearchChange` |
| Dashboards | `PensieveDashboard` | `dashboardId`, `onPanelClick`, `onSaveSuccess`, `onSaveError` |
| Agent | `PensieveAgentChat` | `placeholder`, `onMessage` |
| Headless | hooks only | `usePensieveGraph`, `usePensieveQuery`, `usePensieveClient` |

## CORS note

The Pensieve server must allow the origin the browser app runs on. For local dev:

```
PENSIEVE_CORS_ALLOWED_ORIGINS=http://localhost:5173
```

For production, set this to your actual host-app origin(s).
