/**
 * KymaExplore — the unified data-exploration surface.
 *
 * One smart input that auto-detects keyword search vs raw KQL/SQL and routes to
 * the matching engine, sharing a single shell: scope + time-range + Run, a
 * results area, and a row-detail drawer.
 *
 *   keyword → useKymaSearch (/v1/search): instant hybrid (lexical + vector,
 *             RRF-fused) ranked hits across all sources in scope, with db.table
 *             provenance. Click a hit to inspect; switch to KQL/SQL to correlate.
 *   kql/sql → useKymaQuery (/v1/query): schema browser → field-stats rail,
 *             timestamp histogram, results grid, chart, row drawer.
 *
 * The detected mode is a clickable badge so auto-detection is never a mystery.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square, Search as SearchIcon, Code2, Network } from "lucide-react";
import { autoChartAxes } from "@kyma-ai/client";
import type { Column, HybridSearchHit, SchemaDoc } from "@kyma-ai/client";

import { KymaErrorBoundary } from "../internal/KymaErrorBoundary";
import { Button } from "../internal/ui/button";
import { KymaContext, useKymaClient, useKymaContext } from "../provider/context";
import { cn } from "../internal/cn";

import { useKymaQuery } from "../hooks/useKymaQuery";
import { useKymaSearch } from "../hooks/useKymaSearch";
import { RowDetailDrawer } from "../discover/RowDetailDrawer";
import { serializePills } from "../discover/discoverGrammar";
import { resolveTimeRange } from "../discover/useDiscoverSearch";
import { formatCell } from "../discover/columns";
import { colorForKey } from "../internal/data-palette";
import type { Pill, Scope } from "../discover/types";

import { SchemaBrowser } from "../query/schema/SchemaBrowser";
import { ResultsGrid } from "../query/results/ResultsGrid";
import { RowFilter } from "../query/results/RowFilter";
import { ChartPanel } from "../query/chart/ChartPanel";
import { HistogramTimeline } from "../discover/HistogramTimeline";
import { FieldStats } from "../discover/FieldStats";
import { TimeRangePicker } from "../query/time-range/TimeRangePicker";
import { prependTimeFilter } from "../query/time-range/time-range";
import type { TimeRange } from "../query/time-range/time-range-types";

import { detectMode, modeLabel, type ExploreMode } from "./detectMode";

export interface KymaExploreProps {
  /** Initial input text. */
  defaultQuery?: string;
  /** Initial time range. Defaults to "all time" so any data shows on first run. */
  timeRange?: TimeRange;
  /**
   * Database scope. "*" (default) searches/queries across all databases; a
   * concrete name filters. Drives `x-database` for KQL/SQL and the source glob
   * for keyword search.
   */
  database?: string;
  className?: string;
  style?: React.CSSProperties;
  fallback?: React.ReactNode;
  /**
   * Run a search/query automatically on mount so the surface lands with data
   * instead of an empty state — re-running the (persisted) `defaultQuery`, or,
   * when it's empty, an empty keyword search that returns the most recent rows
   * across the sources in scope. Defaults to `true`. Set `false` for embedders
   * that don't want an automatic request on mount.
   */
  autoRun?: boolean;
  onQueryChange?: (q: string) => void;
  /**
   * Called when the user clicks "View in graph" on a result that is a graph
   * entity (a row from a `*_nodes` / file-candidate table with an `id`). The
   * consumer deep-links to the graph page focused on that node.
   */
  onViewInGraph?: (nodeId: string, db: string) => void;
}

