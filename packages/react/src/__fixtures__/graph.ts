import type { GraphPayload } from "@kyma-ai/client";

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
