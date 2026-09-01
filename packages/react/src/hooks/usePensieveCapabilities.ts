/**
 * usePensieveCapabilities — React Query wrapper over client.capabilities.fetchCapabilities().
 *
 * Returns the TanStack Query result. The data shape is Capabilities from
 * @pensieve-ai/client/capabilities — a flat object with boolean feature flags:
 *   { mode, data_sources, credentials, oauth, saved_views, users_admin, explore_live }
 *
 * staleTime: 5 minutes — capabilities change only on server deploys, not per-interaction.
 * retry: 1 — one retry on transient failure; after that the gate fails open (see CapabilityGate).
 */

import { useQuery } from "@tanstack/react-query";
import type { Capabilities } from "@pensieve-ai/client";
import { usePensieveContext } from "../provider/context";

export function usePensieveCapabilities() {
  const { client } = usePensieveContext();
  const endpoint = client.transport.endpoint;

  return useQuery<Capabilities>({
    queryKey: ["pensieve", endpoint, "capabilities"],
    queryFn: () => client.capabilities.fetchCapabilities(),
    staleTime: 5 * 60_000,
    retry: 1,
  });
}
