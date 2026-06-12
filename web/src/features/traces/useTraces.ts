import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSession } from "@/sdk/session";
import { utcIso, type SpanRow } from "./lib";

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
      const row = JSON.parse(t) as SpanRow;
      // Server timestamps are UTC but come without a timezone suffix.
      row.start_time = utcIso(row.start_time);
      row.end_time = utcIso(row.end_time);
      rows.push(row);
    } catch {
      /* skip non-row lines */
    }
  }
  return rows;
}

const esc = (s: string) => s.replace(/'/g, "\\'");

export const TRACE_PAGE_SIZE = 50;

/**
 * Root spans (= one row per trace), newest first, with optional filters.
 * Cursor-paginated: `loadMore()` fetches the next page strictly older than
 * the oldest row loaded so far, so pages stay stable while new spans arrive.
 */
export function useTraceList(opts: {
  agoExpr: string; // e.g. "ago(1h)"; "" = no time filter
  subject: string | null;
  service: string | null;
  text: string;
  errorsOnly: boolean;
  refreshKey: number;
}) {
  const { endpoint, token } = useSession();
  const [rows, setRows] = useState<SpanRow[]>([]);
  const [total, setTotal] = useState<number | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  // Bumped on every filter change so stale in-flight responses are dropped.
  const fetchGen = useRef(0);

  const filters = useMemo(() => {
    // Root spans have NULL parent_span_id (both the OTLP ingest path and the
    // self-trace exporter normalize empty → NULL); keep the '' match for rows
    // ingested before that normalization.
    const parts = [`| where isnull(parent_span_id) or parent_span_id == ''`];
    if (opts.agoExpr) parts.push(`| where start_time > ${opts.agoExpr}`);
    if (opts.subject) parts.push(`| where subject == '${esc(opts.subject)}'`);
    if (opts.service) parts.push(`| where service_name == '${esc(opts.service)}'`);
    if (opts.text.trim()) parts.push(`| where name contains '${esc(opts.text.trim())}'`);
    if (opts.errorsOnly) parts.push(`| where status_code == 'ERROR'`);
    return parts.join("\n");
  }, [opts.agoExpr, opts.subject, opts.service, opts.text, opts.errorsOnly]);

  const pageQuery = useCallback(
    (cursor: string | null) => {
      const parts = ["otel_traces", filters];
      // Strip the Z we added client-side: datetime() then casts to the same
      // naive-UTC type as the stored column.
      if (cursor) parts.push(`| where start_time < datetime("${cursor.replace(/Z$/, "")}")`);
      parts.push("| sort by start_time desc", `| take ${TRACE_PAGE_SIZE}`);
      return parts.join("\n");
    },
    [filters],
  );

  useEffect(() => {
    if (!endpoint || !token) return;
    const gen = ++fetchGen.current;
    setLoading(true);
    kql(endpoint, token, pageQuery(null))
      .then((r) => {
        if (fetchGen.current !== gen) return;
        setRows(r);
        setHasMore(r.length === TRACE_PAGE_SIZE);
        setError(null);
      })
      .catch((e: unknown) => {
        if (fetchGen.current === gen) setError(String(e));
      })
      .finally(() => {
        if (fetchGen.current === gen) setLoading(false);
      });
    // Window total (one row: {"n": N}) — best-effort, failures just hide it.
    kql(endpoint, token, ["otel_traces", filters, "| summarize n = count()"].join("\n"))
      .then((r) => {
        if (fetchGen.current !== gen) return;
        const n = (r[0] as unknown as { n?: number } | undefined)?.n;
        setTotal(typeof n === "number" ? n : null);
      })
      .catch(() => {
        if (fetchGen.current === gen) setTotal(null);
      });
  }, [endpoint, token, pageQuery, filters, opts.refreshKey]);

  const loadMore = useCallback(() => {
    if (!endpoint || !token || rows.length === 0 || loadingMore) return;
    const gen = fetchGen.current;
    const cursor = rows[rows.length - 1].start_time;
    setLoadingMore(true);
    kql(endpoint, token, pageQuery(cursor))
      .then((r) => {
        if (fetchGen.current !== gen) return;
        setRows((prev) => [...prev, ...r]);
        setHasMore(r.length === TRACE_PAGE_SIZE);
      })
      .catch((e: unknown) => {
        if (fetchGen.current === gen) setError(String(e));
      })
      .finally(() => {
        if (fetchGen.current === gen) setLoadingMore(false);
      });
  }, [endpoint, token, rows, loadingMore, pageQuery]);

  return { rows, total, hasMore, loadMore, error, loading, loadingMore };
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
