import { Plus, X } from "lucide-react";
import { useWorkspace, isTabDirty } from "./workspace-store";

export function TabBar() {
  const { tabs, activeId, setActive, closeTab, newTab } = useWorkspace();
  return (
    <div className="flex items-center gap-1 border-b bg-background px-2 py-1 text-xs">
      {tabs.map((t) => (
        <button key={t.id}
          onClick={() => setActive(t.id)}
          className={`flex items-center gap-1 rounded px-2 py-1 ${t.id === activeId ? "bg-accent text-accent-foreground" : "hover:bg-accent/40 text-muted-foreground"}`}>
          {isTabDirty(t) && <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />}
          <span className="max-w-[20ch] truncate">{t.title}</span>
          <X className="h-3 w-3 opacity-60 hover:opacity-100"
             onClick={(e) => { e.stopPropagation(); closeTab(t.id); }} />
        </button>
      ))}
      <button className="rounded p-1 hover:bg-accent/40" onClick={() => newTab({ query: "" })} title="New tab (⌘T)">
        <Plus className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
