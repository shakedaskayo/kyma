//! Typed client for `/v1/agent/engines` + `/v1/agent/engine`.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

export type EngineKind = "anthropic" | "openai" | "ollama" | "claude_cli";

export interface EngineConfig {
  kind: EngineKind;
  model: string;
  credential_id?: string | null;
  host?: string | null;
  extras?: Record<string, unknown>;
}

export interface EngineSummary {
  kind: EngineKind;
  label: string;
  models: string[];
  needs_key: boolean;
}

export interface EngineList {
  available: EngineSummary[];
  active: EngineConfig;
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function listEngines(t: KymaTransport): Promise<EngineList> {
  return handleResponse<EngineList>(await t.request("/v1/agent/engines"));
}

export async function putEngine(t: KymaTransport, cfg: EngineConfig): Promise<EngineConfig> {
  return handleResponse<EngineConfig>(
    await t.request("/v1/agent/engine", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(cfg),
    }),
  );
}

export interface TestEngineResult {
  ok: boolean;
  kind: EngineKind;
  model: string;
}

export async function testEngine(t: KymaTransport, cfg: EngineConfig): Promise<TestEngineResult> {
  return handleResponse<TestEngineResult>(
    await t.request("/v1/agent/engine/test", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(cfg),
    }),
  );
}
