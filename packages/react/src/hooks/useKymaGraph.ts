/**
 * useKymaGraph — headless hook for the Kyma graph layer.
 *
 * Loads one or more (database, graph) pairs in parallel via React Query,
 * merges them into a single unified canvas and exposes imperative helpers
 * for expansion and search. Ported from web's useUnifiedGraph + useAllDatabaseGraphs.
 */

import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo, useRef, useState } from "react";
import type { GraphNode, GraphRelationship, GraphStats, SearchHits } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";

// ── Public types ──────────────────────────────────────────────────────────────

export interface UseKymaGraphArgs {
  /** Graphs to merge; omit to load all graphs of the default database. */
  graphs?: Array<{ database?: string; graph: string }>;
  /**
   * "all-databases": discover every (database, graph) across the deployment
   * via catalog schema → listGraphs fan-out. Equivalent to web's
   * useAllDatabaseGraphs(). When set, `graphs` is ignored.
   */
  discover?: "all-databases";
  realm?: string;
  limit?: number;
}

/** A (database, graph) coordinate — mirrors the web's GraphCoord shape. */
export interface GraphCoord {
  database: string;
  graph: string;
  kind?: string;
}

/** Composite namespace key: `${database}/${graph}`. */
export function graphKey(database: string, graph: string): string {
  return `${database}/${graph}`;
}

/** Per-namespace fetch progress — surfaces granular load state to the UI. */
export interface GraphProgress {
  total: number;
  settled: number;
  hasAny: boolean;
}

export interface UseKymaGraphResult {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  stats: GraphStats | null;
  /** Node count contributed by each namespace (db/graph composite key). */
  namespaceCounts: Record<string, number>;
  /** Resolved graph coordinates — used by GraphSidebar namespace picker. */
  coords: GraphCoord[];
  /** Granular fetch progress across all (database, graph) pairs. */
  progress: GraphProgress;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  /**
   * expandNode — fetches neighbors of a node by its composite id
   * (`namespace::nodeId` or plain node id if namespace is already stripped).
   */
  expandNode: (compositeId: string) => Promise<void>;
  searchNodes: (text: string) => Promise<SearchHits>;
  refetch: () => void;
}

// ── Implementation ────────────────────────────────────────────────────────────

/**
 * useGraphCoords — resolves (database, graph) coordinates from the three
 * discovery modes. Extracted so useGraphExport can share the same logic.
 *
 * Modes:
 * 1. `discover: "all-databases"` — catalog schema → listGraphs fan-out.
 * 2. `graphs` provided — use those coords directly (no network).
 * 3. Neither — list graphs from the default database only.
 */
export function useGraphCoords(
  args?: Pick<UseKymaGraphArgs, "graphs" | "discover">,
): {
  coords: GraphCoord[];
  isLoading: boolean;
  error: unknown;
} {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;
  const database = client.transport.database ?? "default";

  const discoverAll = args?.discover === "all-databases";
  const explicitGraphs = args?.graphs;
  const needsSimpleDiscovery = !explicitGraphs && !discoverAll;

  // ── Phase 1a: simple discovery (default database only) ───────────────────
  const simpleDiscoveryQuery = useQuery({
    queryKey: ["kyma", endpoint, database, "graph", "__list"],
    queryFn: async () => {
      const refs = await client.graph.listGraphs();
      return refs.map((r): GraphCoord => ({ database, graph: r.name, kind: r.kind }));
    },
    enabled: needsSimpleDiscovery,
    staleTime: 5 * 60_000,
  });

  // ── Phase 1b: all-databases discovery ────────────────────────────────────
  const allDbDiscoveryQuery = useQuery({
    queryKey: ["kyma", endpoint, "__all-databases", "graph", "__list"],
    queryFn: async () => {
      const schema = await client.catalog.fetchSchema();
      const dbNames = schema.databases.map((d: { name: string }) => d.name);
      const lists = await Promise.all(
        dbNames.map((db: string) =>
          client
            .withDatabase(db)
            .graph.listGraphs()
            .then((gs) => gs.map((g): GraphCoord => ({ database: db, graph: g.name, kind: g.kind })))
            .catch((): GraphCoord[] => []),
        ),
      );
      return lists.flat();
    },
    enabled: discoverAll,
    staleTime: 5 * 60_000,
  });

  // ── Resolved coordinates ──────────────────────────────────────────────────
  const coords: GraphCoord[] = useMemo(() => {
    if (discoverAll) return allDbDiscoveryQuery.data ?? [];
    if (explicitGraphs) {
      return explicitGraphs.map((g) => ({ database: g.database ?? database, graph: g.graph }));
    }
    return simpleDiscoveryQuery.data ?? [];
  }, [discoverAll, explicitGraphs, database, allDbDiscoveryQuery.data, simpleDiscoveryQuery.data]);

  const isLoading =
    (needsSimpleDiscovery && simpleDiscoveryQuery.isLoading) ||
    (discoverAll && allDbDiscoveryQuery.isLoading);

  const error =
    simpleDiscoveryQuery.isError
      ? simpleDiscoveryQuery.error
      : allDbDiscoveryQuery.isError
        ? allDbDiscoveryQuery.error
        : null;

  return { coords, isLoading, error };
}

