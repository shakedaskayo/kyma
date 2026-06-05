import { createFileRoute, useSearch, useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { encodeQueryState, decodeQueryState } from "@/lib/url-state";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import { Play, Square, Download, FileJson, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AskAIDialog } from "@/features/agent/AskAIDialog";
import { Editor } from "@/features/editor/Editor";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ResultsGrid, exportCsv } from "@/features/results-grid/ResultsGrid";
import { RowFilter } from "@/features/results-grid/RowFilter";
import { ChartPanel } from "@/features/chart/ChartPanel";
import { SchemaBrowser } from "@/features/schema-browser/SchemaBrowser";
import { TabBar } from "@/features/tabs/TabBar";
import { HistogramTimeline } from "@/features/histogram/HistogramTimeline";
import { FieldStats } from "@/features/field-stats/FieldStats";
import { SavedQueryChips } from "@/features/saved-queries/SavedQueryChips";
import { SmartSearchBar } from "@/features/search/SmartSearchBar";
import { extractLeadingTable } from "@/features/search/parseSearch";
import type { SearchSchema } from "@/features/search/parseSearch";
import { downloadBlob } from "@/lib/download";
import { useWorkspace, findMatchingQueryTab } from "@/features/tabs/workspace-store";
import { useWorkspaceShortcuts } from "@/lib/shortcuts";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { useRunQuery, type TabResult } from "@/features/run/useRunQuery";

export const Route = createFileRoute("/_app/query")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  component: QueryEditorPage,
});

