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
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["graph", "list", endpoint, database],
    queryFn: () => listGraphs({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    staleTime: 5 * 60_000,
  });
}

export function useGraphOverview(graph: string, realm?: string, limit = 800) {
  const { endpoint, token, database } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "overview", endpoint, database, graph, realm ?? null, limit],
    queryFn: () => getOverview({ endpoint, token, database, graph, realm, limit }),
    enabled: Boolean(endpoint && graph),
  });
}

export function useGraphStats(graph: string, realm?: string) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["graph", "stats", endpoint, database, graph, realm ?? null],
    queryFn: () => getStats({ endpoint, token, database, graph, realm }),
    enabled: Boolean(endpoint && graph),
  });
}

export function useGraphSchema(graph: string) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["graph", "schema", endpoint, database, graph],
    queryFn: () => getGraphSchema({ endpoint, token, database, graph }),
    enabled: Boolean(endpoint && graph),
    staleTime: 5 * 60_000,
  });
}

export function useGraphSubgraph(graph: string, id: string | null, depth = 2) {
  const { endpoint, token, database } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "subgraph", endpoint, database, graph, id, depth],
    queryFn: () => getSubgraph({ endpoint, token, database, graph, id: id!, depth }),
    enabled: Boolean(endpoint && graph && id),
  });
}

/** Imperative neighbor expansion bound to the current session. */
export function useExpandNeighbors(graph: string) {
  const { endpoint, token, database } = useSession();
  return useCallback(
    (nodeIds: string[], direction: Direction = "both"): Promise<EdgeExpansion> =>
      apiExpandNeighbors({ endpoint, token, database, graph, nodeIds, direction }),
    [endpoint, token, database, graph],
  );
}

/** Imperative node search bound to the current session. */
export function useSearchNodes(graph: string) {
  const { endpoint, token, database } = useSession();
  return useCallback(
    (text: string) => searchNodes({ endpoint, token, database, graph, text }),
    [endpoint, token, database, graph],
  );
}
