import { useMemo, useState } from "react";
import { useSearch } from "@tanstack/react-router";
import { ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { ReviewStatus } from "@/sdk/review";
import { CandidateCard } from "./CandidateCard";
import { useBulkReview, useResolveReview, useReviewItems } from "./useReview";

const STATUS_FILTERS: { value: ReviewStatus | "all"; label: string }[] = [
  { value: "pending", label: "Pending" },
  { value: "auto_applied", label: "Applied (review)" },
  { value: "all", label: "All" },
];

export function ReviewInbox() {
  // Deep-link support: /memory/review?source_run_id=<run> filters to one run.
  const search = useSearch({ strict: false }) as { source_run_id?: string };
  const sourceRunId = search.source_run_id;

  const [status, setStatus] = useState<ReviewStatus | "all">("pending");
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const filter = useMemo(
    () => ({
      status: status === "all" ? undefined : status,
      source_run_id: sourceRunId,
      limit: 200,
    }),
    [status, sourceRunId],
  );

  const { data: items, isLoading, error } = useReviewItems(filter);
  const resolve = useResolveReview();
  const bulk = useBulkReview();
  const busy = resolve.isPending || bulk.isPending;

  const toggleSelect = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const selectedIds = useMemo(() => [...selected], [selected]);
  const clearSelection = () => setSelected(new Set());

  return (
    <div className="flex h-full flex-col">
      {/* Filter strip */}
      <div className="border-b border-border/60 bg-surface/40">
        <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-2 px-6 py-3">
          {STATUS_FILTERS.map((f) => (
            <button
              key={f.value}
              type="button"
              onClick={() => setStatus(f.value)}
              className={cn(
                "rounded-full border px-2.5 py-0.5 text-2xs transition-colors",
                status === f.value
                  ? "border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300"
                  : "border-border bg-background text-muted-foreground hover:bg-accent/50",
              )}
            >
              {f.label}
            </button>
          ))}
          {sourceRunId && (
            <span className="text-2xs text-muted-foreground">filtered to run {sourceRunId.slice(0, 8)}</span>
          )}
        </div>
      </div>

      {/* Bulk action bar */}
      {selectedIds.length > 0 && (
        <div className="border-b border-border/60 bg-violet-500/5">
          <div className="mx-auto flex w-full max-w-3xl items-center gap-2 px-6 py-2">
            <span className="text-xs text-muted-foreground">{selectedIds.length} selected</span>
            <Button
              size="xs"
              onClick={() =>
                bulk.mutate({ ids: selectedIds, action: "approve" }, { onSuccess: clearSelection })
              }
              disabled={busy}
            >
              Approve selected
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() =>
                bulk.mutate({ ids: selectedIds, action: "reject" }, { onSuccess: clearSelection })
              }
              disabled={busy}
            >
              Reject selected
            </Button>
            <Button size="xs" variant="ghost" onClick={clearSelection}>
              Clear
            </Button>
          </div>
        </div>
      )}

      {/* List */}
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-3 px-6 py-4">
          {isLoading && <SkeletonRows />}
          {error && (
            <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 p-3 text-sm text-rose-500">
              {(error as Error).message}
            </div>
          )}
          {!isLoading && !error && (items?.length ?? 0) === 0 && (
            <EmptyState
              icon={ShieldCheck}
              title="Nothing to review"
              description="Memory merges, invalidations, and other gated operations will appear here when your approval policy holds them."
            />
          )}
          {items?.map((item) => (
            <CandidateCard
              key={item.id}
              item={item}
              selected={selected.has(item.id)}
              onToggleSelect={toggleSelect}
              busy={busy}
              onAction={(action, payload) =>
                resolve.mutate({ id: item.id, action, payload })
              }
            />
          ))}
        </div>
      </div>
    </div>
  );
}
