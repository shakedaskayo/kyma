import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  bulkReview,
  listReview,
  resolveReview,
  reviewCount,
  type ReviewAction,
  type ReviewFilter,
  type ReviewItem,
} from "@/sdk/review";

// Query keys scoped to (endpoint, database) — never the bearer.
const reviewKey = (endpoint: string, database: string, filter?: ReviewFilter) =>
  ["review", "items", endpoint, database, JSON.stringify(filter ?? {})] as const;
const countKey = (endpoint: string, database: string) =>
  ["review", "count", endpoint, database] as const;

/** Pending/post-hoc counts for the nav badge. Polls every 15s. */
export function useReviewCount() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: countKey(endpoint, database),
    queryFn: () => reviewCount({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    refetchInterval: 15000,
    staleTime: 5000,
  });
}

/** The review queue, filtered. Defaults to pending items. */
export function useReviewItems(filter?: ReviewFilter) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: reviewKey(endpoint, database, filter),
    queryFn: () => listReview({ endpoint, token, database, ...filter }),
    enabled: Boolean(endpoint),
    refetchInterval: 15000,
    staleTime: 3000,
  });
}

function invalidateReview(qc: ReturnType<typeof useQueryClient>, endpoint: string, database: string) {
  void qc.invalidateQueries({ queryKey: ["review", "items", endpoint, database] });
  void qc.invalidateQueries({ queryKey: countKey(endpoint, database) });
}

/** Approve / reject / undo a single queue row. */
export function useResolveReview() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: {
      id: string;
      action: ReviewAction;
      payload?: Record<string, unknown>;
      comment?: string;
    }) => resolveReview({ endpoint, token, database, ...vars }),
    onSuccess: (_res, vars) => {
      invalidateReview(qc, endpoint, database);
      const verb =
        vars.action === "approve" ? "Approved" : vars.action === "reject" ? "Rejected" : "Reverted";
      toast.success(`${verb} memory ${vars.action === "undo" ? "change" : "candidate"}.`);
    },
    onError: (e) => toast.error(`Could not resolve: ${(e as Error).message}`),
  });
}

/** Apply one action across many rows. */
export function useBulkReview() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { ids: string[]; action: ReviewAction; comment?: string }) =>
      bulkReview({ endpoint, token, database, ...vars }),
    onSuccess: (res) => {
      invalidateReview(qc, endpoint, database);
      const ok = res.results.filter((r) => r.ok).length;
      const failed = res.results.length - ok;
      toast.success(`${ok} resolved${failed ? `, ${failed} failed` : ""}.`);
    },
    onError: (e) => toast.error(`Bulk action failed: ${(e as Error).message}`),
  });
}

export type { ReviewItem };
