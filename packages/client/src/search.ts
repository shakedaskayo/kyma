// Unified hybrid search client — POST /v1/search.
// Lexical + vector (RRF-fused) ranked search across sources in scope; hits
// carry `db.table` provenance + the row so callers can drill down with SQL/KQL.

import type { KymaTransport } from "./transport";
import type { Scope } from "./discover";
import { errorFromResponse } from "./errors";

export type HybridSearchRequest = {
  query: string;
  scope: Scope;
  time_range?: { from: string; to: string } | null;
  limit?: number;
};

export interface HybridSearchHit {
  /** `db.table` the row came from. */
  source: string;
  /** Fused RRF score (higher = more relevant). */
  score: number;
  row: Record<string, unknown>;
}

export interface HybridSearchResponse {
  hits: HybridSearchHit[];
  sources_searched: number;
  elapsed_ms: number;
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
