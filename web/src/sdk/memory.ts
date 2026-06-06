// Shim: backwards-compatible wrappers around @kyma-ai/client memory functions.
// Old API: fetchMemoryOverview(a), queryMemory(a, req), getMemorySettings(a),
//          putMemorySettings(a, s)

export type {
  Row,
  PipelineRun,
  MemoryOverview,
  RetrievedMemory,
  LinkedResource,
  MemoryQueryResult,
  MemoryQueryRequest,
  MemorySettings,
} from "@kyma-ai/client";

import {
  fetchMemoryOverview as _fetchMemoryOverview,
  queryMemory as _queryMemory,
  getMemorySettings as _getMemorySettings,
  putMemorySettings as _putMemorySettings,
  type MemoryQueryRequest,
  type MemorySettings,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Args = { endpoint: string; token: string; database?: string };

export function fetchMemoryOverview(a: Args) {
  return _fetchMemoryOverview(transportFor(a));
}

export function queryMemory(a: Args, req: MemoryQueryRequest) {
  return _queryMemory(transportFor(a), req);
}

export function getMemorySettings(a: Args) {
  return _getMemorySettings(transportFor(a));
}

export function putMemorySettings(a: Args, s: MemorySettings) {
  return _putMemorySettings(transportFor(a), s);
}
