// Shim: backwards-compatible wrappers around @kyma-ai/client agent-engine functions.

export type {
  EngineKind,
  EngineConfig,
  EngineSummary,
  EngineList,
  TestEngineResult,
} from "@kyma-ai/client";

import type { EngineConfig } from "@kyma-ai/client";
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
