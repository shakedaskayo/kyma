// HTTP NDJSON query client. Posts KQL/SQL to `/v1/query` and streams back
// NDJSON rows. Each iteration of the generator yields a snapshot of
// accumulated columns + rows so the caller can render incrementally.
//
// A Flight gRPC-web path existed here earlier but was removed: tonic-web's
// chunked-transfer termination and @connectrpc/connect-web's envelope reader
// disagreed on the end-of-stream condition, producing "incomplete envelope"
// on every query. HTTP NDJSON over the existing `/v1/query` surface ships
// the same data with one less protocol in the mix. Flight remains the
// server-to-server and agent-SDK path on :9090.

import type { KymaTransport } from "./transport";
import type { Column, ColKind } from "./arrow";
export type { Column, ColKind };
export type ResultChunk = { columns: Column[]; rows: Record<string, unknown>[] };

export type QueryArgs = {
  database: string;
  query: string;
  language: "kql" | "sql" | "cypher";
  /** Cypher only: "<db>/<graph>" identifying the target graph. */
  graph?: string;
  walMs?: number;
  memBytes?: number;
  signal?: AbortSignal;
};

export function encodeTicket(t: { database: string; query: string; language: string; graph?: string }): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(t));
}

const CHUNK_ROWS = 500;

export async function* runQuery(transport: KymaTransport, args: QueryArgs): AsyncGenerator<ResultChunk, void, void> {
  const contentType =
    args.language === "sql"    ? "application/sql" :
    args.language === "cypher" ? "application/x-cypher" :
                                 "application/x-kql";
  const extraHeaders: Record<string, string> = {
    "content-type": contentType,
  };
  if (args.language === "cypher" && args.graph) {
    extraHeaders["x-graph"] = args.graph;
  }
  if (args.walMs)    extraHeaders["x-kyma-max-wall-clock-ms"] = String(args.walMs);
  if (args.memBytes) extraHeaders["x-kyma-max-memory-bytes"]  = String(args.memBytes);

  const res = await transport.request("/v1/query", {
    method: "POST",
    headers: extraHeaders,
    body: args.query,
    signal: args.signal,
    database: args.database,
  });

  if (!res.ok) {
    let detail = "";
    try { detail = await res.text(); } catch { /* ignore */ }
    throw new Error(`query failed (${res.status}): ${detail.slice(0, 500)}`);
  }

  const reader = res.body?.getReader();
  if (!reader) {
    // Non-streaming fallback: decode whole body as NDJSON.
    const text = await res.text();
    const rows = parseNdjson(text);
    const columns = inferColumns(rows);
    if (rows.length > 0) yield { columns, rows };
    return;
  }

  const decoder = new TextDecoder();
  let leftover = "";
  let columns: Column[] | null = null;
  let pending: Record<string, unknown>[] = [];

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    leftover += decoder.decode(value, { stream: true });
    let nl;
    while ((nl = leftover.indexOf("\n")) >= 0) {
      const line = leftover.slice(0, nl).trim();
      leftover = leftover.slice(nl + 1);
      if (!line) continue;
      try {
        const row = JSON.parse(line) as Record<string, unknown>;
        pending.push(row);
        if (!columns) columns = inferColumnsFromRow(row);
      } catch { /* skip malformed line */ }
    }
    if (columns && pending.length >= CHUNK_ROWS) {
      yield { columns, rows: pending };
      pending = [];
    }
  }

  // Flush trailing incomplete line if it's a complete JSON object (no newline at EOF).
  const tail = leftover.trim();
  if (tail) {
    try {
      const row = JSON.parse(tail) as Record<string, unknown>;
      pending.push(row);
      if (!columns) columns = inferColumnsFromRow(row);
    } catch { /* ignore */ }
  }
  if (pending.length > 0) yield { columns: columns ?? [], rows: pending };
}

function parseNdjson(text: string): Record<string, unknown>[] {
  const out: Record<string, unknown>[] = [];
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try { out.push(JSON.parse(t) as Record<string, unknown>); } catch { /* skip */ }
  }
  return out;
}

function inferColumns(rows: Record<string, unknown>[]): Column[] {
  return rows.length > 0 ? inferColumnsFromRow(rows[0]) : [];
}

function inferColumnsFromRow(row: Record<string, unknown>): Column[] {
  return Object.keys(row).map((name) => ({ name, kind: columnKindOf(row[name]) }));
}

function columnKindOf(v: unknown): ColKind {
  if (v == null) return "other";
  if (typeof v === "number" || typeof v === "bigint") return "numeric";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "string") {
    // ISO-8601 timestamp heuristic: 2026-04-21T... or 2026-04-21 ...
    if (/^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2})?/.test(v)) return "time";
    return "string";
  }
  return "other";
}
