import { useEffect } from "react";
import { useWorkspace } from "@/features/tabs/workspace-store";

export function useWorkspaceShortcuts(runActive: () => void) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      if (e.key === "Enter") { e.preventDefault(); runActive(); }
      else if (e.key.toLowerCase() === "t") { e.preventDefault(); useWorkspace.getState().newTab({ query: "" }); }
      else if (e.key.toLowerCase() === "w") {
        e.preventDefault();
        const s = useWorkspace.getState();
        if (s.activeId) s.closeTab(s.activeId);
      } else if (/^[1-9]$/.test(e.key)) {
        const idx = Number(e.key) - 1;
        const t = useWorkspace.getState().tabs[idx];
        if (t) { e.preventDefault(); useWorkspace.getState().setActive(t.id); }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [runActive]);
}
