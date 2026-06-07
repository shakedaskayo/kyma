import { createFileRoute, useSearch, useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState, useCallback } from "react";
import { encodeQueryState, decodeQueryState } from "@/lib/url-state";
import { toast } from "sonner";
import { Download, FileJson, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AskAIDialog } from "@/features/agent/AskAIDialog";
import { TabBar } from "@/features/tabs/TabBar";
import { HistogramTimeline, FieldStats } from "@kyma-ai/react/discover";
import { SavedQueryChips } from "@/features/saved-queries/SavedQueryChips";
import { SmartSearchBar } from "@/features/search/SmartSearchBar";
import { extractLeadingTable } from "@/features/search/parseSearch";
import type { SearchSchema } from "@/features/search/parseSearch";
import { downloadBlob } from "@/lib/download";
import { useWorkspace, findMatchingQueryTab } from "@/features/tabs/workspace-store";
import { useSession } from "@/sdk/session";
import { KymaQueryEditor } from "@kyma-ai/react/query";
import type { Column } from "@kyma-ai/client";

export const Route = createFileRoute("/_app/query")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  component: QueryEditorPage,
});

type LiveResult = {
  columns: Column[];
  rows: Record<string, unknown>[];
};

/** Minimal CSV export — used only for the download button. */
function buildCsvBlob(columns: Column[], rows: Record<string, unknown>[]): Blob {
  const escape = (s: string) => (/[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s);
  const header = columns.map((c) => escape(c.name)).join(",");
  const body = rows
    .map((r) => columns.map((c) => escape(r[c.name] == null ? "" : String(r[c.name]))).join(","))
    .join("\n");
  return new Blob([header + "\n" + body], { type: "text/csv" });
}

function QueryEditorPage() {
  const { database } = useSession();
  const workspace = useWorkspace();
  // Per-tab live result from onResults callback (used for histogram + field-stats + downloads)
  const [liveResult, setLiveResult] = useState<LiveResult | null>(null);
  const { q } = useSearch({ from: "/_app/query" });
  const navigate = useNavigate();

  // Find the active *query* tab. Discover tabs are filtered out — this route
  // is the Query Editor only.
  const activeRaw = workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0];
  const active = activeRaw && activeRaw.kind === "query" ? activeRaw : null;

  // Clear live result when switching tabs so stale data doesn't leak.
  const prevTabId = useRef<string | null>(null);
  useEffect(() => {
    if (active?.id !== prevTabId.current) {
      setLiveResult(null);
      prevTabId.current = active?.id ?? null;
    }
  }, [active?.id]);

  // Load from URL on first mount only.
  const bootstrapped = useRef(false);
  useEffect(() => {
    if (bootstrapped.current) return;
    bootstrapped.current = true;
    const hasAnyQueryTab = workspace.tabs.some((t) => t.kind === "query");
    if (q) {
      const decoded = decodeQueryState(q);
      if (decoded) {
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
      const activeTab = workspace.tabs.find((t) => t.id === workspace.activeId);
      if (activeTab?.kind !== "query") {
        const first = workspace.tabs.find((t) => t.kind === "query");
        if (first) workspace.setActive(first.id);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Build search schema for SmartSearchBar (requires schema data — we reuse the
  // package's internal query to get it, but we need it here for the smart bar.
  // Fetch it via the same endpoint/database from the session).
  const [webSchema, setWebSchema] = useState<SearchSchema | null>(null);
  useEffect(() => {
    // We cannot import fetchSchema here without duplicating schema fetching —
    // instead we derive the leading table from the active tab query text only.
    // The SmartSearchBar gracefully degrades to null schema.
    setWebSchema(null);
  }, []);

  // Leading table from active query (for SmartSearchBar)
  const leadingTable = useMemo(
    () => (active ? extractLeadingTable(active.state.query) : null),
    [active],
  );

  // Sync URL whenever the active tab's query or time range changes.
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  useEffect(() => {
    if (!active) return;
    if (!pathname.startsWith("/query") && !pathname.startsWith("/explore")) return;
    const encoded = encodeQueryState({
      query: active.state.query,
      preset: active.state.timeRange.preset,
      from: active.state.timeRange.from,
      to: active.state.timeRange.to,
    });
    if (encoded === q) return;
    navigate({ to: "/query", search: { q: encoded }, replace: true });
  }, [active, navigate, pathname, q]);

  // Find the timestamp column in current results
  const timeCol = liveResult?.columns.find((c) => c.kind === "time")?.name;

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

  const handleSearch = useCallback((kql: string) => {
    if (!active) return;
    workspace.updateQuery(active.id, { query: kql });
  }, [active]);

  const hasResults = Boolean(liveResult && liveResult.rows.length > 0);

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
    toast.info(answer.length > 200 ? answer.slice(0, 200) + "…" : answer);
  };

  return (
    <div className="flex h-full bg-muted/20">
      {/* ── Left rail ── */}
      <aside className="flex w-64 shrink-0 flex-col border-r bg-background">
        <div className="min-h-0 flex-1 overflow-hidden flex flex-col">
          {/* FieldStats: shown when we have live results */}
          {liveResult && (
            <div className="min-h-0 flex-1 overflow-hidden">
              <FieldStats
                rows={liveResult.rows}
                columns={liveResult.columns}
                onAddFilter={(col, value) => {
                  if (!active) return;
                  const addition = ` | where ${col} == "${value}"`;
                  workspace.updateQuery(active.id, { query: active.state.query + addition });
                }}
              />
            </div>
          )}
        </div>
      </aside>

      {/* ── Main panel ── */}
      <section className="flex min-w-0 flex-1 flex-col">
        {/* Tab bar */}
        <TabBar />

        {/* Saved query chips — derived from the connected schema */}
        <SavedQueryChips onPick={handlePickSavedQuery} schema={undefined} database={database} />

        {/* Smart search bar */}
        <SmartSearchBar
          leadingTable={leadingTable}
          schema={webSchema}
          onSearch={handleSearch}
          onClear={() => {}}
        />

        {/* Download toolbar — only shown when results available */}
        {hasResults && liveResult && (
          <div className="flex items-center gap-2 border-b bg-background px-4 py-1 text-xs">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                if (!liveResult) return;
                downloadBlob(buildCsvBlob(liveResult.columns, liveResult.rows), `kyma-${Date.now()}.csv`);
              }}
            >
              <Download className="mr-1 h-3 w-3" /> CSV
            </Button>
            <Button
              size="sm"
              variant="ghost"
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
            <Button size="sm" variant="outline" onClick={() => setAskOpen(true)}>
              <Sparkles className="mr-1 h-3.5 w-3.5" /> Ask AI
            </Button>
            <span className="ml-auto tabular-nums text-muted-foreground">
              {liveResult.rows.length.toLocaleString()} rows
            </span>
          </div>
        )}

        {/* Histogram timeline — only when timestamp col present */}
        {hasResults && timeCol && liveResult && (
          <HistogramTimeline
            rows={liveResult.rows}
            timeCol={timeCol}
            onBucketClick={handleBucketClick}
          />
        )}

        {/* Per-tab KymaQueryEditor — re-keyed on tabId so switching tabs mounts a fresh editor */}
        {active ? (
          <KymaQueryEditor
            key={active.id}
            defaultQuery={active.state.query}
            timeRange={active.state.timeRange}
            className="min-h-0 flex-1"
            onQueryChange={(q) => workspace.updateQuery(active.id, { query: q })}
            onResults={(r) => setLiveResult({ columns: r.columns as Column[], rows: r.rows })}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            No query tab — click + to open one.
          </div>
        )}
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
