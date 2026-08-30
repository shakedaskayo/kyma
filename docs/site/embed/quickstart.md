---
title: Quickstart — embed Pensieve in React
description: Install @pensieve-ai/react, wire PensieveProvider, and render your first embedded view in under 20 lines.
---

# Quickstart

## Install

```bash
npm install @pensieve-ai/react @pensieve-ai/client
# or
pnpm add @pensieve-ai/react @pensieve-ai/client
```

`@pensieve-ai/react` ships pre-built ESM. Peer dependencies — React and React DOM
18+ — must already be present in your project.

```bash
# peer deps (if not already installed)
npm install react react-dom
```

## Import the stylesheet

Pensieve's components use scoped Tailwind CSS compiled into a single stylesheet.
Import it once at the top level of your application:

```ts
import "@pensieve-ai/react/styles.css";
```

## Add the provider and a view

```tsx
import { PensieveProvider } from "@pensieve-ai/react";
import { PensieveGraph }    from "@pensieve-ai/react/graph";
import "@pensieve-ai/react/styles.css";

export function PensievePanel() {
  return (
    <PensieveProvider
      endpoint="https://pensieve.acme.internal"
      auth={{ token: "pv-abc123" }}
      database="prod"
    >
      <div style={{ height: 600 }}>
        <PensieveGraph />
      </div>
    </PensieveProvider>
  );
}
```

That is the complete setup. `PensieveProvider` creates an isolated React Query
client and injects the theme tokens; `PensieveGraph` loads graph data from the
connected server and renders it.

## What each import does

| Import | Purpose | Bundle note |
|---|---|---|
| `@pensieve-ai/react` | `PensieveProvider`, hooks, utilities | Core; no heavy UI deps |
| `@pensieve-ai/react/graph` | `PensieveGraph` | Adds `react-force-graph-2d` (WebGL canvas) |
| `@pensieve-ai/react/query` | `PensieveQueryEditor` | Adds `@monaco-editor/react` (Monaco) |
| `@pensieve-ai/react/discover` | `PensieveDiscover` | Streaming search UI; no extra heavy deps |
| `@pensieve-ai/react/dashboards` | `PensieveDashboard` | Adds `react-grid-layout` + `echarts` |
| `@pensieve-ai/react/agent` | `PensieveAgentChat` | SSE streaming chat; no extra heavy deps |
| `@pensieve-ai/react/styles.css` | All component CSS | Import once per app |

Each subpath is a separate bundle entry, so if you only render `PensieveGraph`,
Monaco and ECharts are never loaded.

## Next steps

- [Authentication](/embed/authentication) — configure tokens or OIDC
- [CORS](/embed/cors) — allow the browser to reach the Pensieve server
- [Theming](/embed/theming) — match your brand
- [Components](/embed/components) — full prop tables for every component
