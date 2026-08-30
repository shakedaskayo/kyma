/**
 * PensieveQueryEditor — self-contained embedded query surface.
 *
 * Composes KqlEditor/Monaco SQL, SchemaBrowser, ResultsGrid, ChartPanel, and
 * TimeRangePicker into a single embeddable component backed by usePensieveQuery.
 *
 * Schema is loaded via React Query (key ["pensieve", endpoint, database, "schema"])
 * and passed to the editor for completions and to SchemaBrowser for browsing.
 *
 * Time-range injection: when language="kql" and showTimeRange=true, the time
 * filter is spliced into the query immediately before execution using
 * prependTimeFilter() from time-range.ts — the editor's display text is never
 * mutated (the user always sees the raw query).
 *
 * SQL mode: uses Monaco's built-in "sql" language and sends content-type
 * application/sql. Time-range injection is skipped in SQL mode (no standard
 * syntax; the user writes their own WHERE clause).
 *
 * Omitted props (wired to nothing, not surfaced):
 *   - knownValues: not computed from the embedded results path (would require
 *     post-execute column scan across potentially many rows — omitted to keep
 *     the component simple; KqlEditor's schema completions still work).
 */

import React, { useState, useCallback, useEffect, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square } from "lucide-react";
import { autoChartAxes } from "@pensieve-ai/client";
import type { Column, SchemaDoc } from "@pensieve-ai/client";

import { PensieveErrorBoundary } from "../internal/PensieveErrorBoundary";
import { Button } from "../internal/ui/button";
import { usePensieveClient, usePensieveContext } from "../provider/context";
import { usePensieveQuery } from "../hooks/usePensieveQuery";
import { KqlEditor } from "./editor/KqlEditor";
import { SchemaBrowser } from "./schema/SchemaBrowser";
import { ResultsGrid } from "./results/ResultsGrid";
import { RowFilter } from "./results/RowFilter";
import { ChartPanel } from "./chart/ChartPanel";
import { TimeRangePicker } from "./time-range/TimeRangePicker";
import { prependTimeFilter } from "./time-range/time-range";
import type { TimeRange } from "./time-range/time-range-types";

// ── Public types ──────────────────────────────────────────────────────────────

export type { TimeRange };

export interface PensieveQueryEditorProps {
  /** Query language. Default "kql". SQL mode skips KQL niceties + time injection. */
  language?: "kql" | "sql";
  /** Initial query text (uncontrolled — prop changes after mount are ignored). */
  defaultQuery?: string;
  /** Show the schema browser panel. Default true. */
  showSchemaBrowser?: boolean;
  /** Show the results grid below the editor. Default true. */
  showResults?: boolean;
  /** Show the chart panel below results when the result shape is plottable. Default true. */
  showChart?: boolean;
  /** Show the TimeRangePicker in the toolbar. Default true. */
  showTimeRange?: boolean;
  /**
   * Initial/re-applies time range when the prop changes.
   * Only applies in kql mode. Default preset "1h".
   */
  timeRange?: TimeRange;
  /** When true: editor is read-only and the Run button is hidden. Default false. */
  readOnly?: boolean;
  /**
   * Override the database used for execution and schema loading.
   * Falls back to the endpoint's default database when omitted.
   */
  database?: string;
  /** Optional className for the outermost container div. */
  className?: string;
  /** Optional inline styles for the outermost container div. */
  style?: React.CSSProperties;
  /** Override the error boundary fallback. */
  fallback?: React.ReactNode;
  /** Called after each successful execute with the resulting columns + rows. */
  onResults?: (r: { columns: Column[]; rows: Record<string, unknown>[] }) => void;
  /** Called whenever the query text changes. */
  onQueryChange?: (q: string) => void;
}

/** Best-effort human-readable message from an unknown query error. */
function formatQueryError(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === "string") return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

// ── Inner implementation (wrapped by error boundary below) ────────────────────

