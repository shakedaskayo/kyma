/**
 * usePensieveDashboards — headless hook for dashboard listing and lookup.
 *
 * React Query wrapper over client.dashboards.listDashboards(). Exposes the
 * list for display and a getDashboard(id) helper for on-demand detail loads.
 *
 * CRUD mutations (create, update, delete) are intentionally omitted from this
 * hook — they belong to the edit flow which components can drive via the client
 * directly (usePensieveClient().dashboards.*). This keeps the headless hook minimal.
 *
 * Query key: ["pensieve", endpoint, database, "dashboards"]
 */

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import type { Dashboard, DashboardWithPanels } from "@pensieve-ai/client";
import { usePensieveClient } from "../provider/context";

// ── Public types ──────────────────────────────────────────────────────────────

export interface UsePensieveDashboardsResult {
  dashboards: Dashboard[];
  isLoading: boolean;
  error: unknown;
  /**
   * Fetch a single dashboard with its panels. Uses the React Query cache when
   * available; otherwise fetches from the server.
   */
  getDashboard: (id: string) => Promise<DashboardWithPanels>;
  refetch: () => void;
}

// ── Implementation ────────────────────────────────────────────────────────────

export function usePensieveDashboards(): UsePensieveDashboardsResult {
  const client = usePensieveClient();
  const endpoint = client.transport.endpoint;
  const database = client.transport.database ?? "default";
  const queryClient = useQueryClient();

  const queryKey = ["pensieve", endpoint, database, "dashboards"] as const;

  const query = useQuery<Dashboard[]>({
    queryKey,
    queryFn: () => client.dashboards.listDashboards(),
    staleTime: 30_000,
  });

  const getDashboard = useCallback(
    (id: string): Promise<DashboardWithPanels> => {
      const cacheKey = ["pensieve", endpoint, database, "dashboards", id];
      const cached = queryClient.getQueryData<DashboardWithPanels>(cacheKey);
      if (cached) return Promise.resolve(cached);
      return queryClient.fetchQuery({
        queryKey: cacheKey,
        queryFn: () => client.dashboards.getDashboard({ id }),
        staleTime: 30_000,
      });
    },
    [client, endpoint, database, queryClient],
  );

  const refetch = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey });
  }, [queryClient, queryKey]);

  return {
    dashboards: query.data ?? [],
    isLoading: query.isLoading,
    error: query.error ?? null,
    getDashboard,
    refetch,
  };
}
