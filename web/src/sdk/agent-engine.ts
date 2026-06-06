// Shim: backwards-compatible wrappers around @kyma-ai/client agent-engine functions.
// Old API: listEngines(a), putEngine(a, cfg), testEngine(a, cfg)

export type {
  EngineKind,
  EngineConfig,
  EngineSummary,
  EngineList,
  TestEngineResult,
} from "@kyma-ai/client";

import {
  listEngines as _listEngines,
  putEngine as _putEngine,
  testEngine as _testEngine,
  type EngineConfig,
} from "@kyma-ai/client";
import { transportFor } from "./compat";

type Args = { endpoint: string; token: string; database?: string };

export function listEngines(a: Args) {
  return _listEngines(transportFor(a));
}

export function putEngine(a: Args, cfg: EngineConfig) {
  return _putEngine(transportFor(a), cfg);
}

export function testEngine(a: Args, cfg: EngineConfig) {
  return _testEngine(transportFor(a), cfg);
}
