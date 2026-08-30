---
title: Server compatibility — @pensieve-ai/react
description: Feature detection via /v1/capabilities, capability gating, default-on behavior for pre-capabilities servers, and known embed limitations.
---

# Server compatibility

## `GET /v1/capabilities` feature detection

The SDK fetches `GET /v1/capabilities` once per `PensieveProvider` mount and
caches the result for 5 minutes (`usePensieveCapabilities`). The server returns a
JSON object with boolean feature flags:

```json
{
  "mode": "server",
  "data_sources": true,
  "credentials": true,
  "oauth": true,
  "saved_views": true,
  "users_admin": false,
  "explore_live": true,
  "agent": true
}
```

These flags gate optional UI surfaces in the components. The `agent` flag
specifically controls whether `PensieveAgentChat` shows the chat UI or a "not
available on this server" card.

## `CapabilityGate` — fail-open design

The `CapabilityGate` internal component renders its children only when a
specific capability flag is `true`. While capabilities are loading or on a
transient fetch error (one retry allowed), the gate **fails open**: children
are rendered as if the server supports the capability. This ensures that
pre-capabilities servers (which do not return this endpoint at all) and servers
with transient network issues do not silently blank out components.

For servers that actively return `{ "agent": false }`, `PensieveAgentChat` renders
a "not available on this server" informational card rather than the chat UI.

## Pre-capabilities servers (legacy behaviour)

Servers predating the `/v1/capabilities` endpoint serve the SPA HTML for
unknown `/v1/*` paths. The transport detects this:

```ts
// From packages/client/src/transport.ts
if (res.ok && url.includes("/v1/") && (res.headers.get("content-type") ?? "").includes("text/html")) {
  return new Response(JSON.stringify({ error: "endpoint not available on this server" }), {
    status: 404,
    headers: { "content-type": "application/json" },
  });
}
```

The 404 response causes `usePensieveCapabilities` to error after one retry. The
gate fails open, so all components render as if the server supports every
capability. This is the correct default: old servers without a capability
endpoint still support the full surface (they predate the flag, not the feature).

## Known limitations in embeds

| Feature | Status | Notes |
|---|---|---|
| **Live tail (SSE)** | Not in embeds | `discover_live` / `LiveSession` is available in `@pensieve-ai/client` but not exposed in any embedded component today. Planned for a future SDK version. |
| **PensieveAgentChat with scoped tokens** | 403 (fail-closed) | The agent's tool loop addresses databases internally; fine-grained enforcement is roadmap. Use a full-access token. |
| **MCP with scoped tokens** | 403 | Same restriction as the agent. |
| **Arrow Flight with scoped tokens** | 403 | Server-to-server only anyway. |
| **Database-scope enforcement in agent/MCP** | Roadmap | When the tool context enforces `allowed_databases`, scoped tokens will be accepted on these surfaces. |

## Checking server version compatibility

No formal minimum server version is declared for `@pensieve-ai/react` 0.x. The SDK
handles missing capabilities gracefully. If you encounter 404s or unexpected
shapes from the server, check whether your server build is recent enough to
support the API surface you are using. The `GET /health` endpoint returns the
server build commit and version.
