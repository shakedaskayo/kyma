import { useNavigate } from "@tanstack/react-router";
import { Plus, X, Search, Code } from "lucide-react";
import { useWorkspace, isTabDirty, type Tab } from "./workspace-store";

function tabTitle(t: Tab): string {
  if (t.kind === "query") return t.state.title;
  // discover tab — show a short label based on the current search/scope.
  if (t.state.search.trim()) return t.state.search.trim().slice(0, 24);
  if (t.state.scope.kind === "sources" && t.state.scope.sources.length > 0) {
    return `discover (${t.state.scope.sources.length})`;
  }
  if (t.state.scope.kind === "view") return "discover (view)";
  return "discover";
}

export function TabBar() {
  const { tabs, activeId, setActive, closeTab, newTab } = useWorkspace();
  const navigate = useNavigate();

  // All tabs render on the unified Explore page (KymaExplore handles both
  // keyword search and KQL/SQL from one input).
  const activate = (t: Tab) => {
    setActive(t.id);
    void navigate({ to: "/explore", search: { q: undefined } });
  };

  return (
    <div className="flex items-center gap-1 border-b bg-background px-2 py-1 text-xs">
      {tabs.map((t) => {
        const Icon = t.kind === "discover" ? Search : Code;
        return (
          <button key={t.id}
            onClick={() => activate(t)}
            className={`flex items-center gap-1 rounded px-2 py-1 ${t.id === activeId ? "bg-accent text-accent-foreground" : "hover:bg-accent/40 text-muted-foreground"}`}>
            {isTabDirty(t) && <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />}
            <Icon className="h-3 w-3 opacity-70" />
            <span className="max-w-[20ch] truncate">{tabTitle(t)}</span>
            <X className="h-3 w-3 opacity-60 hover:opacity-100"
               onClick={(e) => { e.stopPropagation(); closeTab(t.id); }} />
          </button>
        );
      })}
      <button
        className="rounded p-1 hover:bg-accent/40"
        onClick={() => {
          newTab({ kind: "query" });
          void navigate({ to: "/explore", search: { q: undefined } });
        }}
        title="New tab (⌘T)"
      >
        <Plus className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
