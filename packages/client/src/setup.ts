// Typed client for the first-run setup surface (`/v1/auth/status`,
// `/v1/auth/signup`, `/v1/setup/probe`). The first three functions are
// unauthenticated — they power the onboarding wizard before any user/token
// exists, so they take `endpoint: string` and use plain fetch.
// `ingestSample` requires a Bearer token and uses a KymaTransport.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";
import type { TokenPair } from "./auth";

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

export interface SetupStatus {
  users_exist: boolean;
  setup_required: boolean;
}

export interface EngineRecommendation {
  kind: string;
  model: string;
  needs_pull?: boolean;
}

export interface SetupProbe {
  ollama_reachable: boolean;
  ollama_models: string[];
  gemma4_present: boolean;
  anthropic_key_present: boolean;
  openai_key_present: boolean;
  claude_binary_found: boolean;
  recommend: EngineRecommendation;
}

/** GET /v1/auth/status — does this instance still need first-run setup? Unauthenticated. */
export async function getSetupStatus(endpoint: string): Promise<SetupStatus> {
  const res = await globalThis.fetch(`${base(endpoint)}/v1/auth/status`);
  if (!res.ok) throw new Error(`setup status: ${res.status}`);
  return res.json();
}

/** GET /v1/setup/probe — host capabilities for the AI-engine step. Unauthenticated. */
export async function getSetupProbe(endpoint: string): Promise<SetupProbe> {
  const res = await globalThis.fetch(`${base(endpoint)}/v1/setup/probe`);
  if (!res.ok) throw new Error(`setup probe: ${res.status}`);
  return res.json();
}

/** POST /v1/auth/signup — create the first admin (auto-login). Unauthenticated. */
export async function signup(args: {
  endpoint: string;
  username: string;
  password: string;
}): Promise<TokenPair> {
  const res = await globalThis.fetch(`${base(args.endpoint)}/v1/auth/signup`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: args.username, password: args.password }),
  });
  if (!res.ok) {
    const body = (await res.json().catch(() => null)) as
      | { error?: { message?: string } }
      | null;
    throw new Error(body?.error?.message ?? `signup failed (${res.status})`);
  }
  return res.json();
}

/** Ingest a small sample telemetry dataset so the instance is immediately useful. */
export async function ingestSample(
  t: KymaTransport,
  args: { database: string; table: string },
): Promise<void> {
  const now = Date.now();
  const services = ["payments-svc", "auth-svc", "orders-svc", "search-svc"];
  const levels = ["INFO", "INFO", "INFO", "WARN", "ERROR"];
  const msgs = [
    "request completed",
    "cache miss",
    "slow query detected",
    "retry scheduled",
    "card declined",
  ];
  const lines: string[] = [];
  for (let i = 0; i < 40; i++) {
    const lvl = levels[i % levels.length];
    // Values are sent as strings: a fresh table's columns are schema-inferred
    // as Utf8, so numeric JSON would be rejected by the ingest decoder.
    lines.push(
      JSON.stringify({
        timestamp: new Date(now - i * 1373).toISOString(),
        service_name: services[i % services.length],
        severity_text: lvl,
        status: String(lvl === "ERROR" ? 500 : lvl === "WARN" ? 429 : 200),
        latency_ms: String(20 + ((i * 37) % 900)),
        message: msgs[i % msgs.length],
      }),
    );
  }
  const res = await t.request("/v1/ingest", {
    method: "POST",
    headers: {
      "x-table": args.table,
      "content-type": "application/x-ndjson",
    },
    body: lines.join("\n"),
    database: args.database,
  });
  if (!res.ok) throw await errorFromResponse(res);
}