function PensieveQueryEditorInner({
  language = "kql",
  defaultQuery = "",
  showSchemaBrowser = true,
  showResults = true,
  showChart = true,
  showTimeRange = true,
  timeRange: timeRangeProp,
  readOnly = false,
  database: databaseProp,
  className,
  style,
  onResults,
  onQueryChange,
}: Omit<PensieveQueryEditorProps, "fallback">) {
  const client = usePensieveClient();
  const { } = usePensieveContext(); // asserts provider is present

  // Use the scoped client when a database override is provided.
  const scopedClient = databaseProp ? client.withDatabase(databaseProp) : client;
  const endpoint = scopedClient.transport.endpoint;
  // Effective database: the prop override, or the transport's own default (may
  // be undefined if never set — we fall back to empty string for KqlEditor which
  // needs a string).
  const effectiveDatabase: string = databaseProp ?? (scopedClient.transport.database ?? "");

  // ── Query state ───────────────────────────────────────────────────────────────

  const [query, setQuery] = useState(defaultQuery);
  const [timeRange, setTimeRange] = useState<TimeRange>(
    timeRangeProp ?? { preset: "1h" },
  );
  const [rowFilter, setRowFilter] = useState("");

  // Sync time range prop changes (controlled-ish).
  const prevTimeRangePropRef = useRef(timeRangeProp);
  useEffect(() => {
    if (timeRangeProp && timeRangeProp !== prevTimeRangePropRef.current) {
      setTimeRange(timeRangeProp);
      prevTimeRangePropRef.current = timeRangeProp;
    }
  }, [timeRangeProp]);

  // ── Schema ────────────────────────────────────────────────────────────────────

  const { data: schema } = useQuery<SchemaDoc>({
    queryKey: ["pensieve", endpoint, effectiveDatabase, "schema"],
    queryFn: () => scopedClient.catalog.fetchSchema(),
    staleTime: 5 * 60_000,
    enabled: true,
  });

  // ── Execution ─────────────────────────────────────────────────────────────────

  const { columns, rows, isRunning, error, execute, cancel } = usePensieveQuery();

  // Track whether we have had a successful run
  const hasResults = columns.length > 0 && rows.length > 0;

  // Check chart plottability
  const chartSpec = hasResults ? autoChartAxes(columns) : null;
  const isPlottable = chartSpec !== null && chartSpec.type !== "none";

  const handleRun = useCallback(async () => {
    if (readOnly || isRunning) return;
    let effectiveQuery = query;
    // Inject time filter for KQL only — schema-aware so it targets the table's
    // real time column (and injects nothing when there isn't one).
    if (language === "kql" && showTimeRange) {
      effectiveQuery = prependTimeFilter(query, timeRange, schema);
    }
    try {
      await execute({
        database: effectiveDatabase,
        query: effectiveQuery,
        language,
      });
    } catch {
      // errors are surfaced via the hook's `.error` field; re-throw is swallowed
    }
  }, [readOnly, isRunning, query, language, showTimeRange, timeRange, schema, execute, effectiveDatabase]);

  const handleQueryChange = useCallback(
    (v: string) => {
      setQuery(v);
      onQueryChange?.(v);
    },
    [onQueryChange],
  );

  const handleInsert = useCallback(
    (text: string) => {
      setQuery((prev) => {
        const sep = prev.endsWith("\n") || !prev ? "" : " ";
        return `${prev}${sep}${text}`;
      });
    },
    [],
  );

  const handleReplaceAndRun = useCallback(
    (kql: string) => {
      setQuery(kql);
      onQueryChange?.(kql);
      // Execute immediately (bypass the read-only guard since this is
      // an internal quick action — the browser panel's SchemaBrowser
      // only renders when showSchemaBrowser=true; readOnly hides that path)
      void execute({
        database: effectiveDatabase,
        query: kql,
        language: "kql",
      });
    },
    [execute, effectiveDatabase, onQueryChange],
  );

  // Fire onResults after each successful run
  const prevColumnsRef = useRef(columns);
  const prevRowsRef = useRef(rows);
  useEffect(() => {
    if (
      !isRunning &&
      (columns !== prevColumnsRef.current || rows !== prevRowsRef.current) &&
      columns.length > 0
    ) {
      onResults?.({ columns, rows });
      prevColumnsRef.current = columns;
      prevRowsRef.current = rows;
    }
  }, [isRunning, columns, rows, onResults]);

  // ── Layout ────────────────────────────────────────────────────────────────────

  return (
    <div
      className={`ky-pensieve-query-editor ky-flex ky-h-full ky-flex-col ky-overflow-hidden ky-bg-background ky-text-foreground ${className ?? ""}`}
      style={style}
    >
      {/* ── Toolbar ── */}
      <div className="ky-flex ky-shrink-0 ky-items-center ky-gap-2 ky-border-b ky-bg-background ky-px-3 ky-py-1.5">
        {showTimeRange && language === "kql" && (
          <TimeRangePicker value={timeRange} onChange={setTimeRange} />
        )}
        {!readOnly && (
          isRunning ? (
            <Button size="sm" variant="destructive" onClick={cancel} data-testid="cancel-btn">
              <Square className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Cancel
            </Button>
          ) : (
            <Button size="sm" onClick={handleRun} data-testid="run-btn">
              <Play className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Run
              <kbd className="ky-ml-2 ky-rounded ky-bg-primary-foreground/20 ky-px-1 ky-py-0.5 ky-text-[10px] ky-text-primary-foreground/80">
                ⌘↵
              </kbd>
            </Button>
          )
        )}
      </div>

      {/* ── Main row: schema browser + editor ── */}
      <div className="ky-flex ky-min-h-0 ky-flex-1 ky-overflow-hidden" style={{ maxHeight: "40%" }}>
        {showSchemaBrowser && (
          <aside className="ky-w-56 ky-shrink-0 ky-overflow-hidden ky-border-r ky-bg-background">
            <SchemaBrowser
              schema={schema}
              onInsert={handleInsert}
              onReplaceAndRun={readOnly ? undefined : handleReplaceAndRun}
            />
          </aside>
        )}
        <div className="ky-min-w-0 ky-flex-1 ky-overflow-hidden">
          {language === "kql" ? (
            <KqlEditor
              value={query}
              onChange={handleQueryChange}
              onRun={handleRun}
              schema={schema}
              database={effectiveDatabase}
            />
          ) : (
            <SqlEditor value={query} onChange={handleQueryChange} onRun={handleRun} readOnly={readOnly} />
          )}
        </div>
      </div>

      {/* ── Results area ── */}
      {showResults && (
        <div className="ky-flex ky-min-h-0 ky-flex-1 ky-flex-col ky-overflow-hidden ky-border-t">
          {error != null && !isRunning && (
            <div
              role="alert"
              className="ky-m-3 ky-rounded ky-border ky-border-destructive/50 ky-bg-destructive/10 ky-p-3 ky-text-xs ky-text-destructive"
            >
              <span className="ky-font-semibold">Query failed:</span>{" "}
              {formatQueryError(error)}
            </div>
          )}
          {error == null && !hasResults && !isRunning && (
            <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-p-6 ky-text-xs ky-text-muted-foreground">
              {columns.length === 0 ? "Run a query to see results." : "0 rows returned."}
            </div>
          )}
          {isRunning && (
            <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-p-6 ky-text-xs ky-text-muted-foreground">
              Streaming results…
            </div>
          )}
          {hasResults && (
            <div className="ky-flex ky-min-h-0 ky-flex-1 ky-flex-col">
              <RowFilter
                value={rowFilter}
                onChange={setRowFilter}
                totalRows={rows.length}
                filteredRows={
                  rowFilter.trim()
                    ? rows.filter((row) =>
                        columns.some((col) => {
                          const v = row[col.name];
                          if (v == null) return false;
                          return String(v).toLowerCase().includes(rowFilter.trim().toLowerCase());
                        }),
                      ).length
                    : rows.length
                }
              />
              <div className="ky-min-h-0 ky-flex-1 ky-overflow-hidden">
                <ResultsGrid columns={columns} rows={rows} filter={rowFilter} />
              </div>
            </div>
          )}
        </div>
      )}

      {/* ── Chart panel ── */}
      {showChart && isPlottable && hasResults && (
        <div className="ky-h-48 ky-shrink-0 ky-border-t">
          <ChartPanel columns={columns} rows={rows} />
        </div>
      )}
    </div>
  );
}

