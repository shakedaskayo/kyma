# kyma-embed-demo

Reference host app for the `@kyma-ai/react` embeddable SDK. Demonstrates all
five Kyma views (Graph, Query, Discover, Dashboards, Agent) plus the headless
hooks layer (L3), theme switching, and the multi-tenant mint-server auth pattern.

## Prerequisites

- Node 20+ / pnpm 9+
- A running Kyma server (default `http://localhost:8080`)
- A valid Kyma bearer token
- Server started with CORS open to the Vite dev port:

```
KYMA_CORS_ALLOWED_ORIGINS=http://localhost:5173 kyma serve
```

## Quick start

```bash
# 1. Install all workspace deps (from repo root)
pnpm install

# 2. Build the SDK packages first (workspace dep resolves to dist/)
pnpm --filter @kyma-ai/client build
pnpm --filter @kyma-ai/react build

# 3. Start the demo app
pnpm --filter kyma-embed-demo dev
# → http://localhost:5173
```

Open the app, enter your endpoint + token, click **Connect**.

## Mint-server pattern (multi-tenant auth)

The "Use mint server" toggle demonstrates the production auth pattern: the host
app's backend mints a short-lived token so the raw credential is never exposed
to the browser.

In a separate terminal:

```bash
KYMA_TOKEN=<your-token> pnpm --filter kyma-embed-demo mint-token
# → http://localhost:8788/api/kyma-token
```

Enable "Use mint server" in the UI and connect. The SDK will call
`GET http://localhost:8788/api/kyma-token` before each request.

### OIDC environment variables (server-side)

| Variable | Default | Description |
|---|---|---|
| `KYMA_OIDC_ISSUERS` | — | Comma-separated list of trusted OIDC issuer URLs |
| `KYMA_OIDC_AUDIENCE` | `kyma` | Expected `aud` claim value |
| `KYMA_OIDC_ROLE_CLAIM` | `kyma_role` | JWT claim carrying `admin`/`editor`/`viewer` |
| `KYMA_OIDC_SUBJECT_CLAIM` | `sub` | JWT claim used as the audit identity |
| `KYMA_OIDC_DATABASES_CLAIM` | `kyma_databases` | JWT claim containing allowed database list (omit for full access) |

## Scoped-token caveats

When a token is scoped to specific databases (`kyma_databases` non-empty):

- **Agent / Ask Kyma** — returns 403 (fail-closed; agent needs cross-database access).
- **MCP endpoint** — same 403.
- **Arrow Flight** — same 403 (server-to-server only).
- **Live tail (SSE)** — not yet supported in embeds; planned for a future SDK version.

Use a full-access token (no `kyma_databases` claim) when testing the **Agent** tab.

## Views

| Tab | Component | Key props exercised |
|---|---|---|
| Graph | `KymaGraph` | `discover="all-databases"`, `onNodeClick`, `onSelectionChange` |
| Query | `KymaQueryEditor` | `defaultQuery`, `showSchemaBrowser`, `onResults`, `onQueryChange` |
| Discover | `KymaDiscover` | `defaultQuery`, `onRowOpen`, `onExportKql`, `onSearchChange` |
| Dashboards | `KymaDashboard` | `dashboardId`, `onPanelClick`, `onSaveSuccess`, `onSaveError` |
| Agent | `KymaAgentChat` | `placeholder`, `onMessage` |
| Headless | hooks only | `useKymaGraph`, `useKymaQuery`, `useKymaClient` |

## CORS note

The Kyma server must allow the origin the browser app runs on. For local dev:

```
KYMA_CORS_ALLOWED_ORIGINS=http://localhost:5173
```

For production, set this to your actual host-app origin(s).
