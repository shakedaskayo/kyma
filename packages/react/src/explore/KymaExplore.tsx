/**
 * KymaExplore — the unified data-exploration surface.
 *
 * One smart input that auto-detects keyword search (Discover grammar) vs raw
 * KQL/SQL and routes to the matching engine, sharing a single shell: scope +
 * time-range + Run, a histogram, a left rail, a results area, and a row-detail
 * drawer. Replaces the separate Discover and Query Editor pages.
 *
 *   keyword → useDiscoverSearch (/v1/explore/search): sources/fields rail,
 *             per-source histogram, stream/table, row drawer. Cross-DB by design.
 *   kql/sql → useKymaQuery (/v1/query): schema browser → field-stats rail,
 *             timestamp histogram, results grid, chart, row drawer.
 *
 * The detected mode is shown as a badge the user can click to force a mode, so
 * the auto-detection is never a mystery.
 */

import React, { useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square, Search as SearchIcon, Code2 } from "lucide-react";
import { autoChartAxes } from "@kyma-ai/client";
import type { Column, SchemaDoc } from "@kyma-ai/client";

import { KymaErrorBoundary } from "../internal/KymaErrorBoundary";
import { Button } from "../internal/ui/button";
import { KymaContext, useKymaClient, useKymaContext } from "../provider/context";
import { cn } from "../internal/cn";

