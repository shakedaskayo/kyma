import { useQuery } from "@tanstack/react-query";
import { useCallback } from "react";
import { useSession } from "@/sdk/session";
import {
  expandNeighbors as apiExpandNeighbors,
  getGraphSchema,
  getOverview,
  getStats,
  getSubgraph,
  listGraphs,
  searchNodes,
  type Direction,
  type EdgeExpansion,
  type GraphPayload,
} from "@/sdk/graph";

export function useGraphList() {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "list", endpoint],
    queryFn: () => listGraphs({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 5 * 60_000,
  });
}

export function useGraphOverview(graph: string, realm?: string, limit = 800) {
  const { endpoint, token } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "overview", endpoint, graph, realm ?? null, limit],
    queryFn: () => getOverview({ endpoint, token, graph, realm, limit }),
    enabled: Boolean(endpoint && token && graph),
  });
}

export function useGraphStats(graph: string, realm?: string) {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "stats", endpoint, graph, realm ?? null],
    queryFn: () => getStats({ endpoint, token, graph, realm }),
    enabled: Boolean(endpoint && token && graph),
  });
}

export function useGraphSchema(graph: string) {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "schema", endpoint, graph],
    queryFn: () => getGraphSchema({ endpoint, token, graph }),
    enabled: Boolean(endpoint && token && graph),
    staleTime: 5 * 60_000,
  });
}

export function useGraphSubgraph(graph: string, id: string | null, depth = 2) {
  const { endpoint, token } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "subgraph", endpoint, graph, id, depth],
    queryFn: () => getSubgraph({ endpoint, token, graph, id: id!, depth }),
    enabled: Boolean(endpoint && token && graph && id),
  });
}

/** Imperative neighbor expansion bound to the current session. */
export function useExpandNeighbors(graph: string) {
  const { endpoint, token } = useSession();
  return useCallback(
    (nodeIds: string[], direction: Direction = "both"): Promise<EdgeExpansion> =>
      apiExpandNeighbors({ endpoint, token, graph, nodeIds, direction }),
    [endpoint, token, graph],
  );
}

/** Imperative node search bound to the current session. */
export function useSearchNodes(graph: string) {
  const { endpoint, token } = useSession();
  return useCallback(
    (text: string) => searchNodes({ endpoint, token, graph, text }),
    [endpoint, token, graph],
  );
}
