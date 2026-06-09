import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useRouterState } from "@tanstack/react-router";
import { Database, ChevronDown, Check, Layers } from "lucide-react";
import { useSession } from "@/sdk/session";
import { fetchSchema } from "@/sdk/catalog";
import { cn } from "@/lib/utils";

/** Databases that hold kyma-internal state (agent memory). Hidden from the
 *  switcher and excluded from the "All databases" union — mirrors the backend's
 *  `INTERNAL_DATABASES` (`crates/kyma-server/src/discover/scope.rs`). */
export const INTERNAL_DATABASES = ["memory"];

/** Sentinel for "span every database" — sent verbatim as `x-database: *`, which
 *  the query engine resolves to a union across all accessible databases. */
export const ALL_DATABASES = "*";

/**
 * Per-route decision of what the header should say about database scope.
 *
 *  - "query": the Query Editor is cross-database-capable. Default scope is
 *    "All databases" (`*`); the user may filter to one database. Bound to
 *    `session.queryScope`, NOT `session.database`.
 *  - "switch": a single-database page (Agent, Connectors, Dashboards) that
 *    re-scopes to the active concrete database via the `x-database` header.
 *    Bound to `session.database`; no "All databases" option.
 *  - "all": cross-database *by design* (Graph merges every (database, graph)
 *    pair). Read-only chip.
 *  - "hidden": the header database is meaningless here — the page owns its own
 *    in-page scope control (Discover's ScopePicker) or isn't database-scoped.
 */
type HeaderScope =
  | { mode: "query" }
  | { mode: "switch" }
  | { mode: "all"; label: string; reason: string }
  | { mode: "hidden" };

export function headerScopeFor(pathname: string): HeaderScope {
  if (pathname.startsWith("/graph")) {
    return {
      mode: "all",
      label: "All databases",
      reason:
        "Graph spans every database — the unified canvas merges all (database, graph) pairs and isn't constrained by the active database.",
    };
  }
  // Query Editor (and the legacy /explore alias) unify across all databases by
  // default and can be filtered to one — a distinct affordance from the
  // single-database pages below.
  if (pathname.startsWith("/query") || pathname.startsWith("/explore")) {
    return { mode: "query" };
  }
  if (
    pathname.startsWith("/agent") ||
    pathname.startsWith("/connectors") ||
    pathname.startsWith("/dashboards")
  ) {
    return { mode: "switch" };
  }
  // Discover owns its scope in-page (ScopePicker); Credentials/Settings/etc.
  // aren't database-scoped at all.
  return { mode: "hidden" };
}

/**
 * Header slot that renders the right database-scope affordance for the current
 * route — the cross-database query scope, the concrete single-database
 * switcher, a read-only chip, or nothing.
 */
export function HeaderDatabaseScope() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const scope = headerScopeFor(pathname);

  const database = useSession((s) => s.database);
  const queryScope = useSession((s) => s.queryScope);
  const setSession = useSession((s) => s.set);

  if (scope.mode === "hidden") return null;
  if (scope.mode === "all") {
    return (
      <div
        title={scope.reason}
        className="flex cursor-default items-center gap-1 rounded-md border border-dashed bg-muted/20 px-2 py-0.5 text-xs text-muted-foreground"
      >
        <Database className="h-3.5 w-3.5 opacity-60" />
        <span className="max-w-[16ch] truncate font-medium">{scope.label}</span>
      </div>
    );
  }
  if (scope.mode === "query") {
    return (
      <ScopeSwitcher
        value={queryScope}
        allowAll
        onPick={(v) => setSession({ queryScope: v })}
      />
    );
  }
  return <ScopeSwitcher value={database} onPick={(v) => setSession({ database: v })} />;
}

/**
 * Header control to view and switch the active database scope.
 *
 * When `allowAll` is set, an "All databases" entry (the `*` sentinel) is
 * offered — picking it clears the scope to the cross-database union. Otherwise
 * only concrete databases are selectable. Internal databases (agent memory) are
 * never listed. The chosen value is sent as the `x-database` header, so
 * switching re-scopes the pages that key off it.
 */
function ScopeSwitcher({
  value,
  onPick,
  allowAll = false,
}: {
  value: string;
  onPick: (value: string) => void;
  allowAll?: boolean;
}) {
  const { endpoint, token } = useSession();
  const [open, setOpen] = useState(false);

  const schema = useQuery({
    queryKey: ["schema", endpoint],
    queryFn: () => fetchSchema({ endpoint, token }),
    enabled: Boolean(endpoint && open),
    staleTime: 60_000,
  });

  const dbs = (schema.data?.databases ?? []).filter(
    (db) => !INTERNAL_DATABASES.includes(db.name),
  );

  const isAll = value === ALL_DATABASES;
  const label = isAll ? "All databases" : value || "—";

  function pick(name: string) {
    if (name !== value) onPick(name);
    setOpen(false);
  }

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={allowAll ? "Switch database scope" : "Switch database"}
        className="flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs transition-colors hover:bg-muted"
      >
        {isAll ? <Layers className="h-3.5 w-3.5" /> : <Database className="h-3.5 w-3.5" />}
        <span className="max-w-[16ch] truncate font-medium">{label}</span>
        <ChevronDown className={cn("h-3 w-3 opacity-60 transition-transform", open && "rotate-180")} />
      </button>

      {open && (
        <>
          {/* click-away backdrop */}
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div className="absolute left-0 top-full z-40 mt-1 max-h-72 w-56 overflow-y-auto rounded-md border bg-popover p-1 shadow-md">
            <p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              {allowAll ? "Database scope" : "Database"}
            </p>
            {allowAll && (
              <button
                type="button"
                onClick={() => pick(ALL_DATABASES)}
                className={cn(
                  "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-xs transition-colors",
                  isAll ? "bg-accent text-accent-foreground" : "hover:bg-accent/60",
                )}
              >
                <Layers className="h-3.5 w-3.5 shrink-0 opacity-70" />
                <span className="flex-1 truncate">All databases</span>
                {isAll && <Check className="h-3.5 w-3.5 shrink-0" />}
              </button>
            )}
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
              const active = db.name === value;
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
