// Feature discovery (`/v1/capabilities`) — which surfaces this server has.
//
// Local mode (`kyma serve`, embedded SQLite) deliberately omits the
// control-plane surfaces (connectors, credentials, OAuth, saved Discover
// views); pages gate on these flags and explain instead of hitting 404s.
// Servers that predate the endpoint 404 here — we then assume the full
// hosted feature set, which matches every pre-capabilities deployment.
//
// NOTE: The `useCapabilities` React hook from the original web/src/sdk version
// depended on `./session` (web-only Zustand store) and `@tanstack/react-query`.
// Those dependencies are NOT included here; the hook lives in web/src/sdk.
// Only the framework-agnostic `fetchCapabilities` function is exported.

import type { KymaTransport } from "./transport";

export type Capabilities = {
  mode: "local" | "server";
  connectors: boolean;
  credentials: boolean;
  oauth: boolean;
  saved_views: boolean;
  users_admin: boolean;
  /** Live-tail WebSocket (`GET /v1/explore/live`). */
  explore_live: boolean;
};

export const FULL_CAPABILITIES: Capabilities = {
  mode: "server",
  connectors: true,
  credentials: true,
  oauth: true,
  saved_views: true,
  users_admin: true,
  explore_live: true,
};

export async function fetchCapabilities(t: KymaTransport): Promise<Capabilities> {
  try {
    const res = await t.request("/v1/capabilities");
    if (!res.ok) return FULL_CAPABILITIES;
    const body = (await res.json()) as Partial<Capabilities>;
    return { ...FULL_CAPABILITIES, ...body };
  } catch {
    return FULL_CAPABILITIES;
  }
}
