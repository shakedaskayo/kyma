import { useQueries, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
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
  type GraphNode,
  type GraphPayload,
  type GraphRef,
  type GraphRelationship,
  type GraphStats,
} from "@/sdk/graph";

/** A graph identified by (database, graph-name) — same name can exist in multiple DBs. */
export interface GraphCoord {
  database: string;
  graph: string;
  kind?: string;
}

/** Composite key used as the "namespace" identifier in the unified view. */
export function graphKey(c: { database: string; graph: string }): string {
  return `${c.database}/${c.graph}`;
}

/** A union of several graphs ("namespaces") merged into one canvas. */
export interface UnifiedGraph {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  stats: GraphStats;
  /** Node count contributed by each namespace (db/graph composite key). */
  namespaceCounts: Record<string, number>;
}

/** Per-namespace fetch state — surfaces granular progress to the UI. */
export interface UnifiedGraphProgress {
  /** Total (db, graph) pairs being fetched. */
  total: number;
  /** How many have settled (succeeded or failed). */
  settled: number;
  /** Whether anything has data yet (false = first paint pending). */
  hasAny: boolean;
}

/**
 * Stream-fetch graphs from any number of (database, graph) pairs and merge
 * them into one payload as each per-namespace overview lands. Each node/edge
 * is tagged with its source `database` and the composite `namespace`
 * (`db/graph`) so the legend can disambiguate same-named graphs across
 * databases (e.g. every DB has a `schema` graph).
 *
 * Why `useQueries` instead of one big `useQuery`: the previous shape waited
 * for *every* fetch before showing anything. On a deployment with many DBs
 * that meant the canvas sat on a spinner until the slowest server replied.
 * With `useQueries` each fetch is independent — react-query caches them
 * separately, retries them separately, and the merge below recomputes on
 * every partial completion so the user sees the first DB's graph land while
 * the rest are still in flight.
 */
export function useUnifiedGraph(
  coords: GraphCoord[],
  limit = 800,
): {
  data: UnifiedGraph | undefined;
  isLoading: boolean;
  isError: boolean;
  error: Error | null;
  progress: UnifiedGraphProgress;
} {
  const { endpoint, token } = useSession();

  const queries = useQueries({
    queries: coords.map(({ database, graph }) => ({
      queryKey: ["graph", "overview", endpoint, database, graph, limit] as const,
      queryFn: () => getOverview({ endpoint, token, database, graph, limit }),
      enabled: Boolean(endpoint),
      staleTime: 30_000,
    })),
  });

  // Recompute the merged view on every per-namespace state change. The merge
  // is O(total nodes+edges) so it's cheap relative to the network cost; we
  // keep it as a useMemo over the queries' data identity so React doesn't
  // re-merge on unrelated re-renders.
  const data = useMemo<UnifiedGraph | undefined>(() => {
    let touched = false;
    const nodes: GraphNode[] = [];
    const edges: GraphRelationship[] = [];
    const seenNodes = new Set<string>();
    const seenEdges = new Set<string>();
    const label_counts: Record<string, number> = {};
    const relationship_type_counts: Record<string, number> = {};
    const namespaceCounts: Record<string, number> = {};

    coords.forEach((c, i) => {
      const payload = queries[i]?.data;
      if (!payload) return;
      touched = true;
      const ns = graphKey(c);
      // Node ids are unique within a (database, graph), not across. Scope the
      // dedupe by namespace so an id of "1" in db-a and db-b stays distinct.
      for (const n of payload.nodes) {
        const k = `${ns}::${n.id}`;
        if (seenNodes.has(k)) continue;
        seenNodes.add(k);
        nodes.push({ ...n, namespace: ns, database: c.database });
        namespaceCounts[ns] = (namespaceCounts[ns] ?? 0) + 1;
        const label = n.labels[0] ?? "Node";
        label_counts[label] = (label_counts[label] ?? 0) + 1;
      }
      for (const e of payload.edges) {
        const k = `${ns}::${e.id}`;
        if (seenEdges.has(k)) continue;
        seenEdges.add(k);
        edges.push({ ...e, namespace: ns, database: c.database });
        relationship_type_counts[e.relationship_type] =
          (relationship_type_counts[e.relationship_type] ?? 0) + 1;
      }
    });

    if (!touched) return undefined;
    return {
      nodes,
      edges,
      stats: {
        total_nodes: nodes.length,
        total_relationships: edges.length,
        label_counts,
        relationship_type_counts,
      },
      namespaceCounts,
    };
    // The merge depends on the queries' `data` references — react-query
    // returns stable references for unchanged data, so this only rebuilds
    // when a new namespace lands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coords, ...queries.map((q) => q.data)]);

  const settled = queries.reduce(
    (n, q) => n + (q.isSuccess || q.isError ? 1 : 0),
    0,
  );
  const firstError = queries.find((q) => q.isError)?.error as Error | undefined;
  return {
    data,
    isLoading: coords.length > 0 && !data && settled < coords.length,
    isError: settled === coords.length && data === undefined && Boolean(firstError),
    error: firstError ?? null,
    progress: { total: coords.length, settled, hasAny: data !== undefined },
  };
}

/**
 * List every graph across every database — the source list for the unified
 * graph view. Resolves databases via the catalog schema endpoint, then fans
 * out `listGraphs` per database in parallel. A failure for one database is
 * skipped rather than failing the whole list.
 */
export function useAllDatabaseGraphs() {
  const { endpoint, token } = useSession();
  return useQuery<GraphCoord[]>({
    queryKey: ["graph", "list-all", endpoint],
    enabled: Boolean(endpoint),
    staleTime: 5 * 60_000,
    queryFn: async () => {
      const schema = await fetchSchema({ endpoint, token });
      const dbNames = schema.databases.map((d) => d.name);
      const lists = await Promise.all(
        dbNames.map((database) =>
          listGraphs({ endpoint, token, database })
            .then((gs: GraphRef[]) => gs.map((g) => ({ database, graph: g.name, kind: g.kind })))
            .catch(() => [] as GraphCoord[]),
        ),
      );
      return lists.flat();
    },
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
