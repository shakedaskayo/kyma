/**
 * mint-token-server.mjs — development token-minting server for the Kyma embed demo.
 *
 * Listens on port 8788 (same default as Cloudflare Workers dev). Returns a
 * Kyma bearer token on GET/POST /api/kyma-token. Appropriate CORS headers
 * are set so the Vite dev server (port 5173) can call it from the browser.
 *
 * ── Production pattern ────────────────────────────────────────────────────────
 *
 * This dev server hands out a static KYMA_TOKEN — never do that in production.
 *
 * In production, the host app's backend mints a SHORT-LIVED token per user:
 *
 *   1. Static token variant (simplest):
 *      Your auth middleware reads KYMA_TOKEN (a long-lived server secret) and
 *      returns it only to authenticated users. Never expose the secret directly.
 *
 *   2. OIDC JWT variant (recommended for multi-tenant SaaS):
 *      Your IdP (Auth0, Okta, Keycloak, etc.) issues a JWT for the logged-in
 *      user. Configure the Kyma server to trust that issuer:
 *
 *        KYMA_OIDC_ISSUERS=https://your-idp.example.com
 *        KYMA_OIDC_AUDIENCE=kyma                      # optional, default "kyma"
 *        KYMA_OIDC_ROLE_CLAIM=kyma_role               # claim in the JWT
 *        KYMA_OIDC_SUBJECT_CLAIM=sub                  # used as audit identity
 *        KYMA_OIDC_DATABASES_CLAIM=kyma_databases     # optional; [] = no scope
 *
 *      The JWT must contain:
 *        { "kyma_role": "admin" | "editor" | "viewer",
 *          "kyma_databases": ["db1", "db2"]  // omit for full access
 *        }
 *
 *      Your server-side route (/api/kyma-token) exchanges the user's session
 *      for a Kyma-scoped JWT and returns it as plain text. The SDK calls
 *      getToken() before each request; short-lived tokens (e.g. 15 min) are
 *      refreshed transparently.
 *
 * ── Scoped-token caveats ─────────────────────────────────────────────────────
 *
 * When a token is scoped to specific databases (kyma_databases is non-empty):
 *
 *   - Agent / Ask Kyma endpoint: returns 403 (fail-closed by design; the agent
 *     has access to all databases for cross-source reasoning).
 *   - MCP endpoint: same 403 restriction.
 *   - Arrow Flight: same 403 restriction (server-to-server only anyway).
 *   - Live tail (SSE): not yet supported in embeds; planned for a future SDK version.
 *
 * Use a full-access token (no kyma_databases claim) when testing the Agent tab.
 *
 * ── Usage ─────────────────────────────────────────────────────────────────────
 *
 *   KYMA_TOKEN=<your-token> node mint-token-server.mjs
 *   # or via pnpm:
 *   KYMA_TOKEN=<your-token> pnpm --filter kyma-embed-demo mint-token
 *
 * The demo app's "Use mint server" toggle switches auth to:
 *   { getToken: () => fetch("http://localhost:8788/api/kyma-token").then(r => r.text()) }
 */

import { createServer } from "node:http";

const PORT = 8788;
const ALLOWED_ORIGIN = process.env.KYMA_DEMO_ORIGIN ?? "http://localhost:5173";
const TOKEN = process.env.KYMA_TOKEN ?? "";

if (!TOKEN) {
  console.error("ERROR: KYMA_TOKEN env var is required.");
  console.error("  Usage: KYMA_TOKEN=<token> node mint-token-server.mjs");
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

  if (url.pathname === "/api/kyma-token" && (req.method === "GET" || req.method === "POST")) {
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
  console.log(`kyma mint-token-server running on http://localhost:${PORT}`);
  console.log(`  GET/POST /api/kyma-token  →  returns KYMA_TOKEN`);
  console.log(`  Allowed origin: ${ALLOWED_ORIGIN}`);
  console.log();
  console.log("Connect the demo app with 'Use mint server' enabled.");
});