function KymaExploreInner({
  defaultQuery = "",
  timeRange: timeRangeProp,
  database = "*",
  className,
  style,
  autoRun = true,
  onQueryChange,
  onViewInGraph,
}: Omit<KymaExploreProps, "fallback">) {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;

  const [input, setInput] = useState(defaultQuery);
  const [forcedMode, setForcedMode] = useState<ExploreMode | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>(timeRangeProp ?? { preset: "none" });
  const [rowFilter, setRowFilter] = useState("");
  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);
  const [submittedMode, setSubmittedMode] = useState<ExploreMode | null>(null);
  // Selected columns for the search document table (Kibana-style field pinning).
  const [cols, setCols] = useState<string[]>([]);

  const { data: schema } = useQuery<SchemaDoc>({
    queryKey: ["kyma", endpoint, "explore-schema"],
    queryFn: () => client.catalog.fetchSchema(),
    staleTime: 5 * 60_000,
  });
  const tableNames = useMemo(
    () => (schema?.databases ?? []).flatMap((db) => db.tables.map((t) => t.name)),
    [schema],
  );

  const liveMode: ExploreMode = forcedMode ?? detectMode(input, tableNames);

  const scope: Scope = useMemo(
    () => (database && database !== "*" ? { kind: "sources", sources: [`${database}.*`] } : { kind: "all" }),
    [database],
  );

  const search = useKymaSearch();
  const { columns: qCols, rows: qRows, isRunning: qRunning, error: qError, execute, cancel: cancelQuery } =
    useKymaQuery();

  const inputRef = useRef(input);
  inputRef.current = input;

  const run = () => {
    const text = inputRef.current;
    const mode = forcedMode ?? detectMode(text, tableNames);
    setRowFilter("");
    setOpenRow(null);
    setSubmittedMode(mode);
    if (mode === "search") {
      void search.run({
        query: text,
        scope,
        time_range: resolveTimeRange(timeRange),
        limit: 100,
      });
    } else {
      const effective = mode === "kql" ? prependTimeFilter(text, timeRange, schema) : text;
      void execute({ database, query: effective, language: mode }).catch(() => {});
    }
  };

  const cancel = () => cancelQuery();

  // Auto-run on mount so the page lands with data instead of the empty state:
  // re-run the persisted input, or — when it's empty — an empty keyword search
  // that returns the most recent rows across the sources in scope. A non-empty
  // input waits for the schema so a bare table name is classified as KQL (not a
  // keyword search); an empty input always detects as "search" and runs at once.
  // The instance is keyed per tab + scope upstream, so this fires once per tab
  // activation / scope change.
  const autoRan = useRef(false);
  useEffect(() => {
    if (!autoRun || autoRan.current) return;
    if (inputRef.current.trim() && !schema) return;
    autoRan.current = true;
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoRun, schema]);

  const setInputAndNotify = (v: string) => {
    setInput(v);
    onQueryChange?.(v);
  };
  const appendToInput = (text: string) => {
    setInputAndNotify(input.trim() ? `${input.trim()} ${text}` : text);
  };

  const running = search.isRunning || qRunning;
  const showQuery = submittedMode === "kql" || submittedMode === "sql";

  return (
    <div
      className={cn(
        "ky-flex ky-h-full ky-flex-col ky-overflow-hidden ky-bg-background ky-text-foreground",
        className,
      )}
      style={style}
    >
      {/* ── Smart input bar ── */}
      <div className="ky-flex ky-items-start ky-gap-2 ky-border-b ky-p-3">
        <button
          type="button"
          title="Detected mode — click to force the other mode (click again for auto)"
          onClick={() => setForcedMode(forcedMode ? null : liveMode === "search" ? "kql" : "search")}
          className={cn(
            "ky-mt-0.5 ky-flex ky-shrink-0 ky-items-center ky-gap-1 ky-rounded ky-border ky-px-2 ky-py-1 ky-text-[11px] ky-font-medium",
            liveMode === "search" ? "ky-text-muted-foreground" : "ky-border-primary/40 ky-text-primary",
            forcedMode && "ky-ring-1 ky-ring-primary/40",
          )}
        >
          {liveMode === "search" ? <SearchIcon className="ky-h-3 ky-w-3" /> : <Code2 className="ky-h-3 ky-w-3" />}
          {modeLabel(liveMode)}
        </button>
        <textarea
          rows={1}
          value={input}
          spellCheck={false}
          placeholder='Search keywords (e.g. auth service:payments) — or type KQL/SQL (e.g. claude_code_events | take 50)'
          onChange={(e) => setInputAndNotify(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey || liveMode === "search")) {
              e.preventDefault();
              run();
            }
          }}
          className="ky-min-h-[36px] ky-max-h-40 ky-flex-1 ky-resize-y ky-rounded-md ky-border ky-bg-background ky-px-3 ky-py-2 ky-font-mono ky-text-xs focus:ky-outline-none focus:ky-ring-1 focus:ky-ring-primary/40"
        />
        <TimeRangePicker value={timeRange} onChange={setTimeRange} />
        {running ? (
          <Button size="sm" variant="destructive" onClick={cancel}>
            <Square className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={run}>
            <Play className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Run
            <kbd className="ky-ml-2 ky-rounded ky-bg-primary-foreground/20 ky-px-1 ky-text-[10px]">⌘↵</kbd>
          </Button>
        )}
      </div>

      {/* ── Body ── */}
      {submittedMode === null ? (
        <EmptyState />
      ) : showQuery ? (
        <QueryBody
          schema={schema}
          columns={qCols}
          rows={qRows}
          isRunning={qRunning}
          error={qError}
          rowFilter={rowFilter}
          onRowFilter={setRowFilter}
          onInsert={appendToInput}
          onReplaceAndRun={(kql) => {
            setForcedMode("kql");
            setInputAndNotify(kql);
            setSubmittedMode("kql");
            void execute({ database, query: prependTimeFilter(kql, timeRange, schema), language: "kql" }).catch(() => {});
          }}
          onOpenRow={(row) => setOpenRow({ source: String(row["__database"] ?? "result"), row })}
        />
      ) : (
        <SearchResults
          hits={search.hits}
          sourcesSearched={search.sourcesSearched}
          elapsedMs={search.elapsedMs}
          ran={search.ran}
          isRunning={search.isRunning}
          error={search.error}
          cols={cols}
          onToggleCol={(f) => setCols((c) => (c.includes(f) ? c.filter((x) => x !== f) : [...c, f]))}
          onAddFilter={(field, value) =>
            appendToInput(/\s/.test(value) ? `${field}:"${value}"` : `${field}:${value}`)
          }
          onZoom={(from, to) => {
            setTimeRange({ preset: "custom", from, to });
            void search.run({ query: inputRef.current, scope, time_range: { from, to }, limit: 100 });
          }}
          onOpenRow={(source, row) => setOpenRow({ source, row })}
          onViewInGraph={onViewInGraph}
        />
      )}

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={(p: Pill) => {
          appendToInput(serializePills([p]));
          setOpenRow(null);
        }}
      />
    </div>
  );
}

