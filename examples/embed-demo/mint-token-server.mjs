/**
 * mint-token-server.mjs — development token-minting server for the Pensieve embed demo.
 *
 * Listens on port 8788 (same default as Cloudflare Workers dev). Returns a
 * Pensieve bearer token on GET/POST /api/pensieve-token. Appropriate CORS headers
 * are set so the Vite dev server (port 5173) can call it from the browser.
 *
 * ── Production pattern ────────────────────────────────────────────────────────
 *
 * This dev server hands out a static PENSIEVE_TOKEN — never do that in production.
 *
 * In production, the host app's backend mints a SHORT-LIVED token per user:
 *
 *   1. Static token variant (simplest):
 *      Your auth middleware reads PENSIEVE_TOKEN (a long-lived server secret) and
 *      returns it only to authenticated users. Never expose the secret directly.
 *
 *   2. OIDC JWT variant (recommended for multi-tenant SaaS):
 *      Your IdP (Auth0, Okta, Keycloak, etc.) issues a JWT for the logged-in
 *      user. Configure the Pensieve server to trust that issuer:
 *
 *        PENSIEVE_OIDC_ISSUERS=https://your-idp.example.com
 *        PENSIEVE_OIDC_AUDIENCE=pensieve                      # optional, default "pensieve"
 *        PENSIEVE_OIDC_ROLE_CLAIM=pensieve_role               # claim in the JWT
 *        PENSIEVE_OIDC_SUBJECT_CLAIM=sub                  # used as audit identity
 *        PENSIEVE_OIDC_DATABASES_CLAIM=pensieve_databases     # optional; [] = no scope
 *
 *      The JWT must contain:
 *        { "pensieve_role": "admin" | "editor" | "viewer",
 *          "pensieve_databases": ["db1", "db2"]  // omit for full access
 *        }
 *
 *      Your server-side route (/api/pensieve-token) exchanges the user's session
 *      for a Pensieve-scoped JWT and returns it as plain text. The SDK calls
 *      getToken() before each request; short-lived tokens (e.g. 15 min) are
 *      refreshed transparently.
 *
 * ── Scoped-token caveats ─────────────────────────────────────────────────────
 *
 * When a token is scoped to specific databases (pensieve_databases is non-empty):
 *
 *   - Agent / Ask Pensieve endpoint: returns 403 (fail-closed by design; the agent
 *     has access to all databases for cross-source reasoning).
 *   - MCP endpoint: same 403 restriction.
 *   - Arrow Flight: same 403 restriction (server-to-server only anyway).
 *   - Live tail (SSE): not yet supported in embeds; planned for a future SDK version.
 *
 * Use a full-access token (no pensieve_databases claim) when testing the Agent tab.
 *
 * ── Usage ─────────────────────────────────────────────────────────────────────
 *
 *   PENSIEVE_TOKEN=<your-token> node mint-token-server.mjs
 *   # or via pnpm:
 *   PENSIEVE_TOKEN=<your-token> pnpm --filter pensieve-embed-demo mint-token
 *
 * The demo app's "Use mint server" toggle switches auth to:
 *   { getToken: () => fetch("http://localhost:8788/api/pensieve-token").then(r => r.text()) }
 */

import { createServer } from "node:http";

const PORT = 8788;
const ALLOWED_ORIGIN = process.env.PENSIEVE_DEMO_ORIGIN ?? "http://localhost:5173";
const TOKEN = process.env.PENSIEVE_TOKEN ?? "";

if (!TOKEN) {
  console.error("ERROR: PENSIEVE_TOKEN env var is required.");
  console.error("  Usage: PENSIEVE_TOKEN=<token> node mint-token-server.mjs");
  process.exit(1);
}

const CORS_HEADERS = {
  "Access-Control-Allow-Origin": ALLOWED_ORIGIN,
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type, Authorization",
  "Access-Control-Max-Age": "86400",
};

const server = createServer((req, res) => {
  // ── CORS preflight ──────────────────────────────────────────────────────────
  if (req.method === "OPTIONS") {
    res.writeHead(204, CORS_HEADERS);
    res.end();
    return;
  }

  const url = new URL(req.url ?? "/", `http://localhost:${PORT}`);

  if (url.pathname === "/api/pensieve-token" && (req.method === "GET" || req.method === "POST")) {
    res.writeHead(200, {
      ...CORS_HEADERS,
      "Content-Type": "text/plain; charset=utf-8",
      "Cache-Control": "no-store",
    });
    res.end(TOKEN);
    return;
  }

  res.writeHead(404, { ...CORS_HEADERS, "Content-Type": "text/plain" });
  res.end("Not found");
});

server.listen(PORT, () => {
  console.log(`pensieve mint-token-server running on http://localhost:${PORT}`);
  console.log(`  GET/POST /api/pensieve-token  →  returns PENSIEVE_TOKEN`);
  console.log(`  Allowed origin: ${ALLOWED_ORIGIN}`);
  console.log();
  console.log("Connect the demo app with 'Use mint server' enabled.");
});
