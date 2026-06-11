import { useEffect, useMemo, useState } from "react";
import { RefreshCw } from "lucide-react";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { fmtDurationNs, STATUS_TONE, type SpanRow } from "./lib";
import { useTraceList } from "./useTraces";

const RANGES: { label: string; ago: string }[] = [
  { label: "15m", ago: "ago(15m)" },
  { label: "1h", ago: "ago(1h)" },
  { label: "6h", ago: "ago(6h)" },
  { label: "24h", ago: "ago(24h)" },
  { label: "7d", ago: "ago(7d)" },
];

export function TracesList({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (traceId: string | null) => void;
}) {
  const [agoExpr, setAgoExpr] = useState("ago(1h)");
  const [subject, setSubject] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [refreshKey, setRefreshKey] = useState(0);
  const [live, setLive] = useState(false);
  const [now] = useState(() => Date.now());
  const { rows, error, loading } = useTraceList({
    agoExpr,
    subject,
    service: null,
    text,
    refreshKey,
  });

  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => setRefreshKey((k) => k + 1), 5000);
    return () => clearInterval(t);
  }, [live]);

  const subjects = useMemo(
    () => [...new Set(rows.map((r) => r.subject).filter((s): s is string => Boolean(s)))],
    [rows],
  );

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex rounded-md border border-border/60 p-0.5">
          {RANGES.map((r) => (
            <button
              key={r.label}
              onClick={() => setAgoExpr(r.ago)}
              className={cn(
                "rounded px-2 py-1 text-xs",
                agoExpr === r.ago
                  ? "bg-card text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {r.label}
            </button>
          ))}
        </div>
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder="filter by operation…"
          className="h-8 w-56 rounded-md border border-border/60 bg-card/40 px-2 text-xs outline-none focus:border-border/80"
        />
        {subjects.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            {subjects.map((s) => (
              <button
                key={s}
                onClick={() => setSubject(subject === s ? null : s)}
                className={cn(
                  "rounded-full border px-2 py-0.5 font-mono text-[10px]",
                  subject === s
                    ? "border-violet-400/60 bg-violet-500/15 text-violet-200"
                    : "border-border/60 text-muted-foreground hover:text-foreground",
                )}
              >
                {s}
              </button>
            ))}
          </div>
        )}
        <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <input
            type="checkbox"
            checked={live}
            onChange={(e) => setLive(e.target.checked)}
            className="h-3.5 w-3.5 accent-violet-400"
          />
          live
        </label>
        <button
          onClick={() => setRefreshKey((k) => k + 1)}
          className="flex h-8 items-center gap-1 rounded-md border border-border/60 px-2 text-xs text-muted-foreground hover:text-foreground"
        >
          <RefreshCw className={cn("h-3 w-3", loading && "animate-spin")} /> refresh
        </button>
      </div>

      {error && <p className="text-xs text-rose-300">{error}</p>}
      {!error && rows.length === 0 && !loading && (
        <p className="px-1 py-8 text-center text-xs text-muted-foreground">
          No traces in this window. Kyma's own API operations appear here as they happen; external
          services can ship spans to the OTLP endpoint (port 4317).
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-border/50">
        <table className="w-full text-left text-xs">
          <thead className="sticky top-0 bg-surface text-xs uppercase tracking-wider text-muted-foreground">
            <tr>
              <th className="px-3 py-2">time</th>
              <th className="px-3 py-2">operation</th>
              <th className="px-3 py-2">entity</th>
              <th className="px-3 py-2">service</th>
              <th className="px-3 py-2 text-right">duration</th>
              <th className="px-3 py-2">status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <TraceRow
                key={r.span_id}
                r={r}
                now={now}
                active={selected === r.trace_id}
                onSelect={onSelect}
              />
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function TraceRow({
  r,
  now,
  active,
  onSelect,
}: {
  r: SpanRow;
  now: number;
  active: boolean;
  onSelect: (id: string | null) => void;
}) {
  return (
    <tr
      onClick={() => onSelect(active ? null : r.trace_id)}
      className={cn(
        "cursor-pointer border-t border-border/40 transition-colors hover:bg-card/60",
        active && "bg-card/80",
      )}
    >
      <td className="whitespace-nowrap px-3 py-2 tabular-nums text-muted-foreground">
        {relTime(r.start_time, now)}
      </td>
      <td className="max-w-[22rem] truncate px-3 py-2 text-foreground/90" title={r.name}>
        {r.name}
      </td>
      <td className="px-3 py-2 font-mono text-[11px] text-foreground/75">{r.subject ?? "—"}</td>
      <td className="px-3 py-2 text-muted-foreground">{r.service_name ?? "—"}</td>
      <td className="px-3 py-2 text-right tabular-nums">{fmtDurationNs(r.duration_ns)}</td>
      <td className={cn("px-3 py-2", STATUS_TONE[r.status_code] ?? STATUS_TONE["UNSET"])}>
        {r.status_code}
      </td>
    </tr>
  );
}