// ── Minimal SQL editor (no Monaco KQL registration, plain sql language) ───────

function SqlEditor({
  value,
  onChange,
  onRun,
  readOnly,
}: {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  readOnly?: boolean;
}) {
  // Dynamic import to avoid bundling @monaco-editor/react at top level when
  // only KQL is needed; in practice the KqlEditor already imports it, so this
  // is the same chunk.
  const [MonacoEditor, setMonacoEditor] = React.useState<React.ComponentType<{
    height: string;
    language: string;
    value: string;
    onChange: (v: string | undefined) => void;
    onMount: (editor: { addCommand: (key: number, fn: () => void) => void }, monaco: { KeyMod: { CtrlCmd: number }; KeyCode: { Enter: number } }) => void;
    options: Record<string, unknown>;
    theme: string;
  }> | null>(null);

  const { isDark } = usePensieveContext();

  useEffect(() => {
    import("@monaco-editor/react").then((mod) => {
      setMonacoEditor(() => mod.default as typeof MonacoEditor);
    });
  }, []);

  const onRunRef = useRef(onRun);
  useEffect(() => { onRunRef.current = onRun; }, [onRun]);

  const handleMount = useCallback((editor: { addCommand: (key: number, fn: () => void) => void }, monaco: { KeyMod: { CtrlCmd: number }; KeyCode: { Enter: number } }) => {
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => onRunRef.current());
  }, []);

  if (!MonacoEditor) {
    return (
      <textarea
        className="ky-h-full ky-w-full ky-resize-none ky-bg-transparent ky-p-3 ky-font-mono ky-text-xs ky-text-foreground focus:ky-outline-none"
        value={value}
        readOnly={readOnly}
        onChange={(e) => onChange(e.target.value)}
        placeholder="SELECT …"
      />
    );
  }

  return (
    <MonacoEditor
      height="100%"
      language="sql"
      theme={isDark ? "vs-dark" : "vs"}
      value={value}
      onChange={(v) => onChange(v ?? "")}
      onMount={handleMount}
      options={{
        minimap: { enabled: false },
        lineNumbers: "on",
        scrollBeyondLastLine: false,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        tabSize: 2,
        automaticLayout: true,
        wordWrap: "on",
        readOnly: readOnly ?? false,
      }}
    />
  );
}

// ── Public wrapper (adds error boundary) ─────────────────────────────────────

export function PensieveQueryEditor({ fallback, ...rest }: PensieveQueryEditorProps): JSX.Element {
  return (
    <PensieveErrorBoundary fallback={fallback}>
      <PensieveQueryEditorInner {...rest} />
    </PensieveErrorBoundary>
  );
}
