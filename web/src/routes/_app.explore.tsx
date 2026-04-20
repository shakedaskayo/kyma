import { createFileRoute, useSearch, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { encodeQueryState, decodeQueryState } from "@/lib/url-state";
import { useQuery } from "@tanstack/react-query";
import { Play, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Editor } from "@/features/editor/Editor";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ResultsGrid, exportCsv } from "@/features/results-grid/ResultsGrid";
import { ChartPanel } from "@/features/chart/ChartPanel";
import { SchemaBrowser } from "@/features/schema-browser/SchemaBrowser";
import { TabBar } from "@/features/tabs/TabBar";
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
  const { q } = useSearch({ from: "/_app/explore" });
  const navigate = useNavigate();

  const { data: schema } = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    staleTime: 5 * 60_000,
    enabled: Boolean(endpoint && token),
  });

  // Load from URL on first mount only. Ref guard prevents re-running on re-render.
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
  }, [active?.query, active?.timeRange.preset, active?.timeRange.from, active?.timeRange.to, navigate]);
  const status = active?.results.kind ?? "idle";

  const runActive = async () => {
    if (!active) return;
    setLiveResult(null);
    await run(active, setLiveResult);
  };

  useWorkspaceShortcuts(runActive);

const tabBtn = (on: boolean) =>
  `rounded-md px-2 py-0.5 transition ${on ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/50"}`;

  return (
    <div className="flex h-full">
      <aside className="w-64 shrink-0 border-r">
        <SchemaBrowser schema={schema} onInsert={(t) => active && workspace.setQuery(active.id, `${active.query}${active.query.endsWith("\n") || !active.query ? "" : " "}${t}`)} />
      </aside>
      <section className="flex flex-1 flex-col">
        <TabBar />
        <div className="flex items-center gap-2 border-b px-4 py-2 text-xs">
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
              <Play className="mr-1 h-3.5 w-3.5" /> Run <kbd className="ml-2 text-muted-foreground">⌘↵</kbd>
            </Button>
          )}
          <Button size="sm" variant="ghost" disabled={!liveResult?.rows.length} onClick={() => {
            if (!liveResult) return;
            downloadBlob(exportCsv(liveResult.columns, liveResult.rows), `kyma-${Date.now()}.csv`);
          }}>Export CSV</Button>
          <Button size="sm" variant="ghost" disabled={!liveResult?.rows.length} onClick={() => {
            if (!liveResult) return;
            downloadBlob(new Blob([JSON.stringify(liveResult.rows, null, 2)], { type: "application/json" }), `kyma-${Date.now()}.json`);
          }}>Export JSON</Button>
          <span className="ml-auto text-muted-foreground">
            {active?.results.kind === "ok"
              ? `${active.results.rowCount.toLocaleString()} rows · ${active.results.durationMs.toFixed(0)} ms`
              : active?.results.kind === "error" ? `error: ${active.results.message}` : ""}
          </span>
        </div>

        <div className="flex-1 overflow-hidden">
          <div className="h-1/2 border-b">
            {active && schema && (
              <Editor
                value={active.query}
                onChange={(v) => workspace.setQuery(active.id, v)}
                onRun={runActive}
                schema={schema}
                database={database}
              />
            )}
          </div>
          <div className="h-1/2 flex flex-col">
            <div className="flex items-center gap-1 border-b px-3 py-1 text-xs">
              <button className={tabBtn(view === "grid")}  onClick={() => setView("grid")}>Results</button>
              <button className={tabBtn(view === "chart")} onClick={() => setView("chart")}>Chart</button>
            </div>
            <div className="flex-1 overflow-hidden">
              {liveResult && view === "grid"  && <ResultsGrid columns={liveResult.columns} rows={liveResult.rows} />}
              {liveResult && view === "chart" && <ChartPanel  columns={liveResult.columns} rows={liveResult.rows} />}
              {!liveResult && <p className="p-6 text-xs text-muted-foreground">Run a query to see results.</p>}
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
