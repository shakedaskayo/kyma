// Web-side capabilities hook. Framework-agnostic fetchCapabilities lives in
// @pensieve-ai/client; this file adds the React/react-query wrapper and re-exports
// the types that web components import.

import { useQuery } from "@tanstack/react-query";
import { fetchCapabilities, FULL_CAPABILITIES } from "@pensieve-ai/client";
import { useSession } from "./session";
import { sessionClient } from "./client";

// Re-export types so ControlPlaneGate and Sidebar imports remain unchanged.
export type { Capabilities } from "@pensieve-ai/client";
export { FULL_CAPABILITIES } from "@pensieve-ai/client";

/** The connected server's capabilities; optimistic (full) until loaded. */
export function useCapabilities() {
  const endpoint = useSession((s) => s.endpoint);
  const token = useSession((s) => s.token);

  const { data } = useQuery({
    queryKey: ["capabilities", endpoint],
    queryFn: () => fetchCapabilities(sessionClient().transport),
    enabled: Boolean(endpoint && token),
    staleTime: Infinity,
    retry: false,
  });

  return data ?? FULL_CAPABILITIES;
}