// ── Keyword (hybrid search) body: Kibana-style Discover ───────────────────────
//
// Fields rail · time histogram · expandable document table. Composes the hybrid
// search hits into the familiar observability-console layout.

type FieldKind = "time" | "number" | "bool" | "text" | "json" | "null";

const HIDDEN_FIELD = (k: string) => k.startsWith("__");
const TIME_NAMES = ["ts", "timestamp", "at", "time", "event_time", "observed_time", "_time", "created_at", "date"];
const ISO_RE = /^\d{4}-\d\d-\d\dT\d\d:\d\d/;

function fieldKind(v: unknown): FieldKind {
  if (v == null) return "null";
  if (typeof v === "number") return "number";
  if (typeof v === "boolean") return "bool";
  if (typeof v === "string") return ISO_RE.test(v) ? "time" : "text";
  return "json";
}

const KIND_GLYPH: Record<FieldKind, string> = {
  time: "◷",
  number: "#",
  bool: "⊤",
  text: "T",
  json: "{}",
  null: "∅",
};

function aggregateFields(hits: HybridSearchHit[]): { name: string; kind: FieldKind; count: number }[] {
  const m = new Map<string, { kind: FieldKind; count: number }>();
  for (const h of hits) {
    for (const [k, v] of Object.entries(h.row)) {
      if (HIDDEN_FIELD(k)) continue;
      const cur = m.get(k);
      const kind = fieldKind(v);
      if (!cur) m.set(k, { kind, count: 1 });
      else {
        cur.count += 1;
        if (cur.kind === "null" && kind !== "null") cur.kind = kind;
      }
    }
  }
  return Array.from(m.entries())
    .map(([name, { kind, count }]) => ({ name, kind, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

function detectTimeField(hits: HybridSearchHit[]): string | null {
  const sample = hits.slice(0, 8);
  for (const name of TIME_NAMES) {
    if (sample.some((h) => typeof h.row[name] === "string" && ISO_RE.test(h.row[name] as string))) return name;
  }
  for (const h of sample) {
    for (const [k, v] of Object.entries(h.row)) {
      if (!HIDDEN_FIELD(k) && typeof v === "string" && ISO_RE.test(v)) return k;
    }
  }
  return null;
}

/** Top values of a field across hits, for the field popover. */
function topValues(hits: HybridSearchHit[], field: string, n = 5): { value: string; pct: number }[] {
  const counts = new Map<string, number>();
  let total = 0;
  for (const h of hits) {
    const v = h.row[field];
    if (v == null || v === "") continue;
    const s = formatCell(v);
    counts.set(s, (counts.get(s) ?? 0) + 1);
    total += 1;
  }
  if (total === 0) return [];
  return Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, n)
    .map(([value, c]) => ({ value, pct: Math.round((c / total) * 100) }));
}

function SourceBadge({ source }: { source: string }) {
  return (
    <span
      className="ky-inline-flex ky-shrink-0 ky-items-center ky-gap-1 ky-rounded ky-px-1.5 ky-py-0.5 ky-font-mono ky-text-[10px]"
      style={{ background: `${colorForKey(source)}1a`, color: colorForKey(source) }}
      title={source}
    >
      <span className="ky-h-1.5 ky-w-1.5 ky-rounded-full" style={{ background: colorForKey(source) }} />
      {source}
    </span>
  );
}

function FieldRow(props: {
  name: string;
  kind: FieldKind;
  selected: boolean;
  hits: HybridSearchHit[];
  onToggleCol: () => void;
  onAddFilter: (field: string, value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const top = open ? topValues(props.hits, props.name) : [];
  return (
    <li className="ky-border-b ky-border-border/40">
      <div className="ky-group ky-flex ky-items-center ky-gap-1.5 ky-px-2 ky-py-1 hover:ky-bg-accent/40">
        <span
          title={props.kind}
          className="ky-flex ky-h-4 ky-w-4 ky-shrink-0 ky-items-center ky-justify-center ky-rounded ky-bg-muted ky-text-[9px] ky-font-mono ky-text-muted-foreground"
        >
          {KIND_GLYPH[props.kind]}
        </span>
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="ky-min-w-0 ky-flex-1 ky-truncate ky-text-left ky-font-mono ky-text-[11px]"
          title={props.name}
        >
          {props.name}
        </button>
        <button
          type="button"
          title={props.selected ? "Remove column" : "Add as column"}
          onClick={props.onToggleCol}
          className={cn(
            "ky-shrink-0 ky-rounded ky-px-1 ky-text-[11px] ky-opacity-0 group-hover:ky-opacity-100",
            props.selected ? "ky-text-primary ky-opacity-100" : "ky-text-muted-foreground hover:ky-text-foreground",
          )}
        >
          {props.selected ? "✓" : "＋"}
        </button>
      </div>
      {open && (
        <div className="ky-space-y-1 ky-bg-muted/30 ky-px-2 ky-py-1.5">
          {top.length === 0 ? (
            <div className="ky-text-[10px] ky-text-muted-foreground">No values in results.</div>
          ) : (
            top.map((t) => (
              <button
                key={t.value}
                type="button"
                onClick={() => props.onAddFilter(props.name, t.value)}
                title={`Filter ${props.name} = ${t.value}`}
                className="ky-flex ky-w-full ky-items-center ky-gap-2 ky-text-left"
              >
                <span className="ky-min-w-0 ky-flex-1 ky-truncate ky-font-mono ky-text-[10px]">{t.value}</span>
                <span className="ky-shrink-0 ky-tabular-nums ky-text-[10px] ky-text-muted-foreground">{t.pct}%</span>
                <span className="ky-h-1 ky-w-8 ky-shrink-0 ky-overflow-hidden ky-rounded ky-bg-muted">
                  <span className="ky-block ky-h-full ky-bg-primary/60" style={{ width: `${t.pct}%` }} />
                </span>
              </button>
            ))
          )}
        </div>
      )}
    </li>
  );
}

function SearchResults(props: {
  hits: HybridSearchHit[];
  sourcesSearched: number;
  elapsedMs: number;
  ran: boolean;
  isRunning: boolean;
  error: unknown;
  cols: string[];
  onToggleCol: (field: string) => void;
  onAddFilter: (field: string, value: string) => void;
  onZoom: (from: string, to: string) => void;
  onOpenRow: (source: string, row: Record<string, unknown>) => void;
  onViewInGraph?: (nodeId: string, db: string) => void;
}) {
  const { hits, cols, isRunning, error, ran } = props;
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const fields = useMemo(() => aggregateFields(hits), [hits]);
  const timeField = useMemo(() => detectTimeField(hits), [hits]);
  const rows = useMemo(() => hits.map((h) => h.row), [hits]);

  if (isRunning) return <Centered>Searching…</Centered>;
  if (error != null) {
    return (
      <div
        role="alert"
        className="ky-m-3 ky-rounded ky-border ky-border-destructive/50 ky-bg-destructive/10 ky-p-3 ky-text-xs ky-text-destructive"
      >
        <span className="ky-font-semibold">Search failed:</span> {formatErr(error)}
      </div>
    );
  }
  if (ran && hits.length === 0) {
    return <Centered>No matches. Try different keywords or widen the time range.</Centered>;
  }

  const toggleExpand = (i: number) =>
    setExpanded((s) => {
      const n = new Set(s);
      n.has(i) ? n.delete(i) : n.add(i);
      return n;
    });

  return (
    <div className="ky-flex ky-min-h-0 ky-flex-1">
      {/* Fields rail */}
      <aside className="ky-flex ky-w-56 ky-shrink-0 ky-flex-col ky-overflow-hidden ky-border-r">
        <div className="ky-border-b ky-px-2 ky-py-1.5 ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground">
          Fields · {fields.length}
        </div>
        <ul className="ky-min-h-0 ky-flex-1 ky-overflow-auto">
          {fields.map((f) => (
            <FieldRow
              key={f.name}
              name={f.name}
              kind={f.kind}
              selected={cols.includes(f.name)}
              hits={hits}
              onToggleCol={() => props.onToggleCol(f.name)}
              onAddFilter={props.onAddFilter}
            />
          ))}
        </ul>
      </aside>

      {/* Histogram + document table */}
      <main className="ky-flex ky-min-w-0 ky-flex-1 ky-flex-col ky-min-h-0">
        <div className="ky-flex ky-items-center ky-gap-3 ky-border-b ky-px-3 ky-py-1 ky-text-[11px] ky-text-muted-foreground">
          <span className="ky-font-semibold ky-text-foreground">{hits.length.toLocaleString()}</span> hits
          <span>·</span>
          <span>{props.sourcesSearched} sources</span>
          <span className="ky-ml-auto ky-tabular-nums">{props.elapsedMs} ms</span>
        </div>
        {timeField && (
          <div className="ky-shrink-0 ky-border-b">
            <HistogramTimeline rows={rows} timeCol={timeField} onBucketClick={(from, to) => props.onZoom(from.toISOString(), to.toISOString())} />
          </div>
        )}
        <div className="ky-min-h-0 ky-flex-1 ky-overflow-auto ky-text-xs">
          <table className="ky-w-full ky-border-collapse">
            <thead className="ky-sticky ky-top-0 ky-z-10 ky-bg-background">
              <tr className="ky-border-b ky-text-left ky-text-[10px] ky-uppercase ky-tracking-wider ky-text-muted-foreground">
                <th className="ky-w-6" />
                {timeField && <th className="ky-px-2 ky-py-1 ky-font-medium">{timeField}</th>}
                {cols.length === 0 ? (
                  <th className="ky-px-2 ky-py-1 ky-font-medium">Document</th>
                ) : (
                  cols.map((c) => (
                    <th key={c} className="ky-px-2 ky-py-1 ky-font-mono ky-font-medium ky-normal-case ky-tracking-normal">
                      {c}
                    </th>
                  ))
                )}
                <th className="ky-px-2 ky-py-1 ky-font-medium">Source</th>
                <th className="ky-px-2 ky-py-1 ky-text-right ky-font-medium">_score</th>
              </tr>
            </thead>
            <tbody>
              {hits.map((h, i) => {
                const isOpen = expanded.has(i);
                const colSpan = 2 + (timeField ? 1 : 0) + (cols.length === 0 ? 1 : cols.length);
                return (
                  <React.Fragment key={i}>
                    <tr
                      className="ky-border-b ky-border-border/40 hover:ky-bg-accent/30 ky-cursor-pointer ky-align-top"
                      onClick={() => toggleExpand(i)}
                    >
                      <td className="ky-py-1 ky-pl-2 ky-text-muted-foreground">{isOpen ? "▾" : "▸"}</td>
                      {timeField && (
                        <td className="ky-whitespace-nowrap ky-px-2 ky-py-1 ky-font-mono ky-text-[11px] ky-text-muted-foreground">
                          {formatCell(h.row[timeField])}
                        </td>
                      )}
                      {cols.length === 0 ? (
                        <td className="ky-px-2 ky-py-1">
                          <div className="ky-flex ky-flex-wrap ky-gap-x-3 ky-gap-y-0.5 ky-font-mono ky-text-[11px]">
                            {Object.entries(h.row)
                              .filter(([k]) => !HIDDEN_FIELD(k) && k !== timeField)
                              .slice(0, 8)
                              .map(([k, v]) => (
                                <span key={k} className="ky-truncate">
                                  <span className="ky-text-muted-foreground">{k}:</span>{" "}
                                  <span>{formatCell(v).slice(0, 120)}</span>
                                </span>
                              ))}
                          </div>
                        </td>
                      ) : (
                        cols.map((c) => (
                          <td key={c} className="ky-max-w-[28ch] ky-truncate ky-px-2 ky-py-1 ky-font-mono ky-text-[11px]">
                            {formatCell(h.row[c])}
                          </td>
                        ))
                      )}
                      <td className="ky-px-2 ky-py-1">
                        <SourceBadge source={h.source} />
                      </td>
                      <td className="ky-px-2 ky-py-1 ky-text-right ky-tabular-nums ky-text-[10px] ky-text-muted-foreground">
                        {h.score.toFixed(3)}
                      </td>
                    </tr>
                    {isOpen && (
                      <tr className="ky-border-b ky-border-border/40 ky-bg-muted/20">
                        <td />
                        <td colSpan={colSpan} className="ky-px-2 ky-py-2">
                          <div className="ky-mb-1 ky-flex ky-items-center ky-gap-2">
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                props.onOpenRow(h.source, h.row);
                              }}
                              className="ky-rounded ky-border ky-px-1.5 ky-py-0.5 ky-text-[10px] ky-text-muted-foreground hover:ky-text-foreground"
                            >
                              Open detail
                            </button>
                            {(() => {
                              const g = graphEntityRef(h.source, h.row);
                              if (!g || !props.onViewInGraph) return null;
                              return (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    props.onViewInGraph!(g.id, g.db);
                                  }}
                                  className="ky-inline-flex ky-items-center ky-gap-1 ky-rounded ky-border ky-border-primary/40 ky-px-1.5 ky-py-0.5 ky-text-[10px] ky-text-primary hover:ky-bg-primary/10"
                                  title="Open this entity in the graph"
                                >
                                  <Network className="ky-h-3 ky-w-3" /> View in graph
                                </button>
                              );
                            })()}
                          </div>
                          <dl className="ky-grid ky-grid-cols-[minmax(8rem,12rem)_1fr] ky-gap-x-3 ky-gap-y-0.5 ky-font-mono ky-text-[11px]">
                            {Object.entries(h.row)
                              .filter(([k]) => !HIDDEN_FIELD(k))
                              .map(([k, v]) => (
                                <React.Fragment key={k}>
                                  <dt className="ky-truncate ky-text-muted-foreground">{k}</dt>
                                  <dd className="ky-whitespace-pre-wrap ky-break-words">{formatCell(v)}</dd>
                                </React.Fragment>
                              ))}
                          </dl>
                        </td>
                      </tr>
                    )}
                  </React.Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      </main>
    </div>
  );
}

// ── Query (KQL/SQL) body ──────────────────────────────────────────────────────

function QueryBody(props: {
  schema?: SchemaDoc;
  columns: Column[];
  rows: Record<string, unknown>[];
  isRunning: boolean;
  error: unknown;
  rowFilter: string;
  onRowFilter: (v: string) => void;
  onInsert: (t: string) => void;
  onReplaceAndRun: (kql: string) => void;
  onOpenRow: (row: Record<string, unknown>) => void;
}) {
  const { schema, columns, rows, isRunning, error, rowFilter } = props;
  const hasResults = columns.length > 0 && rows.length > 0;
  const chartSpec = hasResults ? autoChartAxes(columns) : null;
  const isPlottable = chartSpec !== null && chartSpec.type !== "none";
  const timeColName = useMemo(() => columns.find((c) => c.kind === "time")?.name ?? null, [columns]);

  return (
    <div className="ky-flex ky-min-h-0 ky-flex-1">
      <aside className="ky-flex ky-w-60 ky-shrink-0 ky-flex-col ky-overflow-hidden ky-border-r">
        <div className="ky-min-h-0 ky-flex-1 ky-overflow-auto">
          {hasResults ? (
            <FieldStats rows={rows} columns={columns} onAddFilter={(col, value) => props.onInsert(`| where ${col} == "${value}"`)} />
          ) : (
            <SchemaBrowser schema={schema} onInsert={props.onInsert} onReplaceAndRun={(kql) => props.onReplaceAndRun(kql)} />
          )}
        </div>
      </aside>
      <main className="ky-flex ky-min-w-0 ky-flex-1 ky-flex-col ky-min-h-0">
        {error != null && !isRunning && (
          <div
            role="alert"
            className="ky-m-3 ky-rounded ky-border ky-border-destructive/50 ky-bg-destructive/10 ky-p-3 ky-text-xs ky-text-destructive"
          >
            <span className="ky-font-semibold">Query failed:</span> {formatErr(error)}
          </div>
        )}
        {hasResults && timeColName && <HistogramTimeline rows={rows} timeCol={timeColName} />}
        {isRunning && <Centered>Running…</Centered>}
        {!isRunning && error == null && !hasResults && (
          <Centered>{columns.length === 0 ? "Run a query to see results." : "0 rows returned."}</Centered>
        )}
        {hasResults && (
          <div className="ky-flex ky-min-h-0 ky-flex-1 ky-flex-col">
            <RowFilter
              value={rowFilter}
              onChange={props.onRowFilter}
              totalRows={rows.length}
              filteredRows={
                rowFilter.trim()
                  ? rows.filter((row) =>
                      columns.some((c) => {
                        const v = row[c.name];
                        return v != null && String(v).toLowerCase().includes(rowFilter.trim().toLowerCase());
                      }),
                    ).length
                  : rows.length
              }
            />
            <div className="ky-min-h-0 ky-flex-1 ky-overflow-hidden">
              <ResultsGrid columns={columns} rows={rows} filter={rowFilter} />
            </div>
            {isPlottable && (
              <div className="ky-h-44 ky-shrink-0 ky-border-t">
                <ChartPanel columns={columns} rows={rows} />
              </div>
            )}
          </div>
        )}
      </main>
    </div>
  );
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-6 ky-text-xs ky-text-muted-foreground">
      {children}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-8 ky-text-center ky-text-sm ky-text-muted-foreground">
      <div className="ky-space-y-1">
        <div>Search your data or run a query.</div>
        <div className="ky-text-xs ky-opacity-70">
          Type keywords for instant hybrid search, or a table name / KQL / SQL to query. Press ⌘↵ to run.
        </div>
      </div>
    </div>
  );
}

/** If a hit is a graph entity (a `*_nodes` / file-candidate row with an id),
 *  return its `{db, id}` so the caller can deep-link to the graph. */
function graphEntityRef(source: string, row: Record<string, unknown>): { db: string; id: string } | null {
  const dot = source.indexOf(".");
  const db = dot >= 0 ? source.slice(0, dot) : source;
  const table = dot >= 0 ? source.slice(dot + 1) : source;
  const isNodeTable = table.endsWith("_nodes") || table === "memory_nodes" || table === "file_candidates";
  const id = row["id"];
  if (isNodeTable && typeof id === "string" && id) return { db, id };
  return null;
}

function formatErr(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

export function KymaExplore({ fallback, database, ...rest }: KymaExploreProps): JSX.Element {
  const ctx = useKymaContext();
  const scopedCtx = database ? { ...ctx, client: ctx.client.withDatabase(database) } : ctx;
  return (
    <KymaContext.Provider value={scopedCtx}>
      <KymaErrorBoundary fallback={fallback}>
        <KymaExploreInner database={database} {...rest} />
      </KymaErrorBoundary>
    </KymaContext.Provider>
  );
}
