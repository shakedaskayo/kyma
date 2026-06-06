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
  realm?: string;
  limit?: number;
}

export interface UseKymaGraphResult {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  stats: GraphStats | null;
  isLoading: boolean;
  error: unknown;
  expandNode: (nodeId: string) => Promise<void>;
  searchNodes: (text: string) => Promise<SearchHits>;
  refetch: () => void;
}

// ── Internal ──────────────────────────────────────────────────────────────────

/** Composite namespace key — same as web's graphKey(). */
function graphKey(database: string, graph: string): string {
  return `${database}/${graph}`;
}

/**
 * The hook has two phases:
 * 1. If `graphs` is provided: skip the listing step and go straight to loading.
 * 2. If `graphs` is omitted: first call listGraphs() to discover all graphs for
 *    the client's default database, then load each one.
 */
export function useKymaGraph(args?: UseKymaGraphArgs): UseKymaGraphResult {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;
  const database = client.transport.database ?? "default";
  const realm = args?.realm;
  const limit = args?.limit ?? 800;

  // ── Phase 1: discover graphs when args.graphs is omitted ──────────────────
  const needsDiscovery = !args?.graphs;

  const discoveryQuery = useQuery({
    queryKey: ["kyma", endpoint, database, "graph", "__list"],
    queryFn: async () => {
      const refs = await client.graph.listGraphs();
      return refs.map((r) => ({ database, graph: r.name }));
    },
    enabled: needsDiscovery,
    staleTime: 5 * 60_000,
  });

  // Resolved coords: either the explicitly provided graphs, or discovered ones.
  const resolvedCoords: Array<{ database: string; graph: string }> = useMemo(() => {
    if (args?.graphs) {
      return args.graphs.map((g) => ({
        database: g.database ?? database,
        graph: g.graph,
      }));
    }
    return discoveryQuery.data ?? [];
  }, [args?.graphs, database, discoveryQuery.data]);

  // ── Phase 2: load each graph in parallel ──────────────────────────────────
  const queries = useQueries({
    queries: resolvedCoords.map(({ database: db, graph }) => ({
      queryKey: ["kyma", endpoint, db, "graph", graph, realm ?? null, limit] as const,
      queryFn: () =>
        client.withDatabase(db).graph.getOverview({ graph, realm, limit }),
      staleTime: 30_000,
      enabled: resolvedCoords.length > 0,
    })),
  });

  // ── Merge ─────────────────────────────────────────────────────────────────
  // Extra local state: nodes/edges appended via expandNode. We keep these
  // separate from the query cache so they survive re-renders without
  // triggering a refetch.
  const [expanded, setExpanded] = useState<{
    nodes: GraphNode[];
    edges: GraphRelationship[];
  }>({ nodes: [], edges: [] });

  // Track which nodes have already been expanded (avoid duplicates).
  const expandedNodeIds = useRef(new Set<string>());

  const merged = useMemo(() => {
    let touched = false;
    const nodes: GraphNode[] = [];
    const edges: GraphRelationship[] = [];
    const seenNodes = new Set<string>();
    const seenEdges = new Set<string>();
    const label_counts: Record<string, number> = {};
    const relationship_type_counts: Record<string, number> = {};

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

    // Append expanded nodes/edges (deduplicated by id).
    for (const n of expanded.nodes) {
      if (!seenNodes.has(n.id)) {
        seenNodes.add(n.id);
        nodes.push(n);
        const label = n.labels[0] ?? "Node";
        label_counts[label] = (label_counts[label] ?? 0) + 1;
      }
    }
    for (const e of expanded.edges) {
      if (!seenEdges.has(e.id)) {
        seenEdges.add(e.id);
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
    };
    // Stable collapse of per-query freshness — see web's useUnifiedGraph for rationale.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolvedCoords, queries.map((q) => q.dataUpdatedAt ?? 0).join(","), expanded]);

  // ── Loading / error state ─────────────────────────────────────────────────
  const settled = queries.reduce((n, q) => n + (q.isSuccess || q.isError ? 1 : 0), 0);
  const firstError =
    queries.find((q) => q.isError)?.error ??
    (discoveryQuery.isError ? discoveryQuery.error : null);

  const isLoading =
    (needsDiscovery && discoveryQuery.isLoading) ||
    (resolvedCoords.length > 0 && !merged && settled < resolvedCoords.length);

  // ── Imperative helpers ────────────────────────────────────────────────────
  const queryClient = useQueryClient();

  /**
   * expandNode — fetches neighbors of a node, then fetches any new node objects
   * returned in `new_node_ids`, and appends both to the local expanded set.
   *
   * Determines the (database, graph) by looking at the merged node's database
   * tag and finding the first matching coord for that database. Falls back to
   * the first coord if none match.
   */
  const expandNode = useCallback(
    async (nodeId: string) => {
      if (expandedNodeIds.current.has(nodeId)) return;
      expandedNodeIds.current.add(nodeId);

      // Determine which (database, graph) this node lives in.
      const mergedNode = merged?.nodes.find((n) => n.id === nodeId);
      const nodeDb = mergedNode?.database ?? resolvedCoords[0]?.database ?? database;
      const nodeNs = mergedNode?.namespace ?? resolvedCoords[0] ? graphKey(nodeDb, resolvedCoords[0]?.graph ?? "") : "";
      const graphName =
        resolvedCoords.find((c) => c.database === nodeDb)?.graph ??
        resolvedCoords[0]?.graph ?? "";

      try {
        const expansion = await client
          .withDatabase(nodeDb)
          .graph.expandNeighbors({ graph: graphName, nodeIds: [nodeId] });

        // Fetch node objects for any new node ids returned by the expansion.
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
        // Remove from expanded set on failure so it can be retried.
        expandedNodeIds.current.delete(nodeId);
        throw err;
      }
    },
    [client, database, resolvedCoords, merged],
  );

  /**
   * searchNodes — calls the graph search endpoint across all loaded graphs.
   * When multiple graphs are loaded, searches the first graph only (most
   * common single-graph case); callers that need multi-graph search should
   * call client.graph.searchNodes() directly with explicit graph names.
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
    if (needsDiscovery) {
      void queryClient.invalidateQueries({
        queryKey: ["kyma", endpoint, database, "graph", "__list"],
      });
    }
  }, [queryClient, endpoint, database, resolvedCoords, needsDiscovery]);

  return {
    nodes: merged?.nodes ?? [],
    edges: merged?.edges ?? [],
    stats: merged?.stats ?? null,
    isLoading,
    error: firstError ?? null,
    expandNode,
    searchNodes,
    refetch,
  };
}
