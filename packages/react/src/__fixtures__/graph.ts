import type { GraphExportPage, GraphPayload } from "@kyma-ai/client";

const META = { created_at: "2025-01-01T00:00:00Z", updated_at: "2025-06-01T00:00:00Z", realm: "default" };

export const GRAPH_FIXTURE: GraphPayload = {
  stats: {
    total_nodes: 12,
    total_relationships: 14,
    label_counts: { Service: 4, User: 3, Database: 2, Queue: 2, Gateway: 1 },
    relationship_type_counts: { CALLS: 6, READS: 4, WRITES: 2, ROUTES_TO: 2 },
  },
  nodes: [
    { id: "gw-1",  labels: ["Gateway"],  properties: { name: "api-gateway",      version: "2.1.0" }, metadata: META },
    { id: "svc-1", labels: ["Service"],  properties: { name: "auth-service",     version: "1.4.2", status: "healthy" }, metadata: META },
    { id: "svc-2", labels: ["Service"],  properties: { name: "user-service",     version: "3.0.1", status: "healthy" }, metadata: META },
    { id: "svc-3", labels: ["Service"],  properties: { name: "billing-service",  version: "1.1.0", status: "degraded" }, metadata: META },
    { id: "svc-4", labels: ["Service"],  properties: { name: "notify-service",   version: "2.0.0", status: "healthy" }, metadata: META },
    { id: "db-1",  labels: ["Database"], properties: { name: "users-db",         engine: "postgres", version: "15" }, metadata: META },
    { id: "db-2",  labels: ["Database"], properties: { name: "billing-db",       engine: "postgres", version: "14" }, metadata: META },
    { id: "q-1",   labels: ["Queue"],    properties: { name: "email-queue",      type: "rabbitmq" }, metadata: META },
    { id: "q-2",   labels: ["Queue"],    properties: { name: "billing-events",   type: "kafka" }, metadata: META },
    { id: "u-1",   labels: ["User"],     properties: { name: "alice@acme.com",   role: "admin" }, metadata: META },
    { id: "u-2",   labels: ["User"],     properties: { name: "bob@acme.com",     role: "dev" }, metadata: META },
    { id: "u-3",   labels: ["User"],     properties: { name: "carol@acme.com",   role: "dev" }, metadata: META },
  ],
  edges: [
    { id: "e1",  source_id: "gw-1",  target_id: "svc-1", relationship_type: "ROUTES_TO", properties: {} },
    { id: "e2",  source_id: "gw-1",  target_id: "svc-2", relationship_type: "ROUTES_TO", properties: {} },
    { id: "e3",  source_id: "svc-1", target_id: "db-1",  relationship_type: "READS",     properties: {} },
    { id: "e4",  source_id: "svc-1", target_id: "db-1",  relationship_type: "WRITES",    properties: {} },
    { id: "e5",  source_id: "svc-2", target_id: "db-1",  relationship_type: "READS",     properties: {} },
    { id: "e6",  source_id: "svc-2", target_id: "svc-3", relationship_type: "CALLS",     properties: {} },
    { id: "e7",  source_id: "svc-3", target_id: "db-2",  relationship_type: "READS",     properties: {} },
    { id: "e8",  source_id: "svc-3", target_id: "db-2",  relationship_type: "WRITES",    properties: {} },
    { id: "e9",  source_id: "svc-3", target_id: "q-2",   relationship_type: "CALLS",     properties: {} },
    { id: "e10", source_id: "svc-4", target_id: "q-1",   relationship_type: "CALLS",     properties: {} },
    { id: "e11", source_id: "svc-2", target_id: "svc-4", relationship_type: "CALLS",     properties: {} },
    { id: "e12", source_id: "u-1",   target_id: "svc-1", relationship_type: "CALLS",     properties: {} },
    { id: "e13", source_id: "u-2",   target_id: "svc-2", relationship_type: "CALLS",     properties: {} },
    { id: "e14", source_id: "u-3",   target_id: "svc-2", relationship_type: "CALLS",     properties: {} },
  ],
};

// Server-side layout positions for the export endpoint — a hand-placed spread
// that reads well at the default zoom (gateway left, services center, stores
// and queues right, users below).
const EXPORT_POSITIONS: Record<string, { x: number; y: number }> = {
  "gw-1":  { x: -420, y: 0 },
  "svc-1": { x: -140, y: -160 },
  "svc-2": { x: -100, y: 80 },
  "svc-3": { x: 180, y: 200 },
  "svc-4": { x: 160, y: -60 },
  "db-1":  { x: 180, y: -240 },
  "db-2":  { x: 460, y: 120 },
  "q-1":   { x: 430, y: -140 },
  "q-2":   { x: 460, y: 320 },
  "u-1":   { x: -380, y: -320 },
  "u-2":   { x: -420, y: 220 },
  "u-3":   { x: -300, y: 340 },
};

export const GRAPH_EXPORT_FIXTURE: GraphExportPage = {
  layout_status: "ready",
  layout_id: "fixture-layout-1",
  total_nodes: GRAPH_FIXTURE.nodes.length,
  total_edges: GRAPH_FIXTURE.edges.length,
  nodes: GRAPH_FIXTURE.nodes.map((n) => ({ ...n, ...(EXPORT_POSITIONS[n.id] ?? { x: 0, y: 0 }) })),
  edges: GRAPH_FIXTURE.edges,
};

export const GRAPH_LIST_FIXTURE = [
  { name: "services",  kind: "property", description: "Service mesh topology" },
  { name: "users",     kind: "property", description: "User graph" },
];

export const NEIGHBORS_FIXTURE = {
  edges: [] as GraphPayload["edges"],
  new_node_ids: [] as string[],
};

export const SEARCH_FIXTURE = {
  hits: [GRAPH_FIXTURE.nodes[0], GRAPH_FIXTURE.nodes[1]],
  total: 2,
  limit: 20,
  offset: 0,
};
