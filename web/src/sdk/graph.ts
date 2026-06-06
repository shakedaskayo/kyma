// Shim: backwards-compatible wrappers around @kyma-ai/client graph functions.
// The old web/src/sdk/graph functions accepted { endpoint, token, database?, ...args }.
// The new client functions accept (transport, args?).

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

import {
  listGraphs as _listGraphs,
  getOverview as _getOverview,
  getStats as _getStats,
  getGraphSchema as _getGraphSchema,
  getNode as _getNode,
  getSubgraph as _getSubgraph,
  searchNodes as _searchNodes,
  expandNeighbors as _expandNeighbors,
  type Direction,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Base = { endpoint: string; token: string; database?: string };

export function listGraphs(args: Base) {
  return _listGraphs(transportFor(args));
}

export function getOverview(args: Base & { graph: string; realm?: string; limit?: number }) {
  const { endpoint, token, database, ...rest } = args;
  return _getOverview(transportFor({ endpoint, token, database }), rest);
}

export function getStats(args: Base & { graph: string; realm?: string }) {
  const { endpoint, token, database, ...rest } = args;
  return _getStats(transportFor({ endpoint, token, database }), rest);
}

export function getGraphSchema(args: Base & { graph: string }) {
  const { endpoint, token, database, ...rest } = args;
  return _getGraphSchema(transportFor({ endpoint, token, database }), rest);
}

export function getNode(args: Base & { graph: string; id: string }) {
  const { endpoint, token, database, ...rest } = args;
  return _getNode(transportFor({ endpoint, token, database }), rest);
}

export function getSubgraph(args: Base & { graph: string; id: string; depth?: number }) {
  const { endpoint, token, database, ...rest } = args;
  return _getSubgraph(transportFor({ endpoint, token, database }), rest);
}

export function searchNodes(
  args: Base & { graph: string; text: string; labels?: string[]; realm?: string; limit?: number; offset?: number },
) {
  const { endpoint, token, database, ...rest } = args;
  return _searchNodes(transportFor({ endpoint, token, database }), rest);
}

export function expandNeighbors(
  args: Base & { graph: string; nodeIds: string[]; direction?: Direction; onlyInternal?: boolean; limit?: number },
) {
  const { endpoint, token, database, ...rest } = args;
  return _expandNeighbors(transportFor({ endpoint, token, database }), rest);
}
