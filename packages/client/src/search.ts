// Unified hybrid search client — POST /v1/search.
// Lexical + vector (RRF-fused) ranked search across sources in scope; hits
// carry `db.table` provenance + the row so callers can drill down with SQL/KQL.
//
// Modes:
//   "data"   (default) — structured rows from connected databases.
//   "memory" — agentic memory records; supports realm/tag/importance filters.
//   "graph"  — property-graph nodes/edges; supports hop expansion + label filter.
// All mode fields are optional; omitting `mode` preserves the pre-existing
// data-only behaviour and response shape.

import type { KymaTransport } from "./transport";
import type { Scope } from "./discover";
import { errorFromResponse } from "./errors";

export type HybridSearchRequest = {
  query: string;
  /** Scope of databases/tables to search. Optional — server defaults to all. */
  scope?: Scope;
  time_range?: { from: string; to: string } | null;
  limit?: number;

  // ── unified-search mode ────────────────────────────────────────────────────
  /** Search mode. Defaults to "data" when omitted. */
  mode?: "data" | "memory" | "graph";

  // memory-mode filters
  /** Memory realms to restrict the search to (e.g. ["logs", "traces"]). */
  realms?: string[];
  /** Filter by memory type (e.g. "observation", "insight"). */
  memory_type?: string;
  /** Filter by tag labels attached to memory records. */
  tags?: string[];
  /** Minimum importance score [0–1]; only records at or above this are returned. */
  importance_min?: number;
  /** ISO-8601 timestamp; return memories valid as-of this point in time. */
  as_of?: string;
  /** Include invalidated (soft-deleted) memory records. Defaults to false. */
  include_invalidated?: boolean;

  // graph-mode filters
  /** Name of the property graph to search (server uses the default graph if omitted). */
  graph?: string;
  /** Restrict graph search to nodes/edges with these labels. */
  labels?: string[];
  /** Number of hops to expand from matching nodes (0 = node-only, default 1). */
  expand_hops?: number;
};

export interface HybridSearchHit {
  /** `db.table` the row came from. */
  source: string;
  /** Fused RRF score (higher = more relevant). */
  score: number;
  row: Record<string, unknown>;

  // ── unified-envelope additions (present in memory/graph modes) ─────────────
  /** Entity kind: "memory", "node", "edge", etc. Absent in data mode. */
  kind?: string;
  /** Stable entity identifier within its store. */
  id?: string;
  /** Human-readable title or label for the hit. */
  title?: string;
  /** Short excerpt of the hit content suitable for display. */
  content_preview?: string;
  /** Memory type tag (e.g. "observation", "insight"). Memory mode only. */
  memory_type?: string;
}

export interface HybridSearchResponse {
  hits: HybridSearchHit[];
  sources_searched: number;
  elapsed_ms: number;

  // ── unified-envelope additions ─────────────────────────────────────────────
  /** Echo of the mode used server-side. Absent when mode was "data" (default). */
  mode?: string;
  /** Optional synthesised context string from the server (memory/graph modes). */
  context?: string;
  /** Related entities linked to the top hits (graph traversal results, etc.). */
  linked?: unknown[];
}

export async function search(t: KymaTransport, req: HybridSearchRequest): Promise<HybridSearchResponse> {
  const res = await t.request("/v1/search", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(req),
  });
  if (!res.ok) throw await errorFromResponse(res);
  return (await res.json()) as HybridSearchResponse;
}
