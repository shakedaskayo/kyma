import { useCallback, useEffect, useState } from "react";
import { useSession } from "@/sdk/session";
import { type SpanRow } from "./lib";

async function kql(endpoint: string, token: string, query: string): Promise<SpanRow[]> {
  const res = await fetch(`${endpoint.replace(/\/$/, "")}/v1/query`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/x-kql",
      "x-database": "otel",
    },
    body: query,
  });
  if (!res.ok) throw new Error(`query failed (${res.status})`);
  const rows: SpanRow[] = [];
  for (const line of (await res.text()).split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try {
      rows.push(JSON.parse(t) as SpanRow);
    } catch {
      /* skip non-row lines */
    }
  }
  return rows;
}

const esc = (s: string) => s.replace(/'/g, "\\'");

/** Root spans (= one row per trace), newest first, with optional filters. */
export function useTraceList(opts: {
  agoExpr: string; // e.g. "ago(1h)"; "" = no time filter
  subject: string | null;
  service: string | null;
  text: string;
  refreshKey: number;
}) {
  const { endpoint, token } = useSession();
  const [rows, setRows] = useState<SpanRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!endpoint || !token) return;
    let cancelled = false;
    const parts = ["otel_traces", `| where parent_span_id == ''`];
    if (opts.agoExpr) parts.push(`| where start_time > ${opts.agoExpr}`);
    if (opts.subject) parts.push(`| where subject == '${esc(opts.subject)}'`);
    if (opts.service) parts.push(`| where service_name == '${esc(opts.service)}'`);
    if (opts.text.trim()) parts.push(`| where name contains '${esc(opts.text.trim())}'`);
    parts.push("| sort by start_time desc", "| take 100");
    setLoading(true);
    kql(endpoint, token, parts.join("\n"))
      .then((r) => { if (!cancelled) { setRows(r); setError(null); } })
      .catch((e: unknown) => { if (!cancelled) setError(String(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [endpoint, token, opts.agoExpr, opts.subject, opts.service, opts.text, opts.refreshKey]);

  return { rows, error, loading };
}

/** All spans of one trace (capped) for the waterfall. */
export function useTraceSpans(traceId: string | null) {
  const { endpoint, token } = useSession();
  const [rows, setRows] = useState<SpanRow[]>([]);
  const [loading, setLoading] = useState(false);
  const load = useCallback(() => {
    if (!endpoint || !token || !traceId) { setRows([]); return; }
    setLoading(true);
    kql(endpoint, token,
      `otel_traces | where trace_id == '${esc(traceId)}' | sort by start_time asc | take 500`)
      .then(setRows)
      .catch(() => setRows([]))
      .finally(() => setLoading(false));
  }, [endpoint, token, traceId]);
  useEffect(load, [load]);
  return { rows, loading };
}
