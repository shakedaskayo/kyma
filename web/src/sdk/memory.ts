// Shim: backwards-compatible wrappers around @kyma-ai/client memory functions.

export type {
  Row,
  PipelineRun,
  MemoryOverview,
  RetrievedMemory,
  LinkedResource,
  MemoryQueryResult,
  MemoryQueryRequest,
  MemorySettings,
  DreamingSettings,
} from "@kyma-ai/client";

import type { MemoryQueryRequest, MemorySettings } from "@kyma-ai/client";
import { sessionClient } from "./client";

type Args = { endpoint: string; token: string; database?: string };

export function fetchMemoryOverview(_a: Args) {
  return sessionClient().memory.fetchMemoryOverview();
}

export function queryMemory(_a: Args, req: MemoryQueryRequest) {
  return sessionClient().memory.queryMemory(req);
}

export function getMemorySettings(_a: Args) {
  return sessionClient().memory.getMemorySettings();
}

export function putMemorySettings(_a: Args, s: MemorySettings) {
  return sessionClient().memory.putMemorySettings(s);
}
