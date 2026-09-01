import type { Frame, Scope } from "@pensieve-ai/client";

export type Pill =
  | { kind: "substring"; value: string }
  | { kind: "eq"; field: string; value: string }
  | { kind: "neq"; field: string; value: string }
  | { kind: "cmp"; field: string; op: "gt" | "ge" | "lt" | "le"; value: string }
  | { kind: "exists"; field: string };

export type SourceKey = string; // "db.table"

export type SourceState = {
  source: SourceKey;
  hasTimestamp: boolean;
  timestampColumn: string | null;
  progress: "pending" | "running" | "done" | "error";
  rows: Record<string, unknown>[];
  total: number;
  capped: boolean;
  droppedClauses: unknown[];
  histogram?: { t: string; n: number }[];
  error?: { code: string; message: string };
};

export type DiscoverResultsState = {
  status: "idle" | "running" | "done" | "error" | "live";
  sources: Map<SourceKey, SourceState>;
  startedAt?: number;
  finishedAt?: number;
  topError?: { code: string; message: string };
};

export type { Frame, Scope };
