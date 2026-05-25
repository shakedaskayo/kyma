// Typed client for the kyma graph layer (`/v1/graph/*`). JSON in/out.
// Mirrors the per-module fetch-helper convention used by `sdk/dashboards.ts`.

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
}
export interface GraphRelationship {
  id: string;
  source_id: string;
  target_id: string;
  relationship_type: string;
  properties: Props;
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

type BaseArgs = { endpoint: string; token: string; database?: string };
type GraphArgs = BaseArgs & { graph: string };

function headers(token: string, database?: string): Record<string, string> {
  const h: Record<string, string> = { authorization: `Bearer ${token}`, "content-type": "application/json" };
  if (database) h["x-database"] = database;
  return h;
}
function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}
async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

export async function listGraphs(args: BaseArgs): Promise<GraphRef[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph`, { headers: headers(args.token, args.database) });
  return handleResponse<GraphRef[]>(res);
}

export async function getOverview(
  args: GraphArgs & { realm?: string; limit?: number },
): Promise<GraphPayload> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  if (args.limit != null) q.set("limit", String(args.limit));
  const qs = q.toString();
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/overview${qs ? `?${qs}` : ""}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<GraphPayload>(res);
}

export async function getStats(args: GraphArgs & { realm?: string }): Promise<GraphStats> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  const qs = q.toString();
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/stats${qs ? `?${qs}` : ""}`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<GraphStats>(res);
}

export async function getGraphSchema(args: GraphArgs): Promise<GraphSchema> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/schema`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<GraphSchema>(res);
}

export async function getNode(args: GraphArgs & { id: string }): Promise<GraphNode> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<GraphNode>(res);
}

export async function getSubgraph(
  args: GraphArgs & { id: string; depth?: number },
): Promise<GraphPayload> {
  const qs = args.depth != null ? `?depth=${args.depth}` : "";
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}/subgraph${qs}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<GraphPayload>(res);
}

export async function searchNodes(
  args: GraphArgs & { text: string; labels?: string[]; realm?: string; limit?: number; offset?: number },
): Promise<SearchHits> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/search`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify({
      text: args.text,
      labels: args.labels ?? [],
      realm: args.realm,
      limit: args.limit ?? 20,
      offset: args.offset ?? 0,
    }),
  });
  return handleResponse<SearchHits>(res);
}

export async function expandNeighbors(
  args: GraphArgs & { nodeIds: string[]; direction?: Direction; onlyInternal?: boolean; limit?: number },
): Promise<EdgeExpansion> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/neighbors`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify({
      node_ids: args.nodeIds,
      direction: args.direction ?? "both",
      only_internal: args.onlyInternal ?? false,
      limit: args.limit ?? 200,
    }),
  });
  return handleResponse<EdgeExpansion>(res);
}
