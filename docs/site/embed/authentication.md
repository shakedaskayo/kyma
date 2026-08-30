---
title: Authentication — @pensieve-ai/react
description: Static tokens, server-side token minting, and full OIDC JWT setup for embedded Pensieve views.
---

# Authentication

`PensieveProvider` takes an `auth` prop of type `PensieveAuth` (from `@pensieve-ai/client`):

```ts
type PensieveAuth =
  | { token: string }                                      // static token
  | { getToken: (opts?: { reason: "initial" | "expired" }) => Promise<string> };
```

## Static token

The simplest option: pass a bearer token directly. Use this for internal tools,
demos, or local development.

```tsx
<PensieveProvider
  endpoint="http://localhost:8080"
  auth={{ token: "ky-your-static-token" }}
>
  {children}
</PensieveProvider>
```

The transport automatically adds `Authorization: Bearer <token>` to every
request. On a 401 response, static tokens throw immediately — there is no
retry loop.

## Dynamic token with `getToken` (server-side minting)

For production, your server holds the Pensieve credential and issues short-lived
tokens to authenticated browser sessions. The SDK calls `getToken` before the
first request and again whenever a 401 signals expiry. Concurrent in-flight
requests coalesce onto a single `getToken` call rather than firing N parallel
mints.

The transport also watches the `exp` claim of any JWT it holds: if expiry is
within 60 seconds it proactively calls `getToken({ reason: "expired" })` before
the next request, so users rarely see a 401.

```tsx
<PensieveProvider
  endpoint="https://pensieve.acme.internal"
  auth={{
    getToken: async ({ reason } = {}) => {
      // Call your app's backend to get a short-lived Pensieve token.
      // `reason` is "initial" on first load, "expired" on refresh.
      const res = await fetch("/api/pensieve-token", {
        headers: { "x-refresh-reason": reason ?? "initial" },
      });
      if (!res.ok) throw new Error("Token mint failed");
      return res.text();
    },
  }}
>
  {children}
</PensieveProvider>
```

The reference mint server (`examples/embed-demo/mint-token-server.mjs`) shows
the full Node.js implementation. See `examples/embed-demo/README.md` for
setup instructions.

## Full OIDC setup

Pensieve can validate JWTs issued by any standards-compliant OIDC provider (Auth0,
Okta, Keycloak, Cognito, etc.) without a custom token mint: the provider issues
the JWT directly and the SDK passes it as the bearer token.

### Server environment variables

Set these on the Pensieve server process:

| Variable | Default | Description |
|---|---|---|
| `PENSIEVE_OIDC_ISSUERS` | — | Comma-separated list of trusted issuer URLs. **Required** to enable OIDC. Trailing slashes are stripped before matching. |
| `PENSIEVE_OIDC_AUDIENCE` | `pensieve` | Expected `aud` claim value in the JWT. |
| `PENSIEVE_OIDC_ROLE_CLAIM` | `pensieve_role` | JWT claim carrying the role string. |
| `PENSIEVE_OIDC_SUBJECT_CLAIM` | `sub` | JWT claim used as the audit identity (user ID). |
| `PENSIEVE_OIDC_DATABASES_CLAIM` | `pensieve_databases` | JWT claim containing the allowed-database list. Omit the claim for full (unscoped) access. |

OIDC is **disabled** when `PENSIEVE_OIDC_ISSUERS` is unset or empty. When it is
set, non-JWT (opaque) tokens fall through to the native Pensieve token backend so
existing tokens keep working.

JWKS are fetched via `{issuer}/.well-known/openid-configuration` on first use,
then cached for one hour. An unknown `kid` triggers a forced refresh (rate-limited
to one attempt per 30 seconds).

**HTTPS is required** for non-loopback issuers. HTTP issuers are only accepted
for `localhost`, `127.0.0.1`, and `::1`.

### JWT claim contract

A valid Pensieve OIDC token must contain:

```json
{
  "iss": "https://your-idp.example.com",
  "aud": "pensieve",
  "exp": 1893456000,
  "sub": "user-uuid-or-email",
  "pensieve_role": "read",
  "pensieve_databases": ["prod", "staging"]
}
```

| Claim | Required | Values / semantics |
|---|---|---|
| `iss` | yes | Must match one of `PENSIEVE_OIDC_ISSUERS` (trailing slashes tolerated in the token). |
| `aud` | yes | Must equal `PENSIEVE_OIDC_AUDIENCE` (default `pensieve`). |
| `exp` | yes | Standard Unix timestamp; expired tokens are rejected. |
| `nbf` | no | If present, the token is rejected before this time. |
| `sub` | no | Recorded in audit logs. Defaults to `PENSIEVE_OIDC_SUBJECT_CLAIM`. |
| `pensieve_role` | no | `"read"` / `"write"` / `"admin"`. Missing → `"read"`. |
| `pensieve_databases` | no | JSON array of database names the token may access. Missing → all databases. |

### Role semantics

| Role | Permitted operations |
|---|---|
| `read` | GET / query operations; no writes, no admin |
| `write` | Ingest, create/update dashboards, manage data sources |
| `admin` | Everything, including user and engine management |

Roles are ordered: `read < write < admin`. A `write` token satisfies any `read`
check; an `admin` token satisfies any check.

### Database scoping semantics

When `pensieve_databases` is present and non-empty, every request's `x-database`
header is checked against the list. Requests to unlisted databases receive 403.

When `pensieve_databases` is absent from the token, access is **unrestricted** —
the token may address any database the server hosts.

**Fail-closed surfaces.** Three surfaces return 403 for any database-scoped
token regardless of which databases are listed, because they resolve the target
database internally rather than from the request header:

| Surface | Path | Reason |
|---|---|---|
| Agent / Ask Pensieve | `POST /v1/agent/ask` | The agent's tool loop can address any database; fine-grained enforcement is roadmap. |
| MCP | `/mcp` | Same tool-dispatch model as the agent. |
| Arrow Flight | `/flight/*` | Flight tickets embed the database name; server-to-server only. |

Use a full-access token (no `pensieve_databases` claim) when embedding
`PensieveAgentChat`. All other components work with scoped tokens.

### Issuer trailing-slash note

`PENSIEVE_OIDC_ISSUERS` values are normalised by stripping trailing slashes.
Tokens may carry the issuer with or without a trailing slash — both match
the same normalised allowlist entry. Auth0, for example, typically appends
a trailing slash; Okta does not. Both work.

### React integration with OIDC

Your `getToken` function retrieves the user's OIDC access token from your IdP
SDK and returns it:

```tsx
import { useAuth } from "your-idp-sdk";

function App() {
  const { getAccessToken } = useAuth();

  return (
    <PensieveProvider
      endpoint="https://pensieve.acme.internal"
      auth={{
        getToken: () => getAccessToken({ audience: "pensieve" }),
      }}
    >
      {children}
    </PensieveProvider>
  );
}
```

The transport handles proactive refresh and concurrent-request coalescing
automatically; you only need to return a valid token.
