// Typed client for the kyma graph layer (`/v1/graph/*`). JSON in/out.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

export type Props = Record<string, unknown>;

export interface NodeMetadata {
  created_at: string;
  updated_at: string;
  source_type?: string | null;
  source_id?: string | null;
  realm: string;
}
export interface GraphNode {
  id: string;
  labels: string[];
  properties: Props;
  metadata: NodeMetadata;
  /** Which graph ("namespace") this node was merged from in the unified view. */
  namespace?: string;
  /** Database the node lives in — needed for cross-DB unified view (expand, deep-link). */
  database?: string;
}
export interface GraphRelationship {
  id: string;
  source_id: string;
  target_id: string;
  relationship_type: string;
  properties: Props;
  /** Which graph ("namespace") this edge was merged from in the unified view. */
  namespace?: string;
  /** Database the edge lives in — needed for cross-DB unified view. */
  database?: string;
}
export interface GraphStats {
  total_nodes: number;
  total_relationships: number;
  label_counts: Record<string, number>;
  relationship_type_counts: Record<string, number>;
}
export interface GraphPayload {
  stats: GraphStats;
  nodes: GraphNode[];
  edges: GraphRelationship[];
}
export interface EdgeExpansion {
  edges: GraphRelationship[];
  new_node_ids: string[];
}
export interface SearchHits {
  hits: GraphNode[];
  total: number;
  limit: number;
  offset: number;
}
export interface GraphSchema {
  node_kinds: string[];
  edge_types: string[];
  property_keys: Record<string, string[]>;
}
export interface GraphRef {
  name: string;
  kind: string;
  description: string;
}
export type Direction = "forward" | "backward" | "both";

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function listGraphs(t: KymaTransport): Promise<GraphRef[]> {
  return handleResponse<GraphRef[]>(await t.request("/v1/graph"));
}

export async function getOverview(
  t: KymaTransport,
  args: { graph: string; realm?: string; limit?: number },
): Promise<GraphPayload> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  if (args.limit != null) q.set("limit", String(args.limit));
  const qs = q.toString();
  return handleResponse<GraphPayload>(
    await t.request(`/v1/graph/${args.graph}/overview${qs ? `?${qs}` : ""}`),
  );
}

export async function getStats(
  t: KymaTransport,
  args: { graph: string; realm?: string },
): Promise<GraphStats> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  const qs = q.toString();
  return handleResponse<GraphStats>(
    await t.request(`/v1/graph/${args.graph}/stats${qs ? `?${qs}` : ""}`),
  );
}

export async function getGraphSchema(
  t: KymaTransport,
  args: { graph: string },
): Promise<GraphSchema> {
  return handleResponse<GraphSchema>(await t.request(`/v1/graph/${args.graph}/schema`));
}

export async function getNode(
  t: KymaTransport,
  args: { graph: string; id: string },
): Promise<GraphNode> {
  return handleResponse<GraphNode>(
    await t.request(`/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}`),
  );
}

export async function getSubgraph(
  t: KymaTransport,
  args: { graph: string; id: string; depth?: number },
): Promise<GraphPayload> {
  const qs = args.depth != null ? `?depth=${args.depth}` : "";
  return handleResponse<GraphPayload>(
    await t.request(
      `/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}/subgraph${qs}`,
    ),
  );
}

export async function searchNodes(
  t: KymaTransport,
  args: { graph: string; text: string; labels?: string[]; realm?: string; limit?: number; offset?: number },
): Promise<SearchHits> {
  return handleResponse<SearchHits>(
    await t.request(`/v1/graph/${args.graph}/search`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        text: args.text,
        labels: args.labels ?? [],
        realm: args.realm,
        limit: args.limit ?? 20,
        offset: args.offset ?? 0,
      }),
    }),
  );
}

export async function expandNeighbors(
  t: KymaTransport,
  args: { graph: string; nodeIds: string[]; direction?: Direction; onlyInternal?: boolean; limit?: number },
): Promise<EdgeExpansion> {
  return handleResponse<EdgeExpansion>(
    await t.request(`/v1/graph/${args.graph}/neighbors`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        node_ids: args.nodeIds,
        direction: args.direction ?? "both",
        only_internal: args.onlyInternal ?? false,
        limit: args.limit ?? 200,
      }),
    }),
  );
}

