import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Moon, Settings2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { useSession } from "@/sdk/session";
import { getMemorySettings, type DreamingSettings } from "@/sdk/memory";
import type { RunKind, RunStatus } from "@/sdk/dreaming";
import { cn } from "@/lib/utils";
import { useDreamingRuns, useTriggerDreaming } from "./useDreaming";
import { RunRow } from "./RunRow";

type KindFilter = "all" | RunKind;
type StatusFilter = "all" | RunStatus;

const PAGE = 20;

const STATUS_FILTERS: StatusFilter[] = ["all", "running", "success", "error", "skipped"];

export function RunsList({ selectedId }: { selectedId: string | null }) {
  const { endpoint, token } = useSession();
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [limit, setLimit] = useState(PAGE);

  const { data: settings } = useQuery({
    queryKey: ["memory-settings", endpoint],
    queryFn: () => getMemorySettings({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 30_000,
  });
  const dreaming: DreamingSettings | undefined = settings?.dreaming;

  const kindArg = kindFilter === "all" ? undefined : kindFilter;
  const { data: runs, isLoading, error } = useDreamingRuns({ kind: kindArg, limit });
  const trigger = useTriggerDreaming();

  const filtered = useMemo(
    () => (runs ?? []).filter((r) => statusFilter === "all" || r.status === statusFilter),
    [runs, statusFilter],
  );

  const hasMore = (runs?.length ?? 0) >= limit;
  const clearFilters = () => {
    setKindFilter("all");
    setStatusFilter("all");
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-wrap items-center gap-1.5 border-b border-border/60 px-3 py-2.5">
        <FilterGroup>
          <Chip active={kindFilter === "all"} onClick={() => setKindFilter("all")}>
            All
          </Chip>
          <Chip active={kindFilter === "dreaming"} onClick={() => setKindFilter("dreaming")}>
            Dreaming
          </Chip>
          <Chip
            active={kindFilter === "consolidation"}
            onClick={() => setKindFilter("consolidation")}
          >
            Consolidation
          </Chip>
        </FilterGroup>
        <span className="mx-1 h-4 w-px bg-border" />
        <FilterGroup>
          {STATUS_FILTERS.map((s) => (
            <Chip key={s} active={statusFilter === s} onClick={() => setStatusFilter(s)}>
              {s === "all" ? "All" : s[0].toUpperCase() + s.slice(1)}
            </Chip>
          ))}
        </FilterGroup>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-3 py-3">
        {isLoading && !runs ? (
          <SkeletonRows rows={6} />
        ) : error ? (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            Failed to load runs: {(error as Error).message}
          </div>
        ) : filtered.length === 0 ? (
          <RunsEmptyState
            dreaming={dreaming}
            hasAnyRuns={(runs?.length ?? 0) > 0}
            onDream={() => trigger.mutate(undefined)}
            onClearFilters={clearFilters}
            triggering={trigger.isPending}
          />
        ) : (
          <>
            <div className="space-y-2">
              {filtered.map((r) => (
                <RunRow key={r.id} run={r} active={r.id === selectedId} />
              ))}
            </div>
            <div className="mt-3 flex items-center justify-between px-1 text-2xs text-muted-foreground">
              <span>
                {filtered.length} run{filtered.length === 1 ? "" : "s"} shown
              </span>
              {hasMore && (
                <Button variant="outline" size="xs" onClick={() => setLimit((l) => l + PAGE)}>
                  Load older runs
                </Button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function RunsEmptyState({
  dreaming,
  hasAnyRuns,
  onDream,
  onClearFilters,
  triggering,
}: {
  dreaming: DreamingSettings | undefined;
  hasAnyRuns: boolean;
  onDream: () => void;
  onClearFilters: () => void;
  triggering: boolean;
}) {
  if (hasAnyRuns) {
    return (
      <EmptyState
        icon={Moon}
        title="No runs match these filters"
        description="Try a different kind or status filter."
        action={
          <Button variant="outline" onClick={onClearFilters}>
            Clear filters
          </Button>
        }
      />
    );
  }
  if (dreaming && !dreaming.enabled) {
    return (
      <EmptyState
        icon={Moon}
        title="The engine is asleep"
        description="Enable autonomous consolidation to let the agent distill, merge, and link memories on a schedule — or wake it once now."
        action={
          <div className="flex items-center gap-2">
            <Button
              onClick={onDream}
              disabled={triggering}
              className="bg-violet-600 text-white hover:bg-violet-500"
            >
              <Sparkles className="mr-1.5 h-4 w-4" /> Dream now
            </Button>
            <Button variant="outline" asChild>
              <Link to="/memory/settings">
                <Settings2 className="mr-1.5 h-4 w-4" /> Settings
              </Link>
            </Button>
          </div>
        }
      />
    );
  }
  return (
    <EmptyState
      icon={Moon}
      title="No runs yet"
      description="No dreaming runs have happened yet. Trigger one now, or wait for the next scheduled run."
      action={
        <Button
          onClick={onDream}
          disabled={triggering}
          className="bg-violet-600 text-white hover:bg-violet-500"
        >
          <Sparkles className="mr-1.5 h-4 w-4" /> Dream now
        </Button>
      }
    />
  );
}

function FilterGroup({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-wrap items-center gap-1">{children}</div>;
}

function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full border px-2.5 py-0.5 text-2xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300"
          : "border-border bg-background text-muted-foreground hover:bg-accent/50",
      )}
    >
      {children}
    </button>
  );
}
