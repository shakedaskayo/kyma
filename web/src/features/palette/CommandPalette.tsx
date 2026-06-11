import { useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  Search,
  Code,
  Network,
  LayoutDashboard,
  Database,
  KeyRound,
  Sparkles,
  Brain,
  Settings as Gear,
  FilePlus,
  Monitor,
  Moon,
  Sun,
} from "lucide-react";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { useUI } from "@/lib/ui-store";
import { useTheme } from "@/lib/theme";

/**
 * App-wide command palette (⌘K / Ctrl-K). Mounted once in the shell so it works
 * on every route. Open state lives in the UI store so the top-bar search button
 * can trigger it too. Query execution stays a contextual ⌘↵ shortcut inside the
 * editor — this palette is a launcher (navigate / new tab / theme).
 */
export function CommandPalette() {
  const open = useUI((s) => s.cmdOpen);
  const setOpen = useUI((s) => s.setCmdOpen);
  const toggle = useUI((s) => s.toggleCmd);
  const navigate = useNavigate();
  const setTheme = useTheme((s) => s.setTheme);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [toggle]);

  const run = (fn: () => void) => () => {
    fn();
    setOpen(false);
  };

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Search or jump to…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>

        <CommandGroup heading="Explore">
          <CommandItem onSelect={run(() => navigate({ to: "/discover" }))}>
            <Search /> Discover
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/query", search: { q: undefined } }))}>
            <Code /> Query Editor
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/graph" }))}>
            <Network /> Graph
          </CommandItem>
        </CommandGroup>

        <CommandGroup heading="Build">
          <CommandItem onSelect={run(() => navigate({ to: "/dashboards" }))}>
            <LayoutDashboard /> Dashboards
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/data-sources" }))}>
            <Database /> Data Sources
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/credentials" }))}>
            <KeyRound /> Credentials
          </CommandItem>
        </CommandGroup>

        <CommandGroup heading="Intelligence">
          <CommandItem onSelect={run(() => navigate({ to: "/agent" }))}>
            <Sparkles /> Agent
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/memory" }))}>
            <Brain /> Memory
          </CommandItem>
        </CommandGroup>

        <CommandSeparator />

        <CommandGroup heading="Actions">
          <CommandItem
            onSelect={run(() => useWorkspace.getState().newTab({ kind: "query" }))}
          >
            <FilePlus /> New query tab <CommandShortcut>⌘T</CommandShortcut>
          </CommandItem>
          <CommandItem onSelect={run(() => navigate({ to: "/settings", search: { next: "/discover" } }))}>
            <Gear /> Settings
          </CommandItem>
        </CommandGroup>

        <CommandGroup heading="Theme">
          <CommandItem onSelect={run(() => setTheme("light"))}>
            <Sun /> Light
          </CommandItem>
          <CommandItem onSelect={run(() => setTheme("dark"))}>
            <Moon /> Dark
          </CommandItem>
          <CommandItem onSelect={run(() => setTheme("system"))}>
            <Monitor /> System
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
