import { createFileRoute, useSearch, useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { encodeQueryState, decodeQueryState } from "@/lib/url-state";
import { useQuery } from "@tanstack/react-query";
import { Play, Square, Download, FileJson } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Editor } from "@/features/editor/Editor";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ResultsGrid, exportCsv } from "@/features/results-grid/ResultsGrid";
import { RowFilter } from "@/features/results-grid/RowFilter";
import { ChartPanel } from "@/features/chart/ChartPanel";
import { SchemaBrowser } from "@/features/schema-browser/SchemaBrowser";
import { TabBar } from "@/features/tabs/TabBar";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { HistogramTimeline } from "@/features/histogram/HistogramTimeline";
import { FieldStats } from "@/features/field-stats/FieldStats";
import { SavedQueryChips } from "@/features/saved-queries/SavedQueryChips";
import { SmartSearchBar } from "@/features/search/SmartSearchBar";
import { extractLeadingTable } from "@/features/search/parseSearch";
import type { SearchSchema } from "@/features/search/parseSearch";
import { downloadBlob } from "@/lib/download";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { useWorkspaceShortcuts } from "@/lib/shortcuts";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { useRunQuery, type TabResult } from "@/features/run/useRunQuery";

export const Route = createFileRoute("/_app/explore")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  component: ExplorePage,
});

