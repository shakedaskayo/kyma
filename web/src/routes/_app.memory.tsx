import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { Search, SlidersHorizontal, Workflow } from "lucide-react";
import { MemoryPanel } from "@/features/agent/MemoryPanel";
import { MemorySearch } from "@/features/agent/MemorySearch";
import { MemorySettingsPanel } from "@/features/agent/MemorySettingsPanel";

export const Route = createFileRoute("/_app/memory")({
  component: MemoryPage,
});

type Tab = "search" | "pipeline" | "settings";

function MemoryPage() {
  const [tab, setTab] = useState<Tab>("search");
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 border-b px-2 py-1">
        <TabButton
          active={tab === "search"}
          onClick={() => setTab("search")}
          icon={<Search className="h-3.5 w-3.5" />}
        >
          Search
        </TabButton>
        <TabButton
          active={tab === "pipeline"}
          onClick={() => setTab("pipeline")}
          icon={<Workflow className="h-3.5 w-3.5" />}
        >
          Ingestion
        </TabButton>
        <TabButton
          active={tab === "settings"}
          onClick={() => setTab("settings")}
          icon={<SlidersHorizontal className="h-3.5 w-3.5" />}
        >
          Settings
        </TabButton>
      </div>
      <div className="min-h-0 flex-1">
        {tab === "search" ? (
          <MemorySearch />
        ) : tab === "pipeline" ? (
          <MemoryPanel />
        ) : (
          <MemorySettingsPanel />
        )}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-md px-3 py-1 text-sm transition-colors ${
        active
          ? "bg-accent font-medium text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}
