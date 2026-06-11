import { useMemo, useState } from "react";
import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { CatalogEntry } from "@/sdk/datasources";
import { useDataSourceCatalog } from "./useDataSources";
import { CATEGORIES, categoryLabel, authLabel } from "./datasource-kinds";
import { BrandIcon } from "./BrandIcon";

/**
 * The vendor catalog grid — the source-selection surface. Entries come from the
 * engine (`/v1/data-sources/catalog`), grouped by category. Available data sources
 * are clickable; "coming soon" ones are shown (greyed, with a badge) so the
 * product reads like a real integrations marketplace, not a single-vendor tool.
 */
export function DataSourceCatalog({
  selectedType,
  onPick,
}: {
  selectedType?: string | null;
  onPick: (entry: CatalogEntry) => void;
}) {
  const { data, isLoading, isError, error } = useDataSourceCatalog();
  const [query, setQuery] = useState("");

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const items = (data ?? []).filter(
      (e) =>
        !q ||
        e.label.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q) ||
        e.category.toLowerCase().includes(q),
    );
    const order = CATEGORIES.map((c) => c.id);
    const byCat = new Map<string, CatalogEntry[]>();
    for (const e of items) {
      const arr = byCat.get(e.category) ?? [];
      arr.push(e);
      byCat.set(e.category, arr);
    }
    // Known categories first (in CATEGORIES order), then any others.
    const cats = [...byCat.keys()].sort((a, b) => {
      const ia = order.indexOf(a);
      const ib = order.indexOf(b);
      return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
    });
    return cats.map((cat) => ({ cat, entries: byCat.get(cat)! }));
  }, [data, query]);

  return (
    <div className="space-y-3">
      <div className="relative">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          className="h-9 pl-8"
          placeholder="Search data sources…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />
      </div>

      {isLoading && (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2" aria-label="Loading data sources">
          {Array.from({ length: 6 }).map((_, i) => (
            <div key={i} className="flex items-start gap-3 rounded-lg border p-3">
              <div className="h-9 w-9 shrink-0 animate-pulse rounded-md bg-muted" />
              <div className="min-w-0 flex-1 space-y-2 py-0.5">
                <div className="h-3 w-24 animate-pulse rounded bg-muted" />
                <div className="h-2.5 w-full animate-pulse rounded bg-muted/70" />
                <div className="h-2.5 w-2/3 animate-pulse rounded bg-muted/70" />
              </div>
            </div>
          ))}
        </div>
      )}
      {isError && (
        <div className="py-12 text-center text-sm text-destructive">
          Failed to load catalog: {(error as Error)?.message}
        </div>
      )}
      {!isLoading && !isError && groups.length === 0 && (
        <div className="py-12 text-center text-sm text-muted-foreground">No data sources match.</div>
      )}

      <div className="max-h-[52vh] space-y-4 overflow-y-auto pr-1">
        {groups.map(({ cat, entries }) => (
          <div key={cat}>
            <p className="mb-1.5 px-0.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
              {categoryLabel(cat)}
            </p>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {entries.map((e) => (
                <CatalogCard
                  key={e.type_id}
                  entry={e}
                  selected={selectedType === e.type_id}
                  onPick={() => e.status === "available" && onPick(e)}
                />
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function CatalogCard({
  entry,
  selected,
  onPick,
}: {
  entry: CatalogEntry;
  selected: boolean;
  onPick: () => void;
}) {
  const available = entry.status === "available";
  return (
    <button
      type="button"
      onClick={onPick}
      disabled={!available}
      title={available ? `Add ${entry.label}` : `${entry.label} — coming soon`}
      className={cn(
        "group relative flex items-start gap-3 rounded-lg border p-3 text-left transition",
        available
          ? "hover:border-primary/50 hover:bg-accent/30"
          : "cursor-default opacity-70",
        selected && "border-primary bg-primary/5",
      )}
    >
      <div
        className={cn(
          "flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-background",
          !available && "grayscale",
        )}
      >
        <BrandIcon brand={entry.brand} size={20} />
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">{entry.label}</span>
          {available ? (
            <span className="shrink-0 rounded-full bg-emerald-500/10 px-1.5 py-px text-[9px] font-semibold uppercase tracking-wide text-emerald-600">
              Live
            </span>
          ) : (
            <span className="shrink-0 rounded-full bg-muted px-1.5 py-px text-[9px] font-semibold uppercase tracking-wide text-muted-foreground">
              Soon
            </span>
          )}
        </div>
        <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{entry.description}</p>
        <p className="mt-1 text-[10px] uppercase tracking-wide text-muted-foreground/70">
          {authLabel(entry.auth_mode)}
        </p>
      </div>
    </button>
  );
}
