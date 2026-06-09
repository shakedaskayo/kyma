import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { KymaDiscover } from "@kyma-ai/react/discover";
import type { Scope, TimeRange as KymaTimeRange } from "@kyma-ai/react/discover";
import { TabBar } from "@/features/tabs/TabBar";
import { SavedViewsMenu } from "@/features/discover/SavedViewsMenu";
import { initialDiscoverTabState } from "@/features/discover/discover-store";

export const Route = createFileRoute("/_app/discover")({
  component: DiscoverRoute,
});

function DiscoverRoute() {
  const tabs = useWorkspace((s) => s.tabs);
  const activeId = useWorkspace((s) => s.activeId);
  const newTab = useWorkspace((s) => s.newTab);
  const setActive = useWorkspace((s) => s.setActive);
  const patchDiscover = useWorkspace((s) => s.patchDiscover);

  // Render the active discover tab, falling back to the first one so closing
  // a tab (which may hand `activeId` to a query tab) never blanks the page.
  const activeTab = tabs.find((t) => t.id === activeId);
  const shown =
    activeTab?.kind === "discover" ? activeTab : tabs.find((t) => t.kind === "discover");

  // Bootstrap once per mount: ensure a discover tab exists and is active.
  const booted = useRef(false);
  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    if (shown) {
      if (shown.id !== activeId) setActive(shown.id);
    } else {
      newTab({ kind: "discover" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!shown || shown.kind !== "discover") return null;

  // Defensive: a tab persisted by an older workspace schema can lack `state`
  // (stale localStorage) — fall back to a fresh discover state instead of
  // crashing the route on `st.search`.
  const st = shown.state ?? initialDiscoverTabState();

  const handleExportKql = (kql: string) => {
    newTab({
      kind: "query",
      state: {
        title: "from discover",
        query: kql,
        timeRange: st.timeRange,
        results: { kind: "idle" },
        chart: {},
        submittedQuery: null,
      },
    });
  };

  return (
    <div className="flex h-full flex-col">
      <TabBar />

      {/* Web-only discover chrome: SavedViewsMenu */}
      {/* Live toggle is omitted in v1 — KymaDiscover reserves a `live` prop for v2.
          The LiveSession WebSocket path requires a raw session token that cannot
          be threaded through KymaDiscover's transport-agnostic client today. */}
      <div className="flex items-center gap-2 px-3 py-1 border-b bg-background">
        <SavedViewsMenu
          currentScope={st.scope}
          onSaved={() => {
            // Nothing extra needed — saved view is persisted on the server
          }}
        />
      </div>

      <div className="flex-1 min-h-0">
        {/*
          key remounts KymaDiscover per tab so per-tab state (submitted query,
          open row, scroll position) never bleeds across tabs.

          Applying a saved view: set tab state (search/scope/timeRange) via
          patchDiscover then force a remount by appending a revision counter to
          the key. For now, SavedViewsMenu only saves views (does not apply
          them), so a simple tabId key is sufficient. When saved-view application
          is added, use key={`${shown.id}:${st.savedViewRevision ?? 0}`}.
        */}
        <KymaDiscover
          key={shown.id}
          defaultQuery={st.search}
          scope={st.scope as Scope}
          timeRange={st.timeRange}
          className="h-full"
          onSearchChange={(search) => patchDiscover(shown.id, { search })}
          onScopeChange={(scope) => patchDiscover(shown.id, { scope })}
          onTimeRangeChange={(timeRange: KymaTimeRange) => patchDiscover(shown.id, { timeRange })}
          onExportKql={handleExportKql}
        />
      </div>
    </div>
  );
}
