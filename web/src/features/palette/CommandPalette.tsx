import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Search, Code, Settings as Gear, FilePlus, Play } from "lucide-react";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { useWorkspace } from "@/features/tabs/workspace-store";

export function CommandPalette({ onRun }: { onRun: () => void }) {
  const [open, setOpen] = useState(false);
  const navigate = useNavigate();

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  const close = () => setOpen(false);

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Type a command or search…" />
      <CommandList>
        <CommandEmpty>No results.</CommandEmpty>
        <CommandGroup heading="Actions">
          <CommandItem
            onSelect={() => {
              onRun();
              close();
            }}
          >
            <Play className="mr-2 h-4 w-4" /> Run current query <kbd className="ml-auto text-xs">⌘↵</kbd>
          </CommandItem>
          <CommandItem
            onSelect={() => {
              useWorkspace.getState().newTab({ kind: "query" });
              close();
            }}
          >
            <FilePlus className="mr-2 h-4 w-4" /> New tab <kbd className="ml-auto text-xs">⌘T</kbd>
          </CommandItem>
        </CommandGroup>
        <CommandGroup heading="Navigate">
          <CommandItem
            onSelect={() => {
              navigate({ to: "/discover" });
              close();
            }}
          >
            <Search className="mr-2 h-4 w-4" /> Discover
          </CommandItem>
          <CommandItem
            onSelect={() => {
              navigate({ to: "/query", search: { q: undefined } });
              close();
            }}
          >
            <Code className="mr-2 h-4 w-4" /> Query Editor
          </CommandItem>
          <CommandItem
            onSelect={() => {
              navigate({ to: "/settings", search: { next: "/discover" } });
              close();
            }}
          >
            <Gear className="mr-2 h-4 w-4" /> Settings
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