function ExplorePage() {
  const { endpoint, token, database } = useSession();
  const workspace = useWorkspace();
  const { run, cancel } = useRunQuery();
  const [liveResult, setLiveResult] = useState<TabResult | null>(null);
  const [view, setView] = useState<"grid" | "chart">("grid");
  const [rowFilter, setRowFilter] = useState("");
  const { q } = useSearch({ from: "/_app/explore" });
  const navigate = useNavigate();

  const { data: schema } = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    staleTime: 5 * 60_000,
    enabled: Boolean(endpoint && token),
  });

  // Build knownValues from FieldStats data (top-20 observed values per column)
  const knownValues = useMemo<Record<string, string[]>>(() => {
    if (!liveResult) return {};
    const result: Record<string, string[]> = {};
    const MAX_DISTINCT = 20;
    for (const col of liveResult.columns) {
      if (col.kind === "time") continue;
      const counts = new Map<string, number>();
      for (const row of liveResult.rows) {
        const v = row[col.name];
        if (v === null || v === undefined) continue;
        const s = String(v);
        counts.set(s, (counts.get(s) ?? 0) + 1);
      }
      if (counts.size <= MAX_DISTINCT * 3) {
        result[col.name] = Array.from(counts.entries())
          .sort((a, b) => b[1] - a[1])
          .slice(0, MAX_DISTINCT)
          .map(([v]) => v);
      }
    }
    return result;
  }, [liveResult]);

  // Load from URL on first mount only.
  const bootstrapped = useRef(false);
  useEffect(() => {
    if (bootstrapped.current) return;
    bootstrapped.current = true;
    if (q) {
      const decoded = decodeQueryState(q);
      if (decoded) {
        workspace.newTab({ query: decoded.query, timeRange: { preset: decoded.preset, from: decoded.from, to: decoded.to } });
      } else if (workspace.tabs.length === 0) {
        workspace.newTab({ query: "otel_logs | take 50" });
      }
    } else if (workspace.tabs.length === 0) {
      workspace.newTab({ query: "otel_logs | take 50" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const active = workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0];

  // Build search schema for SmartSearchBar
  const searchSchema = useMemo<SearchSchema | null>(() => {
    if (!schema) return null;
    const db = schema.databases.find((d) => d.name === database);
    if (!db) return null;
    const tables = db.tables.map((t) => t.name);
    const columnsByTable: Record<string, string[]> = {};
    const columnTypes: Record<string, Record<string, string>> = {};
    db.tables.forEach((t) => {
      columnsByTable[t.name] = t.columns.map((c) => c.name);
      columnTypes[t.name] = {};
      t.columns.forEach((c) => { columnTypes[t.name][c.name] = c.type; });
    });
    return { tables, columnsByTable, columnTypes };
  }, [schema, database]);

  // Leading table from active query (for SmartSearchBar)
  const leadingTable = useMemo(
    () => (active ? extractLeadingTable(active.query) : null),
    [active],
  );

  // Sync URL whenever the active tab's query or time range changes.
  useEffect(() => {
    if (!active) return;
    const encoded = encodeQueryState({
      query: active.query,
      preset: active.timeRange.preset,
      from: active.timeRange.from,
      to: active.timeRange.to,
    });
    navigate({ to: "/explore", search: { q: encoded }, replace: true });
  }, [active, navigate]);

  const status = active?.results.kind ?? "idle";

  const runActive = async () => {
    if (!active) return;
    setLiveResult(null);
    await run(active, setLiveResult);
  };

  useWorkspaceShortcuts(runActive);

  // Find the timestamp column in current results
  const timeCol = liveResult?.columns.find((c) => c.kind === "time")?.name;

  const handleAddFilter = (col: string, value: string) => {
    if (!active) return;
    const addition = ` | where ${col} == "${value}"`;
    workspace.setQuery(active.id, active.query + addition);
  };

  const handleBucketClick = (from: Date, to: Date) => {
    if (!active) return;
    const fmt = (d: Date) => d.toISOString();
    const addition = ` | where timestamp >= datetime(${fmt(from)}) and timestamp < datetime(${fmt(to)})`;
    workspace.setQuery(active.id, active.query + addition);
  };

  const handlePickSavedQuery = (query: string) => {
    if (!active) return;
    workspace.setQuery(active.id, query);
  };

  const handleSearch = (kql: string) => {
    if (!active) return;
    workspace.setQuery(active.id, kql);
    // Run the query with the updated query text directly
    setLiveResult(null);
    void run({ ...active, query: kql }, setLiveResult);
  };

  const tabBtn = (on: boolean) =>
    `rounded-md px-2.5 py-1 text-xs transition font-medium ${on ? "bg-primary text-primary-foreground shadow-sm" : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"}`;

  const hasResults = Boolean(liveResult && active?.results.kind === "ok" && liveResult.rows.length > 0);

  return (
    <div className="flex h-full bg-muted/20">
      {/* ── Left rail ── */}
      <aside className="flex w-64 shrink-0 flex-col border-r bg-background">
        <div className="min-h-0 flex-1 overflow-hidden flex flex-col">
          <SchemaBrowser
            schema={schema}
            onInsert={(t) =>
              active && workspace.setQuery(active.id, `${active.query}${active.query.endsWith("\n") || !active.query ? "" : " "}${t}`)
            }
          />
          {liveResult && (
            <>
              <div className="border-t border-muted/40" />
              <div className="min-h-0 flex-1 overflow-hidden">
                <FieldStats
                  rows={liveResult.rows}
                  columns={liveResult.columns}
                  onAddFilter={handleAddFilter}
                />
              </div>
            </>
          )}
        </div>
      </aside>

      {/* ── Main panel ── */}
      <section className="flex min-w-0 flex-1 flex-col">
        {/* Tab bar */}
        <TabBar />

        {/* Saved query chips */}
        <SavedQueryChips onPick={handlePickSavedQuery} />

        {/* Smart search bar */}
        <SmartSearchBar
          leadingTable={leadingTable}
          schema={searchSchema}
          onSearch={handleSearch}
          onClear={() => {
            // no-op: user can clear via X; editor retains current query
          }}
        />

        {/* Toolbar */}
        <div className="flex items-center gap-2 border-b bg-background px-4 py-1.5 text-xs shadow-sm">
          <TimeRangePicker
            value={active?.timeRange ?? { preset: "1h" }}
            onChange={(r) => active && workspace.setTimeRange(active.id, r)}
          />
          {status === "running" ? (
            <Button size="sm" variant="destructive" onClick={() => active && cancel(active.id)}>
              <Square className="mr-1 h-3.5 w-3.5" /> Cancel
            </Button>
          ) : (
            <Button size="sm" onClick={runActive} disabled={!schema}>
              <Play className="mr-1 h-3.5 w-3.5" /> Run
              <kbd className="ml-2 rounded bg-primary-foreground/20 px-1 py-0.5 text-[10px] text-primary-foreground/80">⌘↵</kbd>
            </Button>
          )}
          <Button
            size="sm"
            variant="ghost"
            disabled={!hasResults}
            onClick={() => {
              if (!liveResult) return;
              downloadBlob(exportCsv(liveResult.columns, liveResult.rows), `kyma-${Date.now()}.csv`);
            }}
          >
            <Download className="mr-1 h-3 w-3" /> CSV
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={!hasResults}
            onClick={() => {
              if (!liveResult) return;
              downloadBlob(
                new Blob([JSON.stringify(liveResult.rows, null, 2)], { type: "application/json" }),
                `kyma-${Date.now()}.json`,
              );
            }}
          >
            <FileJson className="mr-1 h-3 w-3" /> JSON
          </Button>
          <span className="ml-auto tabular-nums">
            {active?.results.kind === "ok" && (
              <span className="font-medium text-foreground">
                {active.results.rowCount.toLocaleString()} rows · {active.results.durationMs.toFixed(0)} ms
              </span>
            )}
            {active?.results.kind === "error" &&
              (() => {
                const msg = `error: ${active.results.message}`;
                const truncated = msg.length > 80 ? msg.slice(0, 80) + "…" : msg;
                return (
                  <span className="text-destructive" title={msg}>
                    {truncated}
                  </span>
                );
              })()}
          </span>
        </div>

        {/* Editor */}
        <div className="bg-background border-b" style={{ height: "38%" }}>
          {active && schema && (
            <Editor
              value={active.query}
              onChange={(v) => workspace.setQuery(active.id, v)}
              onRun={runActive}
              schema={schema}
              database={database}
              knownValues={knownValues}
            />
          )}
        </div>

        {/* Histogram timeline — only when timestamp col present */}
        {hasResults && timeCol && liveResult && (
          <HistogramTimeline
            rows={liveResult.rows}
            timeCol={timeCol}
            onBucketClick={handleBucketClick}
          />
        )}

        {/* Results / Chart area */}
        <div className="flex min-h-0 flex-1 flex-col rounded-b-lg">
          {/* View switcher */}
          <div className="flex items-center gap-1 border-b bg-background px-3 py-1.5">
            <button className={tabBtn(view === "grid")} onClick={() => setView("grid")}>
              Results
            </button>
            <button className={tabBtn(view === "chart")} onClick={() => setView("chart")}>
              Chart
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-hidden bg-background rounded-b-lg border-x border-b border-muted/30 shadow-sm flex flex-col">
            {active?.results.kind === "idle" && <EmptyState text="Run a query to see results." />}
            {active?.results.kind === "running" && <EmptyState text="Streaming results…" />}
            {active?.results.kind === "error" && (
              <ErrorCard msg={active.results.message} rid={active.results.requestId} />
            )}
            {liveResult && active?.results.kind === "ok" && liveResult.rows.length === 0 && (
              <EmptyState
                text={`0 rows in ${active.results.durationMs.toFixed(0)} ms — try widening the time range.`}
              />
            )}
            {liveResult && active?.results.kind === "ok" && liveResult.rows.length > 0 && view === "grid" && (
              <>
                <RowFilterWrapper
                  rows={liveResult.rows}
                  columns={liveResult.columns}
                  filter={rowFilter}
                  onFilterChange={setRowFilter}
                />
              </>
            )}
            {liveResult && active?.results.kind === "ok" && liveResult.rows.length > 0 && view === "chart" && (
              <ChartPanel columns={liveResult.columns} rows={liveResult.rows} />
            )}
          </div>
        </div>
      </section>
      <CommandPalette onRun={runActive} />
    </div>
  );
}

import type { Column } from "@/sdk/arrow";

function RowFilterWrapper({
  rows,
  columns,
  filter,
  onFilterChange,
}: {
  rows: Record<string, unknown>[];
  columns: Column[];
  filter: string;
  onFilterChange: (v: string) => void;
}) {
  const filteredCount = filter.trim()
    ? rows.filter((row) =>
        columns.some((col) => {
          const v = row[col.name];
          if (v === null || v === undefined) return false;
          return String(v).toLowerCase().includes(filter.trim().toLowerCase());
        }),
      ).length
    : rows.length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RowFilter
        value={filter}
        onChange={onFilterChange}
        totalRows={rows.length}
        filteredRows={filteredCount}
      />
      <div className="min-h-0 flex-1 overflow-hidden">
        <ResultsGrid columns={columns} rows={rows} filter={filter} />
      </div>
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-xs text-muted-foreground">
      {text}
    </div>
  );
}

function ErrorCard({ msg, rid }: { msg: string; rid?: string }) {
  return (
    <div className="mx-auto mt-6 max-w-2xl rounded-lg border border-destructive/40 bg-destructive/5 p-4 text-xs shadow-sm">
      <div className="font-semibold text-destructive">Query failed</div>
      <pre className="mt-1 whitespace-pre-wrap font-mono text-destructive/90">{msg}</pre>
      {rid && (
        <div className="mt-2 text-muted-foreground">
          request id: <code>{rid}</code>
        </div>
      )}
    </div>
  );
}