import { useKymaQuery } from "../hooks/useKymaQuery";
import { useDiscoverSearch, resolveTimeRange } from "../discover/useDiscoverSearch";
import { mergeSources } from "../discover/stream";
import { Histogram } from "../discover/Histogram";
import { StreamView } from "../discover/StreamView";
import { SourcesRail } from "../discover/SourcesRail";
import { FieldsRail } from "../discover/FieldsRail";
import { SourceTableView } from "../discover/SourceTableView";
import { RowDetailDrawer } from "../discover/RowDetailDrawer";
import { SummaryLine } from "../discover/SummaryLine";
import { serializePills } from "../discover/discoverGrammar";
import type { Pill, Scope, SourceKey } from "../discover/types";

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
   * Database scope. "*" (default) unifies across all databases; a concrete name
   * filters. Drives `x-database` for KQL/SQL and the source glob for keyword search.
   */
  database?: string;
  className?: string;
  style?: React.CSSProperties;
  fallback?: React.ReactNode;
  /** Called whenever the input text changes. */
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
  const baseClient = useKymaClient();
  const client = useMemo(
    () => (database ? baseClient.withDatabase(database) : baseClient),
    [baseClient, database],
  );
  const endpoint = client.transport.endpoint;

  // ── Input + run state ───────────────────────────────────────────────────────
  const [input, setInput] = useState(defaultQuery);
  const [forcedMode, setForcedMode] = useState<ExploreMode | null>(null);
  const [timeRange, setTimeRange] = useState<TimeRange>(timeRangeProp ?? { preset: "none" });
  const [rowFilter, setRowFilter] = useState("");
  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);

  // The submitted query + the mode it was run as (drives which engine renders).
  const [submitted, setSubmitted] = useState<{ text: string; mode: ExploreMode } | null>(null);

  // ── Schema (table names for mode detection + the schema browser) ─────────────
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

  // ── Discover (keyword) engine ────────────────────────────────────────────────
  const discoverScope: Scope = useMemo(
    () => (database && database !== "*" ? { kind: "sources", sources: [`${database}.*`] } : { kind: "all" }),
    [database],
  );
  const searchEnabled = submitted?.mode === "search";
  const { results, rerun, cancel: cancelDiscover } = useDiscoverSearch({
    search: searchEnabled ? submitted!.text : "",
    scope: discoverScope,
    timeRange,
    enabled: searchEnabled,
  });
  const [visibleSources, setVisibleSources] = useState<SourceKey[] | null>(null);
  const [selectedSource, setSelectedSource] = useState<SourceKey | null>(null);
  const [columns, setColumns] = useState<string[]>([]);
  const [viewMode, setViewMode] = useState<"stream" | { table: SourceKey }>("stream");

  // ── Query (KQL/SQL) engine ────────────────────────────────────────────────────
  const { columns: qCols, rows: qRows, isRunning: qRunning, error: qError, execute, cancel: cancelQuery } =
    useKymaQuery();

  // ── Run / cancel ──────────────────────────────────────────────────────────────
  const runInputRef = useRef(input);
  runInputRef.current = input;

  const run = () => {
    const text = runInputRef.current;
    const mode = forcedMode ?? detectMode(text, tableNames);
    setRowFilter("");
    setOpenRow(null);
    if (mode === "search") {
      // If the same search is re-submitted, force a re-run.
      if (submitted?.mode === "search" && submitted.text === text) rerun();
      setSubmitted({ text, mode });
    } else {
      setSubmitted({ text, mode });
      const effective = mode === "kql" ? prependTimeFilter(text, timeRange, schema) : text;
      void execute({ database, query: effective, language: mode }).catch(() => {});
    }
  };

  const cancel = () => {
    cancelDiscover();
    cancelQuery();
  };

  const setInputAndNotify = (v: string) => {
    setInput(v);
    onQueryChange?.(v);
  };
  const appendToInput = (text: string) => {
    setInputAndNotify(input.trim() ? `${input.trim()} ${text}` : text);
  };

  const handleOpenRow = (source: string, row: Record<string, unknown>) =>
    setOpenRow({ source, row });

  const running = results.status === "running" || qRunning;
  const showQuery = submitted?.mode === "kql" || submitted?.mode === "sql";

  // ── Render ────────────────────────────────────────────────────────────────────
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
          title="Detected mode — click to force"
          onClick={() => setForcedMode(cycleMode(liveMode, forcedMode))}
          className={cn(
            "ky-mt-0.5 ky-flex ky-shrink-0 ky-items-center ky-gap-1 ky-rounded ky-border ky-px-2 ky-py-1 ky-text-[11px] ky-font-medium",
            liveMode === "search"
              ? "ky-text-muted-foreground"
              : "ky-border-primary/40 ky-text-primary",
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
      {!submitted ? (
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
            setSubmitted({ text: kql, mode: "kql" });
            void execute({ database, query: prependTimeFilter(kql, timeRange, schema), language: "kql" }).catch(() => {});
          }}
          onOpenRow={(row) => handleOpenRow(String(row["__database"] ?? "result"), row)}
        />
      ) : (
        <DiscoverBody
          results={results}
          timeRange={timeRange}
          visibleSources={visibleSources}
          selectedSource={selectedSource}
          columns={columns}
          viewMode={viewMode}
          onToggleVisible={(s) =>
            setVisibleSources((cur) => toggleIn(cur, s, Array.from(results.sources.keys())))
          }
          onSelectSource={setSelectedSource}
          onToggleColumn={(f) => setColumns((c) => (c.includes(f) ? c.filter((x) => x !== f) : [...c, f]))}
          onInsertFilter={appendToInput}
          onOpenTable={(s) => setViewMode({ table: s })}
          onBackToStream={() => setViewMode("stream")}
          onZoom={(from, to) => setTimeRange({ preset: "custom", from, to })}
          onOpenRow={handleOpenRow}
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

// ── Discover (keyword) body ───────────────────────────────────────────────────

const PRESET_LABEL: Record<string, string> = {
  none: "all time", "5m": "last 5m", "15m": "last 15m", "1h": "last 1h", "6h": "last 6h",
  "24h": "last 24h", "7d": "last 7d", "30d": "last 30d", custom: "custom range",
};

function DiscoverBody(props: {
  results: ReturnType<typeof useDiscoverSearch>["results"];
  timeRange: TimeRange;
  visibleSources: SourceKey[] | null;
  selectedSource: SourceKey | null;
  columns: string[];
  viewMode: "stream" | { table: SourceKey };
  onToggleVisible: (s: SourceKey) => void;
  onSelectSource: (s: SourceKey) => void;
  onToggleColumn: (f: string) => void;
  onInsertFilter: (t: string) => void;
  onOpenTable: (s: SourceKey) => void;
  onBackToStream: () => void;
  onZoom: (from: string, to: string) => void;
  onOpenRow: (source: string, row: Record<string, unknown>) => void;
}) {
  const { results, timeRange, visibleSources, selectedSource, columns, viewMode } = props;
  const selected = selectedSource ? results.sources.get(selectedSource) ?? null : null;
  const streamRows = mergeSources(Array.from(results.sources.values()), visibleSources);
  const tableSrc = typeof viewMode === "object" ? results.sources.get(viewMode.table) ?? null : null;

  return (
    <>
      <SummaryLine
        sourcesSearched={results.sources.size}
        windowLabel={PRESET_LABEL[timeRange.preset] ?? "all time"}
        eventCount={streamRows.length}
        finishedAt={results.finishedAt ?? null}
        status={results.status}
      />
      <div className="ky-flex ky-min-h-0 ky-flex-1">
        <aside className="ky-w-60 ky-shrink-0 ky-overflow-auto ky-border-r">
          <SourcesRail
            results={results}
            visible={visibleSources}
            onToggleVisible={props.onToggleVisible}
            selected={selectedSource}
            onSelect={props.onSelectSource}
            onOpenTable={props.onOpenTable}
          />
          <FieldsRail
            source={selected}
            columns={columns}
            onToggleColumn={props.onToggleColumn}
            onInsertFilter={props.onInsertFilter}
          />
        </aside>
        <main className="ky-flex ky-min-w-0 ky-flex-1 ky-flex-col ky-min-h-0">
          {tableSrc ? (
            <div className="ky-min-h-0 ky-flex-1 ky-overflow-auto">
              <SourceTableView
                src={tableSrc}
                onBack={props.onBackToStream}
                onOpenRow={(row) => props.onOpenRow(tableSrc.source, row)}
              />
            </div>
          ) : (
            <>
              <Histogram
                results={results}
                rangeTo={resolveTimeRange(timeRange)?.to ?? null}
                onZoom={props.onZoom}
              />
              {results.topError && (
                <div
                  role="alert"
                  className="ky-m-3 ky-rounded ky-border ky-border-destructive/50 ky-bg-destructive/10 ky-p-3 ky-text-xs ky-text-destructive"
                >
                  <span className="ky-font-semibold">{results.topError.code}</span>: {results.topError.message}
                </div>
              )}
              {results.status === "done" && results.sources.size === 0 ? (
                <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-6 ky-text-sm ky-text-muted-foreground">
                  No data sources match this scope.
                </div>
              ) : (
                <StreamView
                  rows={streamRows}
                  sources={results.sources}
                  columns={columns}
                  onOpenRow={props.onOpenRow}
                />
              )}
            </>
          )}
        </main>
      </div>
    </>
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
  // Result columns carry an inferred ColKind; "time" is the timestamp column.
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
        {hasResults && timeColName && (
          <HistogramTimeline rows={rows} timeCol={timeColName} />
        )}
        {isRunning && (
          <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-6 ky-text-xs ky-text-muted-foreground">
            Running…
          </div>
        )}
        {!isRunning && error == null && !hasResults && (
          <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-6 ky-text-xs ky-text-muted-foreground">
            {columns.length === 0 ? "Run a query to see results." : "0 rows returned."}
          </div>
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

function EmptyState() {
  return (
    <div className="ky-flex ky-flex-1 ky-items-center ky-justify-center ky-p-8 ky-text-center ky-text-sm ky-text-muted-foreground">
      <div className="ky-space-y-1">
        <div>Search your data or run a query.</div>
        <div className="ky-text-xs ky-opacity-70">
          Type keywords to search, or a table name / KQL / SQL to query. Press ⌘↵ to run.
        </div>
      </div>
    </div>
  );
}

// ── helpers ─────────────────────────────────────────────────────────────────

function cycleMode(live: ExploreMode, forced: ExploreMode | null): ExploreMode | null {
  // Clicking the badge forces the *other* primary mode; clicking again clears
  // the override (back to auto).
  if (forced) return null;
  return live === "search" ? "kql" : "search";
}

function toggleIn(cur: SourceKey[] | null, s: SourceKey, all: SourceKey[]): SourceKey[] {
  const base = cur ?? all;
  return base.includes(s) ? base.filter((x) => x !== s) : [...base, s];
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

// ── Public wrapper ─────────────────────────────────────────────────────────────

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
