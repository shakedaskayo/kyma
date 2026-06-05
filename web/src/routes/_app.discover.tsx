import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { DiscoverPage } from "@/features/discover/DiscoverPage";
import { TabBar } from "@/features/tabs/TabBar";

export const Route = createFileRoute("/_app/discover")({
  component: DiscoverRoute,
});

function DiscoverRoute() {
  const tabs = useWorkspace((s) => s.tabs);
  const activeId = useWorkspace((s) => s.activeId);
  const newTab = useWorkspace((s) => s.newTab);
  const setActive = useWorkspace((s) => s.setActive);

  // Render the active discover tab, falling back to the first one so closing
  // a tab (which may hand `activeId` to a query tab) never blanks the page.
  const activeTab = tabs.find((t) => t.id === activeId);
  const shown =
    activeTab?.kind === "discover" ? activeTab : tabs.find((t) => t.kind === "discover");

  // Bootstrap once per mount: ensure a discover tab exists and is active.
  // Ref-guarded so StrictMode's double effect can't create duplicates, and
  // run-once so it never fights TabBar clicks (which navigate between routes).
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
  return (
    <div className="flex h-full flex-col">
      <TabBar />
      <div className="flex-1 min-h-0">
        <DiscoverPage tabId={shown.id} />
      </div>
    </div>
  );
}
