import { Link } from "@tanstack/react-router";
import { Sparkles } from "lucide-react";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonCard } from "@/components/ui/skeleton";
import type { MemorySourceSummary } from "@/sdk/memory";
import { useMemorySourceSummary } from "./useDataSources";

interface SourceGroup {
  source: string;
  total: number;
  realms: { realm: string; count: number }[];
}

/** Group rows by source, realms sorted by count desc within each group. */
export function groupBySource(rows: MemorySourceSummary[]): SourceGroup[] {
  const bySource = new Map<string, SourceGroup>();
  for (const row of rows) {
    let g = bySource.get(row.source);
    if (!g) {
      g = { source: row.source, total: 0, realms: [] };
      bySource.set(row.source, g);
    }
    g.total += row.count;
    g.realms.push({ realm: row.realm, count: row.count });
  }
  const groups = [...bySource.values()];
  for (const g of groups) g.realms.sort((a, b) => b.count - a.count);
  groups.sort((a, b) => b.total - a.total);
  return groups;
}

function SourceCard({ group }: { group: SourceGroup }) {
  return (
    <div className="rounded-lg border bg-background px-4 py-3 shadow-sm">
      <div className="flex items-baseline justify-between gap-2">
        <span className="font-medium text-foreground">{group.source}</span>
        <span className="text-sm tabular-nums text-muted-foreground">
          {group.total.toLocaleString()} {group.total === 1 ? "memory" : "memories"}
        </span>
      </div>
      <div className="mt-2 flex flex-col gap-1">
        {group.realms.map((r) => (
          <div
            key={r.realm}
            className="flex items-center justify-between gap-2 text-xs text-muted-foreground"
          >
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground">
              {r.realm}
            </code>
            <span className="tabular-nums">{r.count.toLocaleString()}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function MemoriesSummaryTab() {
  const { data: rows, isLoading, error } = useMemorySourceSummary();

  if (isLoading) {
    return (
      <div className="mx-auto flex max-w-3xl flex-col gap-3 py-2">
        <SkeletonCard />
        <SkeletonCard />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load memory summary: {(error as Error).message}
      </div>
    );
  }

  if (!rows || rows.length === 0) {
    return (
      <EmptyState
        icon={Sparkles}
        title="No memories yet"
        description="Once watchers, agents, or dreaming start forming memories, this tab shows where each one came from."
      />
    );
  }

  const groups = groupBySource(rows);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-3">
      <p className="text-sm text-muted-foreground">
        Where memories come from. The full store lives in{" "}
        <Link to="/memory" className="text-primary underline-offset-2 hover:underline">
          Memory →
        </Link>
      </p>
      {groups.map((g) => (
        <SourceCard key={g.source} group={g} />
      ))}
    </div>
  );
}
