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

import React, { useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square, Search as SearchIcon, Code2 } from "lucide-react";
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
  onQueryChange?: (q: string) => void;
}

function KymaExploreInner({
  defaultQuery = "",
  timeRange: timeRangeProp,
  database = "*",
  className,
  style,
  onQueryChange,
}: Omit<KymaExploreProps, "fallback">) {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;

  const [input, setInput] = useState(defaultQuery);
  const [forcedMode, setForcedMode] = useState<ExploreMode | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>(timeRangeProp ?? { preset: "none" });
  const [rowFilter, setRowFilter] = useState("");
  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);
  const [submittedMode, setSubmittedMode] = useState<ExploreMode | null>(null);

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
          isRunning={search.isRunning}
          error={search.error}
          onOpenRow={(source, row) => setOpenRow({ source, row })}
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

// ── Keyword (hybrid search) body: ranked hits ────────────────────────────────

function SearchResults(props: {
  hits: HybridSearchHit[];
  isRunning: boolean;
  error: unknown;
  onOpenRow: (source: string, row: Record<string, unknown>) => void;
}) {
  const { hits, isRunning, error } = props;
  if (isRunning) {
    return <Centered>Searching…</Centered>;
  }
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
  if (hits.length === 0) {
    return <Centered>No matches. Try different keywords or widen the time range.</Centered>;
  }
  return (
    <div className="ky-min-h-0 ky-flex-1 ky-overflow-auto">
      <ul className="ky-divide-y">
        {hits.map((h, i) => (
          <li key={i}>
            <button
              type="button"
              onClick={() => props.onOpenRow(h.source, h.row)}
              className="ky-flex ky-w-full ky-items-start ky-gap-3 ky-px-4 ky-py-2 ky-text-left hover:ky-bg-accent/40"
            >
              <span className="ky-mt-0.5 ky-shrink-0 ky-rounded ky-bg-muted ky-px-1.5 ky-py-0.5 ky-font-mono ky-text-[10px] ky-text-muted-foreground">
                {h.source}
              </span>
              <span className="ky-min-w-0 ky-flex-1 ky-truncate ky-text-xs">{rowPreview(h.row)}</span>
              <span className="ky-shrink-0 ky-tabular-nums ky-text-[10px] ky-text-muted-foreground">
                {h.score.toFixed(3)}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Short human-readable preview of a result row (prefers titley/texty fields). */
function rowPreview(row: Record<string, unknown>): string {
  const prefer = ["title", "content_preview", "content", "message", "msg", "text", "name", "label", "kind"];
  for (const k of prefer) {
    const v = row[k];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  // Fall back to the first few non-empty string fields.
  const parts: string[] = [];
  for (const [k, v] of Object.entries(row)) {
    if (k.startsWith("__")) continue;
    if (typeof v === "string" && v.trim()) parts.push(`${k}=${v.trim()}`);
    if (parts.length >= 3) break;
  }
  return parts.join("  ·  ") || JSON.stringify(row).slice(0, 160);
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
