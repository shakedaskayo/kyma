import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Database, ChevronDown, Check } from "lucide-react";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { cn } from "@/lib/utils";

/**
 * Header control to view and switch the active database. The database is part
 * of the session and is sent as the `x-database` header on every request, so
 * switching it transparently re-scopes Explore, Graph, Dashboards, etc. — the
 * react-query keys that include `database` refetch automatically.
 */
export function DatabaseSwitcher() {
  const { endpoint, token, database } = useSession();
  const setSession = useSession((s) => s.set);
  const [open, setOpen] = useState(false);

  const schema = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    enabled: Boolean(endpoint && open),
    staleTime: 60_000,
  });

  const dbs = schema.data?.databases ?? [];

  function pick(name: string) {
    if (name !== database) setSession({ database: name });
    setOpen(false);
  }

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title="Switch database"
        className="flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs transition-colors hover:bg-muted"
      >
        <Database className="h-3.5 w-3.5" />
        <span className="max-w-[16ch] truncate font-medium">{database || "—"}</span>
        <ChevronDown className={cn("h-3 w-3 opacity-60 transition-transform", open && "rotate-180")} />
      </button>

      {open && (
        <>
          {/* click-away backdrop */}
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div className="absolute left-0 top-full z-40 mt-1 max-h-72 w-56 overflow-y-auto rounded-md border bg-popover p-1 shadow-md">
            <p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              Database
            </p>
            {schema.isLoading && (
              <div className="px-2 py-1.5 text-xs text-muted-foreground">Loading…</div>
            )}
            {schema.isError && (
              <div className="px-2 py-1.5 text-xs text-destructive">Failed to list databases</div>
            )}
            {!schema.isLoading && !schema.isError && dbs.length === 0 && (
              <div className="px-2 py-1.5 text-xs text-muted-foreground">No databases</div>
            )}
            {dbs.map((db) => {
              const active = db.name === database;
              return (
                <button
                  key={db.name}
                  type="button"
                  onClick={() => pick(db.name)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs transition-colors",
                    active ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
                  )}
                >
                  <Database className="h-3.5 w-3.5 shrink-0 opacity-70" />
                  <span className="flex-1 truncate">{db.name}</span>
                  <span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">
                    {db.tables.length}
                  </span>
                  {active && <Check className="h-3.5 w-3.5 shrink-0" />}
                </button>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
