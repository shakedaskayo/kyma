import { Link, useRouterState } from "@tanstack/react-router";
import { cn } from "@/lib/utils";

const TABS = [
  { to: "/data-sources", label: "Sources", exact: true },
  { to: "/data-sources/watchers", label: "File Watchers", exact: false },
  { to: "/data-sources/sync", label: "Memory Sync", exact: false },
  { to: "/data-sources/memories", label: "Memories", exact: false },
] as const;

/**
 * Section nav for the Data Sources surface. Mirrors the Memory header's
 * active-link idiom: pathname-driven `cn()` classes (so the active tab keeps
 * its base classes) plus an underline accent span. The "Sources" tab is the
 * index route — every sibling shares its prefix, so it claims any
 * /data-sources path (including /data-sources/$id details) that no sibling
 * tab matches.
 */
export function DataSourcesTabs() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  // Boundary-safe prefix match: "/data-sources/sync/foo" matches the sync tab
  // but "/data-sources/sync-other" does not.
  const matches = (to: string) => pathname === to || pathname.startsWith(to + "/");
  const siblingActive = TABS.some((t) => !t.exact && matches(t.to));

  return (
    <nav className="flex items-center gap-1 pb-1.5">
      {TABS.map((t) => {
        const active = t.exact ? matches(t.to) && !siblingActive : matches(t.to);
        return (
          <Link
            key={t.to}
            to={t.to}
            className={cn(
              "relative rounded-md px-3 py-1.5 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {t.label}
            {active && (
              <span className="absolute inset-x-2 -bottom-[7px] h-0.5 rounded-full bg-primary" />
            )}
          </Link>
        );
      })}
    </nav>
  );
}
