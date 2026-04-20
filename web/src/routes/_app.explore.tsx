import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Play, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Editor } from "@/features/editor/Editor";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { useRunQuery, type TabResult } from "@/features/run/useRunQuery";

export const Route = createFileRoute("/_app/explore")({ component: ExplorePage });

function ExplorePage() {
  const { endpoint, token, database } = useSession();
  const workspace = useWorkspace();
  const { run, cancel } = useRunQuery();
  const [liveResult, setLiveResult] = useState<TabResult | null>(null);

  const { data: schema } = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    staleTime: 5 * 60_000,
    enabled: Boolean(endpoint && token),
  });

  // Ensure at least one tab exists.
  useEffect(() => {
    if (workspace.tabs.length === 0) workspace.newTab({ query: "otel_logs | take 50" });
  }, [workspace]);

  const active = workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0];
  const status = active?.results.kind ?? "idle";

  const runActive = async () => {
    if (!active) return;
    setLiveResult(null);
    await run(active, setLiveResult);
  };

  return (
    <div className="flex h-full flex-col">
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
        <div className="h-1/2 overflow-auto p-4 text-xs">
          {!liveResult && <p className="text-muted-foreground">Run a query to see results.</p>}
          {liveResult && <PreviewTable r={liveResult} />}
        </div>
      </div>
    </div>
  );
}

function PreviewTable({ r }: { r: TabResult }) {
  const cols = useMemo(() => r.columns.slice(0, 20), [r.columns]);
  return (
    <table className="min-w-full font-mono">
      <thead>
        <tr className="border-b">
          {cols.map((c) => (<th key={c.name} className="px-2 py-1 text-left">{c.name}</th>))}
        </tr>
      </thead>
      <tbody>
        {r.rows.slice(0, 200).map((row, i) => (
          <tr key={i} className="border-b">
            {cols.map((c) => (<td key={c.name} className="px-2 py-1">{String(row[c.name] ?? "")}</td>))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
