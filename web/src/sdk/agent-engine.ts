// Shim: backwards-compatible wrappers around @pensieve-ai/client agent-engine functions.

export type {
  EngineKind,
  EngineConfig,
  EngineSummary,
  EngineList,
  TestEngineResult,
} from "@pensieve-ai/client";

import type { EngineConfig } from "@pensieve-ai/client";
import { sessionClient } from "./client";

type Args = { endpoint: string; token: string; database?: string };

export function listEngines(_a: Args) {
  return sessionClient().agent.listEngines();
}

export function putEngine(_a: Args, cfg: EngineConfig) {
  return sessionClient().agent.putEngine(cfg);
}

export function testEngine(_a: Args, cfg: EngineConfig) {
  return sessionClient().agent.testEngine(cfg);
}
