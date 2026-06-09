---
title: CORS — @kyma-ai/react
description: Configure KYMA_CORS_ALLOWED_ORIGINS on the Kyma server to allow browser requests from your host application.
---

# CORS

The browser enforces the same-origin policy, so your Kyma server must emit
the correct CORS headers for the origin your host application runs on. The
server reads a single environment variable to control this.

## `KYMA_CORS_ALLOWED_ORIGINS`

| State | Behaviour |
|---|---|
| Unset | Permissive: every request origin is reflected back in `Access-Control-Allow-Origin`. **Dev only** — a warning is logged. |
| Set to a valid comma-separated list of origins | Restrictive: only the listed origins are accepted. |
| Set but contains no parseable origins | **Fail-closed**: an empty allow-list is used — no cross-origin request succeeds. A server-side error is logged. |

The fail-closed behaviour for a misconfigured production value is intentional.
A typo must not silently open the API to all origins.

## Local development

Start the Kyma server with the Vite dev server's origin:

```bash
KYMA_CORS_ALLOWED_ORIGINS=http://localhost:5173 kyma serve
# or via Docker Compose:
KYMA_CORS_ALLOWED_ORIGINS=http://localhost:5173 docker compose up
```

## Production

Set the variable to your deployed host-app origin(s):

```bash
# Single origin
KYMA_CORS_ALLOWED_ORIGINS=https://app.acme.com

# Multiple origins (comma-separated, no spaces needed)
KYMA_CORS_ALLOWED_ORIGINS=https://app.acme.com,https://staging.acme.com
```

The Kyma server also allows any method, header, and exposes all response
headers so SSE streams, the `Authorization` header, and custom Kyma headers
(`x-database`, `x-kyma-request-id`) all flow through.

## Interaction with `KymaProvider`

`KymaProvider` sends all API requests from the browser using `fetch`. The
`endpoint` you pass must be reachable from the origin the browser runs on.
Proxy the server through your own ingress if you need to avoid configuring
CORS (e.g. `/api/kyma` → the Kyma server).
