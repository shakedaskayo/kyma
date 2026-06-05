import { authFetch } from "./auth-fetch";

export type Frame =
  // '?' tolerates old servers that omit the field; null = source has no timestamp column
  | { type: "plan"; sources: { source: string; has_timestamp: boolean; timestamp_column?: string | null }[] }
  | { type: "source_progress"; source: string; state: "running" }
  | { type: "rows"; source: string; rows: Record<string, unknown>[] }
  | { type: "histogram"; source: string; buckets: { t: string; n: number }[] }
  | { type: "source_done"; source: string; total: number; capped: boolean; dropped_clauses: unknown[] }
  | { type: "error"; source?: string; code: string; message: string }
  | { type: "done"; elapsed_ms: number }
  | { type: "live" }
  | { type: "heartbeat" };

export type Scope =
  | { kind: "all" }
  | { kind: "sources"; sources: string[] }
  | { kind: "view"; view_id: string };

export type SearchRequest = {
  query: string;
  scope: Scope;
  time_range?: { from: string; to: string } | null;
  per_source_limit?: number;
  histogram?: { interval_ms: number };
};

export class DiscoverError extends Error {
  constructor(public code: string, message: string, public status?: number) {
    super(message);
  }
}

export async function* searchDiscover(
  req: SearchRequest,
  signal?: AbortSignal,
): AsyncGenerator<Frame, void, unknown> {
  const resp = await authFetch("/v1/explore/search", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(req),
    signal,
  });
  if (!resp.ok) {
    let detail: unknown = null;
    try { detail = await resp.json(); } catch { /* ignore */ }
    const errBody = (detail as { error?: { code?: string; message?: string } })?.error;
    throw new DiscoverError(
      errBody?.code ?? `http_${resp.status}`,
      errBody?.message ?? resp.statusText,
      resp.status,
    );
  }
  if (!resp.body) throw new DiscoverError("no_body", "empty response body", resp.status);
  yield* parseNdjsonStream(resp.body);
}

export async function* parseNdjsonStream(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<Frame, void, unknown> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  while (true) {
    const { value, done } = await reader.read();
    if (value) buf += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      yield JSON.parse(line) as Frame;
    }
    if (done) break;
  }
  const tail = buf.trim();
  if (tail) yield JSON.parse(tail) as Frame;
}

export type SavedView = {
  id: string;
  name: string;
  sources: string[];
  columns: unknown | null;
  created_at: string;
  updated_at: string;
};

export async function listSavedViews(): Promise<SavedView[]> {
  const r = await authFetch("/v1/explore/views");
  if (!r.ok) throw new DiscoverError("list_failed", await r.text(), r.status);
  return r.json();
}

export async function createSavedView(name: string, sources: string[]): Promise<SavedView> {
  const r = await authFetch("/v1/explore/views", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name, sources }),
  });
  if (!r.ok) throw new DiscoverError("create_failed", await r.text(), r.status);
  return r.json();
}

export async function deleteSavedView(id: string): Promise<void> {
  const r = await authFetch(`/v1/explore/views/${id}`, { method: "DELETE" });
  if (!r.ok && r.status !== 204) throw new DiscoverError("delete_failed", await r.text(), r.status);
}
