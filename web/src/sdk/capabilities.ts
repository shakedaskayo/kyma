// Feature discovery (`/v1/capabilities`) — which surfaces this server has.
//
// Local mode (`kyma serve`, embedded SQLite) deliberately omits the
// control-plane surfaces (connectors, credentials, OAuth, saved Discover
// views); pages gate on these flags and explain instead of hitting 404s.
// Servers that predate the endpoint 404 here — we then assume the full
// hosted feature set, which matches every pre-capabilities deployment.

import { useQuery } from "@tanstack/react-query";
import { authFetch } from "./auth-fetch";
import { useSession } from "./session";

export type Capabilities = {
  mode: "local" | "server";
  connectors: boolean;
  credentials: boolean;
  oauth: boolean;
  saved_views: boolean;
  users_admin: boolean;
};

export const FULL_CAPABILITIES: Capabilities = {
  mode: "server",
  connectors: true,
  credentials: true,
  oauth: true,
  saved_views: true,
  users_admin: true,
};

export async function fetchCapabilities(): Promise<Capabilities> {
  try {
    const res = await authFetch("/v1/capabilities");
    if (!res.ok) return FULL_CAPABILITIES;
    const body = (await res.json()) as Partial<Capabilities>;
    return { ...FULL_CAPABILITIES, ...body };
  } catch {
    return FULL_CAPABILITIES;
  }
}

/** The connected server's capabilities; optimistic (full) until loaded. */
export function useCapabilities(): Capabilities {
  const endpoint = useSession((s) => s.endpoint);
  const token = useSession((s) => s.token);
  const { data } = useQuery({
    queryKey: ["capabilities", endpoint],
    queryFn: fetchCapabilities,
    enabled: Boolean(endpoint && token),
    staleTime: Infinity,
    retry: false,
  });
  return data ?? FULL_CAPABILITIES;
}