function QueryEditorPage() {
  const { endpoint, token, database } = useSession();
  const workspace = useWorkspace();
  const [liveResult, setLiveResult] = useState<TabResult | null>(null);
  const [view, setView] = useState<"grid" | "chart">("grid");
  const [rowFilter, setRowFilter] = useState("");
  const { q } = useSearch({ from: "/_app/query" });
  const navigate = useNavigate();

  const { data: schema } = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    staleTime: 5 * 60_000,
    enabled: Boolean(endpoint),
  });

  // Tables (in the current db) that actually have a `timestamp` column — only
  // those get the time-range filter injected; code/graph tables don't.
  const timestampTables = useMemo(() => {
    const set = new Set<string>();
    schema?.databases
      .find((d) => d.name === database)
      ?.tables.forEach((t) => {
        if (t.columns.some((c) => c.name.toLowerCase() === "timestamp")) set.add(t.name);
      });
    return set;
  }, [schema, database]);
  const { run, cancel } = useRunQuery(timestampTables);

  // Find the active *query* tab. Discover tabs are filtered out — this route
  // is the Query Editor only.
  const activeRaw = workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0];
  const active = activeRaw && activeRaw.kind === "query" ? activeRaw : null;

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
    const hasAnyQueryTab = workspace.tabs.some((t) => t.kind === "query");
    if (q) {
      const decoded = decodeQueryState(q);
      if (decoded) {
        // Reuse an existing tab showing this exact state — the URL is synced
        // from the editor on every change, so each reload arrives with a `?q=`
        // that matches a persisted tab. Spawning unconditionally here used to
        // pile up a duplicate tab per page load.
        const timeRange = { preset: decoded.preset, from: decoded.from, to: decoded.to };
        const existing = findMatchingQueryTab(workspace.tabs, decoded.query, timeRange);
        if (existing) {
          workspace.setActive(existing.id);
        } else {
          workspace.newTab({ kind: "query", state: { query: decoded.query, timeRange } });
        }
      } else if (!hasAnyQueryTab) {
        workspace.newTab({ kind: "query" });
      }
    } else if (!hasAnyQueryTab) {
      workspace.newTab({ kind: "query" });
    } else {
      // No deep link, query tabs exist — make sure one is actually active so
      // arriving from /discover doesn't render a dead editor.
      const activeTab = workspace.tabs.find((t) => t.id === workspace.activeId);
      if (activeTab?.kind !== "query") {
        const first = workspace.tabs.find((t) => t.kind === "query");
        if (first) workspace.setActive(first.id);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Once the schema loads, seed the initial tab with a real first-table query
  // (replacing an empty editor or the legacy `otel_logs | take 50` default that
  // doesn't exist in most databases). Runs once — never tramples a real query.
  const seededDefault = useRef(false);
  useEffect(() => {
    if (seededDefault.current || !schema || !active) return;
    const cur = active.state.query.trim();
    if (cur === "" || cur === "otel_logs | take 50") {
      const t0 = schema.databases.find((d) => d.name === database)?.tables[0]?.name;
      if (t0) {
        workspace.updateQuery(active.id, { query: `${t0} | take 50` });
        seededDefault.current = true;
      }
    } else {
      seededDefault.current = true;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [schema, database, active?.id]);

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
    () => (active ? extractLeadingTable(active.state.query) : null),
    [active],
  );

  // Sync URL whenever the active tab's query or time range changes.
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  useEffect(() => {
    if (!active) return;
    // Only sync while we're actually on /query. During a navigation away,
    // `navigate`'s identity changes and re-fires this effect one last time
    // while the page is still mounted — without this guard it yanks the
    // in-flight navigation back to /query (the "can't leave the Query
    // Editor" bug).
    if (!pathname.startsWith("/query") && !pathname.startsWith("/explore")) return;
    const encoded = encodeQueryState({
      query: active.state.query,
      preset: active.state.timeRange.preset,
      from: active.state.timeRange.from,
      to: active.state.timeRange.to,
    });
    if (encoded === q) return; // already in the URL — don't churn history
    navigate({ to: "/query", search: { q: encoded }, replace: true });
  }, [active, navigate, pathname, q]);

  const status = active?.state.results.kind ?? "idle";

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
    workspace.updateQuery(active.id, { query: active.state.query + addition });
  };

  const handleBucketClick = (from: Date, to: Date) => {
    if (!active) return;
    const fmt = (d: Date) => d.toISOString();
    const addition = ` | where timestamp >= datetime(${fmt(from)}) and timestamp < datetime(${fmt(to)})`;
    workspace.updateQuery(active.id, { query: active.state.query + addition });
  };

  const handlePickSavedQuery = (query: string) => {
    if (!active) return;
    workspace.updateQuery(active.id, { query });
  };

  const handleSearch = (kql: string) => {
    if (!active) return;
    workspace.updateQuery(active.id, { query: kql });
    // Run the query with the updated query text directly
    setLiveResult(null);
    void run({ ...active, state: { ...active.state, query: kql } }, setLiveResult);
  };

  const tabBtn = (on: boolean) =>
    `rounded-md px-2.5 py-1 text-xs transition font-medium ${on ? "bg-primary text-primary-foreground shadow-sm" : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"}`;

  const hasResults = Boolean(liveResult && active?.state.results.kind === "ok" && liveResult.rows.length > 0);

  const [askOpen, setAskOpen] = useState(false);
  const onAskedSql = (sql: string) => {
    if (!active) {
      toast.info("No active tab; copied SQL to clipboard instead.");
      void navigator.clipboard.writeText(sql);
      return;
    }
    workspace.updateQuery(active.id, { query: sql });
    toast.success("Agent-generated SQL injected into the editor.");
  };
  const onAskedNoSql = (answer: string) => {
    // Agent didn't run any SQL — show the answer as a toast so the user
    // sees what it said.
    toast.info(answer.length > 200 ? answer.slice(0, 200) + "…" : answer);
  };

  return (
    <div className="flex h-full bg-muted/20">
      {/* ── Left rail ── */}
      <aside className="flex w-64 shrink-0 flex-col border-r bg-background">
        <div className="min-h-0 flex-1 overflow-hidden flex flex-col">
          <SchemaBrowser
            schema={schema}
            onInsert={(t) =>
              active &&
              workspace.updateQuery(active.id, {
                query: `${active.state.query}${active.state.query.endsWith("\n") || !active.state.query ? "" : " "}${t}`,
              })
            }
            onReplaceAndRun={(kql, _db) => {
              // Open the quick-action query in a new tab and execute it so
              // the user's current editor content isn't trampled.
              const id = workspace.newTab({ kind: "query", state: { query: kql } });
              setTimeout(() => {
                const t = useWorkspace.getState().tabs.find((x) => x.id === id);
                if (t) void run(t, setLiveResult);
              }, 0);
            }}
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

        {/* Saved query chips — derived from the connected schema */}
        <SavedQueryChips onPick={handlePickSavedQuery} schema={schema} database={database} />

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
            value={active?.state.timeRange ?? { preset: "1h" }}
            onChange={(r) => active && workspace.updateQuery(active.id, { timeRange: r })}
            disabled={Boolean(leadingTable) && !timestampTables.has(leadingTable!)}
            disabledReason="This table has no timestamp column — the time range doesn't apply."
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
          <Button size="sm" variant="outline" onClick={() => setAskOpen(true)}>
            <Sparkles className="mr-1 h-3.5 w-3.5" /> Ask AI
          </Button>
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
            {active?.state.results.kind === "ok" && (
              <span className="font-medium text-foreground">
                {active.state.results.rowCount.toLocaleString()} rows · {active.state.results.durationMs.toFixed(0)} ms
              </span>
            )}
            {active?.state.results.kind === "error" &&
              (() => {
                const msg = `error: ${active.state.results.message}`;
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
              value={active.state.query}
              onChange={(v) => workspace.updateQuery(active.id, { query: v })}
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
            {active?.state.results.kind === "idle" && <EmptyState text="Run a query to see results." />}
            {active?.state.results.kind === "running" && <EmptyState text="Streaming results…" />}
            {active?.state.results.kind === "error" && (
              <ErrorCard msg={active.state.results.message} rid={active.state.results.requestId} />
            )}
            {liveResult && active?.state.results.kind === "ok" && liveResult.rows.length === 0 && (
              <EmptyState
                text={`0 rows in ${active.state.results.durationMs.toFixed(0)} ms — try widening the time range.`}
              />
            )}
            {liveResult && active?.state.results.kind === "ok" && liveResult.rows.length > 0 && view === "grid" && (
              <>
                <RowFilterWrapper
                  rows={liveResult.rows}
                  columns={liveResult.columns}
                  filter={rowFilter}
                  onFilterChange={setRowFilter}
                />
              </>
            )}
            {liveResult && active?.state.results.kind === "ok" && liveResult.rows.length > 0 && view === "chart" && (
              <ChartPanel columns={liveResult.columns} rows={liveResult.rows} />
            )}
          </div>
        </div>
      </section>
      <AskAIDialog
        open={askOpen}
        onOpenChange={setAskOpen}
        onSql={onAskedSql}
        onNoSql={onAskedNoSql}
      />
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
  // Extract the error code if the message follows the common
  // "<code>: <details>" shape. Split on the first colon so that SQL
  // messages with colons (e.g. "SchemaError: Column not found") still
  // get the code callout.
  const { code, body, hint } = dissectErrorMessage(msg);
  return (
    <div className="mx-auto mt-6 max-w-2xl rounded-lg border border-destructive/40 bg-destructive/5 p-4 text-xs shadow-sm">
      <div className="flex items-center gap-2">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="h-4 w-4 text-destructive">
          <circle cx="12" cy="12" r="10" />
          <line x1="12" y1="8" x2="12" y2="12" />
          <line x1="12" y1="16" x2="12.01" y2="16" />
        </svg>
        <div className="font-semibold text-destructive">
          {code ? "Query failed" : "Query failed"}
        </div>
        {code && (
          <code className="rounded bg-destructive/15 px-1.5 py-0.5 font-mono text-[10px] text-destructive">
            {code}
          </code>
        )}
      </div>
      <pre className="mt-2 whitespace-pre-wrap break-words font-mono text-destructive/90">
        {body}
      </pre>
      {hint && (
        <div className="mt-2 rounded border border-destructive/30 bg-background/40 p-2 text-destructive/80">
          <span className="font-semibold">Hint: </span>
          {hint}
        </div>
      )}
      {rid && (
        <div className="mt-2 flex items-center gap-1 text-muted-foreground">
          request id: <code className="select-all">{rid}</code>
        </div>
      )}
    </div>
  );
}

/** Parse a kyma error message to expose the code + a helpful hint.
 * Handles patterns seen from /v1/query:
 *   - "bad_request_body: failed to decode NDJSON: ..."
 *   - "SchemaError: Column not found: <col>"
 *   - "parse_error: expected '|' at line 1"
 *   - plain unstructured strings
 */
function dissectErrorMessage(msg: string): {
  code?: string;
  body: string;
  hint?: string;
} {
  // Common kyma error-code prefix: lower_snake_case up to the first colon.
  const m = msg.match(/^([a-z][a-z0-9_]+):\s+(.*)$/s);
  let code: string | undefined;
  let body = msg;
  if (m) {
    code = m[1];
    body = m[2];
  }
  // Heuristic hints for the most frequent failures.
  let hint: string | undefined;
  const lower = msg.toLowerCase();
  if (lower.includes("table not found") || lower.includes("relation does not exist")) {
    hint =
      "The referenced table doesn't exist in the selected database. Double-click a table in the left panel to paste its name.";
  } else if (lower.includes("column not found") || lower.includes("no field named")) {
    hint =
      "That column isn't in the table's schema. Expand the table on the left to see its actual columns.";
  } else if (lower.includes("syntax") || lower.includes("expected")) {
    hint =
      "Likely a KQL syntax issue — pipe syntax uses '|', strings are double-quoted, comparisons use '=='.";
  } else if (lower.includes("bad_request_body") || lower.includes("ndjson")) {
    hint =
      "Request body failed to decode. If you're ingesting, check that each line is a valid JSON object and all required columns are present.";
  } else if (lower.includes("budget") || lower.includes("too many requests")) {
    hint =
      "Query budget exceeded. Narrow the time range or add a `| take N` to limit rows.";
  }
  return { code, body, hint };
}
