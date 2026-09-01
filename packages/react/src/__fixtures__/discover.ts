import type { Frame } from "@pensieve-ai/client";

export const DISCOVER_FRAMES: Frame[] = [
  {
    type: "plan",
    sources: [
      { source: "production.events", has_timestamp: true, timestamp_column: "ts" },
      { source: "production.traces", has_timestamp: true, timestamp_column: "ts" },
    ],
  },
  { type: "source_progress", source: "production.events", state: "running" },
  {
    type: "rows",
    source: "production.events",
    rows: [
      { ts: "2026-06-01T10:00:01.234Z", level: "info",  service: "auth-service",    msg: "User login: alice@acme.com" },
      { ts: "2026-06-01T10:00:02.117Z", level: "warn",  service: "billing-service", msg: "Payment retry #2 for order 9821" },
      { ts: "2026-06-01T10:00:03.409Z", level: "error", service: "billing-service", msg: "Payment gateway timeout" },
      { ts: "2026-06-01T10:00:04.882Z", level: "info",  service: "user-service",    msg: "Profile update: bob@acme.com" },
      { ts: "2026-06-01T10:00:05.013Z", level: "info",  service: "notify-service",  msg: "Email queued for delivery" },
    ],
  },
  { type: "source_done", source: "production.events", total: 5, capped: false, dropped_clauses: [] },
  { type: "source_progress", source: "production.traces", state: "running" },
  {
    type: "rows",
    source: "production.traces",
    rows: [
      { ts: "2026-06-01T10:00:03.100Z", trace_id: "abc123", service: "auth-service", operation: "login", duration_ms: 12.3, status: "ok" },
      { ts: "2026-06-01T10:00:03.200Z", trace_id: "abc123", service: "user-service", operation: "lookup", duration_ms: 3.1, status: "ok" },
    ],
  },
  { type: "source_done", source: "production.traces", total: 2, capped: false, dropped_clauses: [] },
  { type: "done", elapsed_ms: 87 },
];

export const DISCOVER_SOURCES = [
  { source: "production.events", has_timestamp: true,  timestamp_column: "ts" },
  { source: "production.traces", has_timestamp: true,  timestamp_column: "ts" },
  { source: "production.metrics", has_timestamp: true, timestamp_column: "ts" },
];
