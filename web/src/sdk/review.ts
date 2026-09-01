// Memory approval-queue (Review inbox) client. Mirrors sdk/dreaming.ts: raw
// fetch against the server's /v1/agent/memory/review* endpoints.

import type { MemoryOp } from "@pensieve-ai/client";

export type ReviewMode = "gate" | "post_hoc";
export type ReviewStatus =
  | "pending"
  | "approved"
  | "rejected"
  | "auto_applied"
  | "rolled_back";
export type ReviewSource = "dreaming" | "realtime" | "file_promote" | "review";
export type ReviewAction = "approve" | "reject" | "undo";

/** One queue row, mirroring the server's QueueRow. `payload`/`inverse` are
 *  opaque op descriptors rendered generically by the card. */
export interface ReviewItem {
  id: string;
  realm: string;
  operation: MemoryOp;
  mode: ReviewMode;
  status: ReviewStatus;
  confidence: number | null;
  reason: string | null;
  source: ReviewSource;
  source_run_id: string | null;
  payload: Record<string, unknown>;
  inverse: Record<string, unknown> | null;
  created_at: string;
  resolved_at: string | null;
  resolved_by: string | null;
  resolution_comment: string | null;
}

export interface ReviewCounts {
  pending: number;
  post_hoc: number;
}

export interface ReviewFilter {
  status?: ReviewStatus;
  source?: ReviewSource;
  realm?: string;
  op?: MemoryOp;
  source_run_id?: string;
  limit?: number;
}

// ── Helpers (same shape as sdk/dreaming.ts) ──────────────────────────────────

type BaseArgs = { endpoint: string; token: string; database?: string };

function headers(token: string, database?: string): Record<string, string> {
  const h: Record<string, string> = {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
  if (database) h["x-database"] = database;
  return h;
}

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

// ── API ──────────────────────────────────────────────────────────────────────

export async function listReview(
  args: BaseArgs & ReviewFilter,
): Promise<ReviewItem[]> {
  const q = new URLSearchParams();
  if (args.status) q.set("status", args.status);
  if (args.source) q.set("source", args.source);
  if (args.realm) q.set("realm", args.realm);
  if (args.op) q.set("op", args.op);
  if (args.source_run_id) q.set("source_run_id", args.source_run_id);
  if (args.limit != null) q.set("limit", String(args.limit));
  const qs = q.toString();
  const res = await fetch(
    `${base(args.endpoint)}/v1/agent/memory/review${qs ? `?${qs}` : ""}`,
    { headers: headers(args.token, args.database) },
  );
  const body = await handleResponse<{ items: ReviewItem[] }>(res);
  return body.items ?? [];
}

export async function reviewCount(args: BaseArgs): Promise<ReviewCounts> {
  const res = await fetch(`${base(args.endpoint)}/v1/agent/memory/review/count`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<ReviewCounts>(res);
}

export async function resolveReview(
  args: BaseArgs & {
    id: string;
    action: ReviewAction;
    payload?: Record<string, unknown>;
    comment?: string;
  },
): Promise<{ ok: boolean; row: ReviewItem }> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/agent/memory/review/${args.id}/${args.action}`,
    {
      method: "POST",
      headers: headers(args.token, args.database),
      body: JSON.stringify({ payload: args.payload, comment: args.comment }),
    },
  );
  return handleResponse<{ ok: boolean; row: ReviewItem }>(res);
}

export async function bulkReview(
  args: BaseArgs & { ids: string[]; action: ReviewAction; comment?: string },
): Promise<{ results: { id: string; ok: boolean; status?: string; error?: string }[] }> {
  const res = await fetch(`${base(args.endpoint)}/v1/agent/memory/review/bulk`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify({ ids: args.ids, action: args.action, comment: args.comment }),
  });
  return handleResponse(res);
}
