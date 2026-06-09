import { createFileRoute, useSearch, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { KymaExplore } from "@kyma-ai/react/explore";
import { TabBar } from "@/features/tabs/TabBar";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { useSession } from "@/sdk/session";
import { decodeQueryState } from "@/lib/url-state";

/**
 * Unified Explore page — one smart input (keyword search or KQL/SQL) over a
 * shared scope + time-range + results shell. Replaces the separate Discover and
 * Query Editor routes (both now redirect here).
 *
 * It renders whichever tab is active regardless of its kind: a `query` tab's
 * text lives in `state.query`, a (legacy) `discover` tab's in `state.search`.
 * KymaExplore is mode-agnostic — the input text drives keyword-vs-KQL/SQL.
 */
export const Route = createFileRoute("/_app/explore")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  component: ExplorePage,
});

function ExplorePage() {
  const queryScope = useSession((s) => s.queryScope);
  const workspace = useWorkspace();
  const navigate = useNavigate();
  const { q } = useSearch({ from: "/_app/explore" });

  const active =
    workspace.tabs.find((t) => t.id === workspace.activeId) ?? workspace.tabs[0] ?? null;

  // Bootstrap one tab on first mount (decode a ?q deep-link if present).
  const booted = useRef(false);
  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    if (q) {
      const decoded = decodeQueryState(q);
      if (decoded) {
        const timeRange = { preset: decoded.preset, from: decoded.from, to: decoded.to };
        workspace.newTab({ kind: "query", state: { query: decoded.query, timeRange } });
        return;
      }
    }
    if (workspace.tabs.length === 0) workspace.newTab({ kind: "query" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Input text + change handler depend on the active tab's kind.
  const text = active ? (active.kind === "query" ? active.state.query : active.state.search) : "";
  const onText = (v: string) => {
    if (!active) return;
    if (active.kind === "query") workspace.updateQuery(active.id, { query: v });
    else workspace.patchDiscover(active.id, { search: v });
  };

  return (
    <div className="flex h-full flex-col">
      <TabBar />
      <div className="min-h-0 flex-1">
        {active ? (
          <KymaExplore
            key={`${active.id}:${queryScope}`}
            defaultQuery={text}
            timeRange={active.state.timeRange}
            database={queryScope}
            className="h-full"
            onQueryChange={onText}
            onViewInGraph={(nodeId) => navigate({ to: "/graph", search: { focus: nodeId } })}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            No tab — click + to open one.
          </div>
        )}
      </div>
    </div>
  );
}
