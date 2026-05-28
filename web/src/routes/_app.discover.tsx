import { createFileRoute } from "@tanstack/react-router";
import { useEffect } from "react";
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

  useEffect(() => {
    const active = tabs.find((t) => t.id === activeId);
    if (active && active.kind === "discover") return;
    const firstDiscover = tabs.find((t) => t.kind === "discover");
    if (firstDiscover) {
      setActive(firstDiscover.id);
    } else {
      newTab({ kind: "discover" });
    }
  }, [tabs, activeId, newTab, setActive]);

  const active = tabs.find((t) => t.id === activeId);
  if (!active || active.kind !== "discover") return null;
  return (
    <div className="flex h-full flex-col">
      <TabBar />
      <div className="flex-1 min-h-0">
        <DiscoverPage tabId={active.id} />
      </div>
    </div>
  );
}
