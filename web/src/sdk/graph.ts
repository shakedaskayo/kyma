// Shim: backwards-compatible wrappers around @kyma-ai/client graph functions.
// All session calls delegate to sessionClient() — no one-shot transports.

export type {
  Props,
  NodeMetadata,
  GraphNode,
  GraphRelationship,
  GraphStats,
  GraphPayload,
  EdgeExpansion,
  SearchHits,
  GraphSchema,
  GraphRef,
  Direction,
} from "@kyma-ai/client";

import type { Direction } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Base = { endpoint: string; token: string; database?: string };

export function listGraphs(_args: Base) {
  return sessionClient().graph.listGraphs();
}

export function getOverview(args: Base & { graph: string; realm?: string; limit?: number }) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.getOverview(rest);
}

export function getStats(args: Base & { graph: string; realm?: string }) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.getStats(rest);
}

export function getGraphSchema(args: Base & { graph: string }) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.getGraphSchema(rest);
}

export function getNode(args: Base & { graph: string; id: string }) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.getNode(rest);
}

export function getSubgraph(args: Base & { graph: string; id: string; depth?: number }) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.getSubgraph(rest);
}

export function searchNodes(
  args: Base & { graph: string; text: string; labels?: string[]; realm?: string; limit?: number; offset?: number },
) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.searchNodes(rest);
}

export function expandNeighbors(
  args: Base & { graph: string; nodeIds: string[]; direction?: Direction; onlyInternal?: boolean; limit?: number },
) {
  const { endpoint: _ep, token: _tok, database: _db, ...rest } = args;
  return sessionClient().graph.expandNeighbors(rest);
}
