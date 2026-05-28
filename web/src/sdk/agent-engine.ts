//! Typed client for `/v1/agent/engines` + `/v1/agent/engine`.

export type EngineKind = "anthropic" | "openai" | "ollama";

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

type Args = { endpoint: string; token: string };

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}
function headers(token: string): Record<string, string> {
  return {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
}

export async function listEngines(a: Args): Promise<EngineList> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engines`, {
    headers: headers(a.token),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`engines: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

export async function putEngine(a: Args, cfg: EngineConfig): Promise<EngineConfig> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engine`, {
    method: "PUT",
    headers: headers(a.token),
    body: JSON.stringify(cfg),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`save engine: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

export interface TestEngineResult {
  ok: boolean;
  kind: EngineKind;
  model: string;
}

export async function testEngine(a: Args, cfg: EngineConfig): Promise<TestEngineResult> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/engine/test`, {
    method: "POST",
    headers: headers(a.token),
    body: JSON.stringify(cfg),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`test engine: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}
