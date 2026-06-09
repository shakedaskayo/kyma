import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  getAgentRunTrace,
  getDreamingRun,
  listDreamingRuns,
  listWorkers,
  triggerDreamingRun,
  type Run,
  type RunKind,
} from "@/sdk/dreaming";

// Query keys are scoped to (endpoint, database) — never the bearer.
const runsKey = (endpoint: string, database: string, kind?: RunKind) =>
  ["dreaming", "runs", endpoint, database, kind ?? "all"] as const;
const runKey = (endpoint: string, database: string, id: string) =>
  ["dreaming", "run", endpoint, database, id] as const;
const workersKey = (endpoint: string, database: string) =>
  ["dreaming", "workers", endpoint, database] as const;
const traceKey = (endpoint: string, database: string, agentRunId: string) =>
  ["dreaming", "trace", endpoint, database, agentRunId] as const;

const anyRunning = (runs: Run[] | undefined): boolean =>
  Boolean(runs?.some((r) => r.status === "running"));

/**
 * The dreaming runs list. Polls fast (3s) whenever any run is in flight so the
 * live activity / progress stays fresh, and slow (15s) otherwise.
 */
export function useDreamingRuns(opts?: { kind?: RunKind; limit?: number; offset?: number }) {
  const { endpoint, token, database } = useSession();
  const kind = opts?.kind;
  return useQuery({
    queryKey: [...runsKey(endpoint, database, kind), opts?.limit ?? 0, opts?.offset ?? 0],
    queryFn: () =>
      listDreamingRuns({ endpoint, token, database, kind, limit: opts?.limit, offset: opts?.offset }),
    enabled: Boolean(endpoint),
    refetchInterval: (q) => (anyRunning(q.state.data as Run[] | undefined) ? 3000 : 15000),
    staleTime: 2000,
  });
}

/** One run. Polls 2.5s while running, off once it settles. */
export function useDreamingRun(id: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: runKey(endpoint, database, id ?? ""),
    queryFn: () => getDreamingRun({ endpoint, token, database, id: id! }),
    enabled: Boolean(endpoint && id),
    refetchInterval: (q) =>
      (q.state.data as Run | undefined)?.status === "running" ? 2500 : false,
  });
}

export function useTriggerDreaming() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars?: { mode?: string; focus?: string }) =>
      triggerDreamingRun({ endpoint, token, database, ...vars }),
    onSuccess: (res) => {
      void qc.invalidateQueries({ queryKey: ["dreaming", "runs", endpoint, database] });
      if (res.job_id == null) {
        toast.message("A dreaming run is already in flight.", {
          description: res.detail ?? undefined,
        });
      } else {
        toast.success("Dreaming run started.");
      }
    },
    onError: (e) => toast.error(`Could not start dreaming: ${(e as Error).message}`),
  });
}

/** The worker registry. Polls every 10s. */
export function useWorkers() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: workersKey(endpoint, database),
    queryFn: () => listWorkers({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    refetchInterval: 10000,
    staleTime: 5000,
  });
}

/**
 * The recorded agent-run trace backing a run's conversation drilldown. Polls
 * 2.5s while the parent run is still running, off otherwise.
 */
export function useAgentRunTrace(agentRunId: string | null, parentRunning: boolean) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: traceKey(endpoint, database, agentRunId ?? ""),
    queryFn: () => getAgentRunTrace({ endpoint, token, database, agentRunId: agentRunId! }),
    enabled: Boolean(endpoint && agentRunId),
    refetchInterval: parentRunning ? 2500 : false,
  });
}
