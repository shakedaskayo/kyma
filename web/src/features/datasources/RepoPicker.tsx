import { useMemo, useState } from "react";
import { Check, Lock, RefreshCw, Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { GitHubRepo } from "@/sdk/datasources";

interface Props {
  /** Fetched repos (lifted to the wizard so the PAT is fetched once). */
  repos: GitHubRepo[] | undefined;
  isPending: boolean;
  isError: boolean;
  /** Re-fetch the repo list. */
  onRefresh: () => void;
  /** Selected repo full_names ("owner/name"). */
  selected: string[];
  onChange: (selected: string[]) => void;
}

export function RepoPicker({ repos, isPending, isError, onRefresh, selected, onChange }: Props) {
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const list = repos ?? [];
    const q = query.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (r) =>
        r.full_name.toLowerCase().includes(q) ||
        (r.description ?? "").toLowerCase().includes(q),
    );
  }, [repos, query]);

  const toggle = (fullName: string) => {
    onChange(
      selected.includes(fullName)
        ? selected.filter((n) => n !== fullName)
        : [...selected, fullName],
    );
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="h-9 pl-8"
            placeholder="Search repositories…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <button
          type="button"
          className="inline-flex h-9 items-center gap-1.5 rounded-md border px-3 text-xs text-muted-foreground hover:bg-accent"
          onClick={onRefresh}
          disabled={isPending}
        >
          <RefreshCw className={cn("h-3.5 w-3.5", isPending && "animate-spin")} />
          Refresh
        </button>
      </div>

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{selected.length} selected</span>
        {repos && repos.length > 0 && (
          <button
            type="button"
            className="hover:text-foreground"
            onClick={() =>
              onChange(
                selected.length === filtered.length ? [] : filtered.map((r) => r.full_name),
              )
            }
          >
            {selected.length === filtered.length ? "Clear all" : "Select all"}
          </button>
        )}
      </div>

      <div className="rounded-md border">
        {isPending && (
          <div className="flex items-center justify-center py-12 text-sm text-muted-foreground animate-pulse">
            Fetching repositories…
          </div>
        )}
        {isError && (
          <div className="flex flex-col items-center gap-2 py-12 text-center text-sm text-destructive">
            <p>Could not fetch repositories.</p>
            <button
              type="button"
              className="rounded-md border px-3 py-1 text-xs text-foreground hover:bg-accent"
              onClick={onRefresh}
            >
              Retry
            </button>
          </div>
        )}
        {!isPending && !isError && repos && filtered.length === 0 && (
          <div className="py-12 text-center text-sm text-muted-foreground">
            {query ? "No matching repositories." : "No repositories found for this token."}
          </div>
        )}
        {!isPending && !isError && filtered.length > 0 && (
          <ScrollArea className="h-72">
            <ul className="divide-y">
              {filtered.map((r) => (
                <RepoItem
                  key={r.full_name}
                  repo={r}
                  checked={selected.includes(r.full_name)}
                  onToggle={() => toggle(r.full_name)}
                />
              ))}
            </ul>
          </ScrollArea>
        )}
      </div>
    </div>
  );
}

function RepoItem({
  repo,
  checked,
  onToggle,
}: {
  repo: GitHubRepo;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onToggle}
        className={cn(
          "flex w-full items-center gap-3 px-3 py-2 text-left transition hover:bg-accent/50",
          checked && "bg-primary/5",
        )}
      >
        <span
          className={cn(
            "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
            checked ? "border-primary bg-primary text-primary-foreground" : "border-input",
          )}
        >
          {checked && <Check className="h-3 w-3" />}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-sm font-medium">{repo.full_name}</span>
            {repo.private && <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />}
          </div>
          {repo.description && (
            <div className="truncate text-xs text-muted-foreground">{repo.description}</div>
          )}
        </div>
        <span className="shrink-0 text-[10px] text-muted-foreground">{repo.default_branch}</span>
      </button>
    </li>
  );
}
