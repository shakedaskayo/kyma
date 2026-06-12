import { useEffect, useMemo, useState } from "react";
import { Activity, ChevronDown, RefreshCw, Search } from "lucide-react";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { fmtDurationNs, parseAttrs, splitOperation, type SpanRow } from "./lib";
import { useTraceList } from "./useTraces";

const RANGES: { label: string; ago: string }[] = [
  { label: "15m", ago: "ago(15m)" },
  { label: "1h", ago: "ago(1h)" },
  { label: "6h", ago: "ago(6h)" },
  { label: "24h", ago: "ago(24h)" },
  { label: "7d", ago: "ago(7d)" },
];

const METHOD_TONE: Record<string, string> = {
  GET: "border-sky-400/30 bg-sky-500/10 text-sky-300",
  POST: "border-violet-400/30 bg-violet-500/10 text-violet-300",
  PUT: "border-amber-400/30 bg-amber-500/10 text-amber-300",
  PATCH: "border-teal-400/30 bg-teal-500/10 text-teal-300",
  DELETE: "border-rose-400/30 bg-rose-500/10 text-rose-300",
};

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
  const [errorsOnly, setErrorsOnly] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const [live, setLive] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const { rows, total, hasMore, loadMore, error, loading, loadingMore } = useTraceList({
    agoExpr,
    subject,
    service: null,
    text,
    errorsOnly,
    refreshKey,
  });

  useEffect(() => {
    if (!live) return;
    const t = setInterval(() => {
      setNow(Date.now());
      setRefreshKey((k) => k + 1);
    }, 5000);
    return () => clearInterval(t);
  }, [live]);

  const subjects = useMemo(
    () => [...new Set(rows.map((r) => r.subject).filter((s): s is string => Boolean(s)))],
    [rows],
  );
  // Scale duration bars against the slowest span in view. sqrt keeps the
  // fast majority visible next to one slow outlier.
  const maxDur = useMemo(() => Math.max(1, ...rows.map((r) => r.duration_ns)), [rows]);

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex rounded-md border border-border/60 bg-card/30 p-0.5">
          {RANGES.map((r) => (
            <button
              key={r.label}
              onClick={() => setAgoExpr(r.ago)}
              className={cn(
                "rounded px-2 py-1 text-xs transition-colors",
                agoExpr === r.ago
                  ? "bg-violet-500/15 font-medium text-violet-200"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {r.label}
            </button>
          ))}
        </div>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
          <input
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="filter operations…"
            className="h-8 w-56 rounded-md border border-border/60 bg-card/40 pl-7 pr-2 text-xs outline-none transition-colors focus:border-violet-400/50"
          />
        </div>
        {subjects.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            {subjects.map((s) => (
              <button
                key={s}
                onClick={() => setSubject(subject === s ? null : s)}
                className={cn(
                  "rounded-full border px-2 py-0.5 font-mono text-[10px] transition-colors",
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
        <button
          onClick={() => setErrorsOnly((v) => !v)}
          className={cn(
            "flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs transition-colors",
            errorsOnly
              ? "border-rose-400/40 bg-rose-500/10 text-rose-300"
              : "border-border/60 text-muted-foreground hover:text-foreground",
          )}
        >
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              errorsOnly ? "bg-rose-400" : "bg-muted-foreground/50",
            )}
          />
          errors
        </button>
        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={() => setLive((v) => !v)}
            className={cn(
              "flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs transition-colors",
              live
                ? "border-emerald-400/40 bg-emerald-500/10 text-emerald-300"
                : "border-border/60 text-muted-foreground hover:text-foreground",
            )}
          >
            <span className="relative flex h-1.5 w-1.5">
              {live && (
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60" />
              )}
              <span
                className={cn(
                  "relative inline-flex h-1.5 w-1.5 rounded-full",
                  live ? "bg-emerald-400" : "bg-muted-foreground/50",
                )}
              />
            </span>
            live
          </button>
          <button
            onClick={() => {
              setNow(Date.now());
              setRefreshKey((k) => k + 1);
            }}
            className="flex h-8 items-center gap-1 rounded-md border border-border/60 px-2.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
          >
            <RefreshCw className={cn("h-3 w-3", loading && "animate-spin")} /> refresh
          </button>
        </div>
      </div>

      {error && <p className="text-xs text-rose-300">{error}</p>}

      <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-border/50">
        <table className="w-full text-left text-xs">
          <thead className="sticky top-0 z-10 bg-surface text-[10px] uppercase tracking-wider text-muted-foreground">
            <tr className="border-b border-border/50">
              <th className="w-16 px-3 py-2">status</th>
              <th className="w-24 px-2 py-2">time</th>
              <th className="px-2 py-2">operation</th>
              <th className="w-32 px-2 py-2">entity</th>
              <th className="w-28 px-2 py-2">service</th>
              <th className="w-48 px-3 py-2 text-right">duration</th>
            </tr>
          </thead>
          <tbody>
            {loading && rows.length === 0
              ? Array.from({ length: 8 }, (_, i) => <SkeletonRow key={i} />)
              : rows.map((r) => (
                  <TraceRow
                    key={r.span_id}
                    r={r}
                    now={now}
                    maxDur={maxDur}
                    active={selected === r.trace_id}
                    onSelect={onSelect}
                  />
                ))}
          </tbody>
        </table>
        {!error && !loading && rows.length === 0 && (
          <div className="flex flex-col items-center gap-2 px-6 py-16 text-center">
            <Activity className="h-6 w-6 text-muted-foreground/50" />
            <p className="text-sm text-foreground/80">
              {errorsOnly ? "No errors in this window" : "No traces in this window"}
            </p>
            <p className="max-w-md text-xs leading-relaxed text-muted-foreground">
              Kyma traces its own API operations as they happen — run a query or recall a memory
              and it shows up here. External services can ship OTLP spans to port 4317.
            </p>
          </div>
        )}
      </div>

      <div className="flex items-center justify-between px-1 text-xs text-muted-foreground">
        <span className="tabular-nums">
          {rows.length > 0 &&
            (total != null
              ? `${rows.length.toLocaleString()} of ${total.toLocaleString()} traces`
              : `${rows.length.toLocaleString()} traces`)}
        </span>
        {hasMore && (
          <button
            onClick={loadMore}
            disabled={loadingMore}
            className="flex items-center gap-1 rounded-md border border-border/60 px-2.5 py-1 transition-colors hover:text-foreground disabled:opacity-50"
          >
            {loadingMore ? (
              <RefreshCw className="h-3 w-3 animate-spin" />
            ) : (
              <ChevronDown className="h-3 w-3" />
            )}
            load older traces
          </button>
        )}
      </div>
    </div>
  );
}

function StatusCell({ r }: { r: SpanRow }) {
  const http = parseAttrs(r.attributes_json)["http.status"];
  const error = r.status_code === "ERROR";
  const ok = r.status_code === "OK";
  return (
    <span
      className="flex items-center gap-1.5"
      title={r.status_message ?? (ok || error ? r.status_code : "no status recorded")}
    >
      <span
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          error && "bg-rose-400",
          ok && "bg-emerald-400",
          !ok && !error && "border border-muted-foreground/50",
        )}
      />
      <span
        className={cn(
          "tabular-nums",
          error ? "text-rose-300" : ok ? "text-foreground/70" : "text-muted-foreground",
        )}
      >
        {http ?? (error ? "error" : ok ? "ok" : "—")}
      </span>
    </span>
  );
}

function TraceRow({
  r,
  now,
  maxDur,
  active,
  onSelect,
}: {
  r: SpanRow;
  now: number;
  maxDur: number;
  active: boolean;
  onSelect: (id: string | null) => void;
}) {
  const { method, path } = splitOperation(r.name);
  const barPct = Math.max(2, Math.sqrt(r.duration_ns / maxDur) * 100);
  const error = r.status_code === "ERROR";
  return (
    <tr
      onClick={() => onSelect(active ? null : r.trace_id)}
      className={cn(
        "cursor-pointer border-t border-border/40 transition-colors hover:bg-card/60",
        active && "bg-violet-500/[0.06] shadow-[inset_2px_0_0_0_theme(colors.violet.400)]",
      )}
    >
      <td className="whitespace-nowrap px-3 py-2">
        <StatusCell r={r} />
      </td>
      <td
        className="whitespace-nowrap px-2 py-2 tabular-nums text-muted-foreground"
        title={new Date(r.start_time).toLocaleString()}
      >
        {relTime(r.start_time, now)}
      </td>
      <td className="max-w-[24rem] px-2 py-2">
        <span className="flex items-center gap-1.5">
          {method && (
            <span
              className={cn(
                "shrink-0 rounded border px-1 py-px font-mono text-[9px] font-semibold",
                METHOD_TONE[method] ?? "border-border/60 text-muted-foreground",
              )}
            >
              {method}
            </span>
          )}
          <span className="truncate font-mono text-[11px] text-foreground/90" title={r.name}>
            {path}
          </span>
        </span>
      </td>
      <td className="truncate px-2 py-2 font-mono text-[11px] text-foreground/70">
        {r.subject ?? "—"}
      </td>
      <td className="truncate px-2 py-2 text-muted-foreground">{r.service_name ?? "—"}</td>
      <td className="px-3 py-2">
        <span className="flex items-center justify-end gap-2">
          <span className="tabular-nums text-foreground/80">{fmtDurationNs(r.duration_ns)}</span>
          <span className="relative h-1 w-20 shrink-0 overflow-hidden rounded-full bg-border/30">
            <span
              className={cn(
                "absolute inset-y-0 left-0 rounded-full",
                error ? "bg-rose-400/80" : "bg-violet-400/70",
              )}
              style={{ width: `${barPct}%` }}
            />
          </span>
        </span>
      </td>
    </tr>
  );
}

function SkeletonRow() {
  return (
    <tr className="border-t border-border/40">
      {[16, 12, 56, 20, 16, 24].map((w, i) => (
        <td key={i} className={cn("px-3 py-2.5", i === 5 && "text-right")}>
          <span
            className="inline-block h-3 animate-pulse rounded bg-border/40"
            style={{ width: `${w * 4}px` }}
          />
        </td>
      ))}
    </tr>
  );
}