/**
 * The hook operates in three discovery modes:
 * 1. `discover: "all-databases"` — catalog schema → listGraphs fan-out across all DBs.
 * 2. `graphs` provided — use those coords directly.
 * 3. Neither — list graphs from the default database only.
 */
export function useKymaGraph(args?: UseKymaGraphArgs): UseKymaGraphResult {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;
  const database = client.transport.database ?? "default";
  const realm = args?.realm;
  const limit = args?.limit ?? 800;

  // ── Discovery via useGraphCoords ──────────────────────────────────────────
  const {
    coords: resolvedCoords,
    isLoading: discoveryLoading,
    error: discoveryError,
  } = useGraphCoords(args);

  const needsSimpleDiscovery = !args?.graphs && args?.discover !== "all-databases";
  const discoverAll = args?.discover === "all-databases";

  // ── Phase 2: load each graph in parallel ──────────────────────────────────
  const queries = useQueries({
    queries: resolvedCoords.map(({ database: db, graph }) => ({
      queryKey: ["kyma", endpoint, db, "graph", graph, realm ?? null, limit] as const,
      queryFn: () => client.withDatabase(db).graph.getOverview({ graph, realm, limit }),
      staleTime: 30_000,
      enabled: resolvedCoords.length > 0,
    })),
  });

  // ── Merge ─────────────────────────────────────────────────────────────────
  const [expanded, setExpanded] = useState<{
    nodes: GraphNode[];
    edges: GraphRelationship[];
  }>({ nodes: [], edges: [] });

  const expandedCompositeIds = useRef(new Set<string>());

  const merged = useMemo(() => {
    let touched = false;
    const nodes: GraphNode[] = [];
    const edges: GraphRelationship[] = [];
    const seenNodes = new Set<string>();
    const seenEdges = new Set<string>();
    const label_counts: Record<string, number> = {};
    const relationship_type_counts: Record<string, number> = {};
    const namespaceCounts: Record<string, number> = {};

    resolvedCoords.forEach((coord, i) => {
      const payload = queries[i]?.data;
      if (!payload) return;
      touched = true;
      const ns = graphKey(coord.database, coord.graph);
      for (const n of payload.nodes) {
        const k = `${ns}::${n.id}`;
        if (seenNodes.has(k)) continue;
        seenNodes.add(k);
        nodes.push({ ...n, namespace: ns, database: coord.database });
        namespaceCounts[ns] = (namespaceCounts[ns] ?? 0) + 1;
        const label = n.labels[0] ?? "Node";
        label_counts[label] = (label_counts[label] ?? 0) + 1;
      }
      for (const e of payload.edges) {
        const k = `${ns}::${e.id}`;
        if (seenEdges.has(k)) continue;
        seenEdges.add(k);
        edges.push({ ...e, namespace: ns, database: coord.database });
        relationship_type_counts[e.relationship_type] =
          (relationship_type_counts[e.relationship_type] ?? 0) + 1;
      }
    });

    // Append expanded nodes/edges (deduplicated by composite key).
    for (const n of expanded.nodes) {
      const k = `${n.namespace ?? ""}::${n.id}`;
      if (!seenNodes.has(k)) {
        seenNodes.add(k);
        nodes.push(n);
        const label = n.labels[0] ?? "Node";
        label_counts[label] = (label_counts[label] ?? 0) + 1;
        if (n.namespace) namespaceCounts[n.namespace] = (namespaceCounts[n.namespace] ?? 0) + 1;
      }
    }
    for (const e of expanded.edges) {
      const k = `${e.namespace ?? ""}::${e.id}`;
      if (!seenEdges.has(k)) {
        seenEdges.add(k);
        edges.push(e);
        relationship_type_counts[e.relationship_type] =
          (relationship_type_counts[e.relationship_type] ?? 0) + 1;
      }
    }

    if (!touched && expanded.nodes.length === 0) return null;

    return {
      nodes,
      edges,
      stats: {
        total_nodes: nodes.length,
        total_relationships: edges.length,
        label_counts,
        relationship_type_counts,
      } as GraphStats,
      namespaceCounts,
    };
    // Stable collapse of per-query freshness — see web's useUnifiedGraph for rationale.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedCoords, queries.map((q) => q.dataUpdatedAt ?? 0).join(","), expanded]);

  // ── Loading / error state ─────────────────────────────────────────────────
  const settled = queries.reduce((n, q) => n + (q.isSuccess || q.isError ? 1 : 0), 0);

  const firstQueryError = queries.find((q) => q.isError)?.error ?? null;
  const firstError = firstQueryError ?? discoveryError;

  const isLoading =
    discoveryLoading ||
    (resolvedCoords.length > 0 && !merged && settled < resolvedCoords.length);

  const isError =
    !isLoading && merged === null && (Boolean(firstError) || (settled === resolvedCoords.length && resolvedCoords.length > 0));

  // ── Imperative helpers ────────────────────────────────────────────────────
  const queryClient = useQueryClient();

  /**
   * expandNode — accepts a composite id (`namespace::nodeId`) as used in the
   * graph canvas, strips the namespace prefix, and expands neighbours.
   * The compositeId format is `${database}/${graph}::${nodeId}`.
   */
  const expandNode = useCallback(
    async (compositeId: string) => {
      if (expandedCompositeIds.current.has(compositeId)) return;
      expandedCompositeIds.current.add(compositeId);

      // Parse composite id — look for a matching node in the merged set first.
      const mergedNode = merged?.nodes.find(
        (n) => `${n.namespace ?? ""}::${n.id}` === compositeId,
      );
      const nodeDb = mergedNode?.database ?? resolvedCoords[0]?.database ?? database;
      const nodeNs = mergedNode?.namespace ?? (resolvedCoords[0] ? graphKey(nodeDb, resolvedCoords[0].graph) : "");
      // Bare node id (strip namespace prefix if present).
      const nodeId = mergedNode?.id ?? compositeId.split("::").pop() ?? compositeId;

      const graphName =
        nodeNs
          ? nodeNs.slice(nodeDb.length + 1) // strip "${db}/" prefix from namespace
          : (resolvedCoords.find((c) => c.database === nodeDb)?.graph ?? resolvedCoords[0]?.graph ?? "");

      try {
        const expansion = await client
          .withDatabase(nodeDb)
          .graph.expandNeighbors({ graph: graphName, nodeIds: [nodeId] });

        const newNodes = await Promise.all(
          expansion.new_node_ids.map((id) =>
            client
              .withDatabase(nodeDb)
              .graph.getNode({ graph: graphName, id })
              .catch(() => null),
          ),
        );

        const ns = nodeNs || graphKey(nodeDb, graphName);
        setExpanded((prev) => ({
          nodes: [
            ...prev.nodes,
            ...newNodes
              .filter((n): n is GraphNode => n !== null)
              .map((n) => ({ ...n, namespace: ns, database: nodeDb })),
          ],
          edges: [
            ...prev.edges,
            ...expansion.edges.map((e) => ({ ...e, namespace: ns, database: nodeDb })),
          ],
        }));
      } catch (err) {
        expandedCompositeIds.current.delete(compositeId);
        throw err;
      }
    },
    [client, database, resolvedCoords, merged],
  );

  /**
   * searchNodes — searches the first loaded graph (most common case).
   */
  const searchNodes = useCallback(
    async (text: string): Promise<SearchHits> => {
      const coord = resolvedCoords[0];
      if (!coord) return { hits: [], total: 0, limit: 20, offset: 0 };
      return client
        .withDatabase(coord.database)
        .graph.searchNodes({ graph: coord.graph, text });
    },
    [client, resolvedCoords],
  );

  const refetch = useCallback(() => {
    for (const coord of resolvedCoords) {
      void queryClient.invalidateQueries({
        queryKey: ["kyma", endpoint, coord.database, "graph", coord.graph],
      });
    }
    if (needsSimpleDiscovery) {
      void queryClient.invalidateQueries({
        queryKey: ["kyma", endpoint, database, "graph", "__list"],
      });
    }
    if (discoverAll) {
      void queryClient.invalidateQueries({
        queryKey: ["kyma", endpoint, "__all-databases", "graph", "__list"],
      });
    }
  }, [queryClient, endpoint, database, resolvedCoords, needsSimpleDiscovery, discoverAll]);

  return {
    nodes: merged?.nodes ?? [],
    edges: merged?.edges ?? [],
    stats: merged?.stats ?? null,
    namespaceCounts: merged?.namespaceCounts ?? {},
    coords: resolvedCoords,
    progress: {
      total: resolvedCoords.length,
      settled,
      hasAny: merged !== null,
    },
    isLoading,
    isError,
    error: firstError ?? null,
    expandNode,
    searchNodes,
    refetch,
  };
}
