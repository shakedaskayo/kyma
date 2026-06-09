/** NDJSON rows for /v1/query fixture responses. */

export interface QueryRow {
  ts: string;
  level: string;
  service: string;
  msg: string;
  host: string;
  duration_ms?: number;
}

export const QUERY_ROWS: QueryRow[] = [
  { ts: "2026-06-01T10:00:01.234Z", level: "info",  service: "auth-service",    msg: "User login: alice@acme.com",       host: "node-01", duration_ms: 12.3 },
  { ts: "2026-06-01T10:00:02.117Z", level: "warn",  service: "billing-service", msg: "Payment retry #2 for order 9821", host: "node-02", duration_ms: 245.6 },
  { ts: "2026-06-01T10:00:03.409Z", level: "error", service: "billing-service", msg: "Payment gateway timeout",          host: "node-02", duration_ms: 5001.1 },
  { ts: "2026-06-01T10:00:04.882Z", level: "info",  service: "user-service",    msg: "Profile update: bob@acme.com",     host: "node-01", duration_ms: 8.7 },
  { ts: "2026-06-01T10:00:05.013Z", level: "info",  service: "notify-service",  msg: "Email queued for delivery",        host: "node-03", duration_ms: 3.1 },
  { ts: "2026-06-01T10:00:06.781Z", level: "debug", service: "auth-service",    msg: "Token refresh: carol@acme.com",    host: "node-01", duration_ms: 5.9 },
  { ts: "2026-06-01T10:00:07.220Z", level: "info",  service: "api-gateway",     msg: "Upstream health check OK",         host: "node-gw", duration_ms: 1.2 },
  { ts: "2026-06-01T10:00:08.543Z", level: "warn",  service: "user-service",    msg: "Rate limit approaching: bob",      host: "node-01", duration_ms: 2.8 },
];

export function makeNdjsonBody(rows: object[]): string {
  return rows.map((r) => JSON.stringify(r)).join("\n");
}
