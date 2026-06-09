---
title: Quickstart — embed Kyma in React
description: Install @kyma-ai/react, wire KymaProvider, and render your first embedded view in under 20 lines.
---

# Quickstart

## Install

```bash
npm install @kyma-ai/react @kyma-ai/client
# or
pnpm add @kyma-ai/react @kyma-ai/client
```

`@kyma-ai/react` ships pre-built ESM. Peer dependencies — React and React DOM
18+ — must already be present in your project.

```bash
# peer deps (if not already installed)
npm install react react-dom
```

## Import the stylesheet

Kyma's components use scoped Tailwind CSS compiled into a single stylesheet.
Import it once at the top level of your application:

```ts
import "@kyma-ai/react/styles.css";
```

## Add the provider and a view

```tsx
import { KymaProvider } from "@kyma-ai/react";
import { KymaGraph }    from "@kyma-ai/react/graph";
import "@kyma-ai/react/styles.css";

export function KymaPanel() {
  return (
    <KymaProvider
      endpoint="https://kyma.acme.internal"
      auth={{ token: "ky-abc123" }}
      database="prod"
    >
      <div style={{ height: 600 }}>
        <KymaGraph />
      </div>
    </KymaProvider>
  );
}
```

That is the complete setup. `KymaProvider` creates an isolated React Query
client and injects the theme tokens; `KymaGraph` loads graph data from the
connected server and renders it.

## What each import does

| Import | Purpose | Bundle note |
|---|---|---|
| `@kyma-ai/react` | `KymaProvider`, hooks, utilities | Core; no heavy UI deps |
| `@kyma-ai/react/graph` | `KymaGraph` | Adds `react-force-graph-2d` (WebGL canvas) |
| `@kyma-ai/react/query` | `KymaQueryEditor` | Adds `@monaco-editor/react` (Monaco) |
| `@kyma-ai/react/discover` | `KymaDiscover` | Streaming search UI; no extra heavy deps |
| `@kyma-ai/react/dashboards` | `KymaDashboard` | Adds `react-grid-layout` + `echarts` |
| `@kyma-ai/react/agent` | `KymaAgentChat` | SSE streaming chat; no extra heavy deps |
| `@kyma-ai/react/styles.css` | All component CSS | Import once per app |

Each subpath is a separate bundle entry, so if you only render `KymaGraph`,
Monaco and ECharts are never loaded.

## Next steps

- [Authentication](/embed/authentication) — configure tokens or OIDC
- [CORS](/embed/cors) — allow the browser to reach the Kyma server
- [Theming](/embed/theming) — match your brand
- [Components](/embed/components) — full prop tables for every component
