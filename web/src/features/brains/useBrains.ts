import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  createBrain,
  deleteBrain,
  getBrain,
  getBrainFile,
  getBrainTree,
  listBrains,
  triggerBrainExport,
  triggerBrainGardener,
  updateBrain,
  type BrainConfig,
  type CreateBrainInput,
} from "@/sdk/brains";

// Query keys scoped to (endpoint, database) — never the bearer.
const brainsKey = (endpoint: string, database: string) =>
  ["brains", endpoint, database] as const;
const brainKey = (endpoint: string, database: string, name: string) =>
  ["brains", endpoint, database, name] as const;

export function useBrains() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: brainsKey(endpoint, database),
    queryFn: () => listBrains({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    refetchInterval: 30_000,
    staleTime: 5_000,
  });
}

export function useBrain(name: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: brainKey(endpoint, database, name ?? ""),
    queryFn: () => getBrain({ endpoint, token, database, name: name! }),
    enabled: Boolean(endpoint && name),
    refetchInterval: 15_000,
  });
}

export function useCreateBrain() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateBrainInput) => createBrain({ endpoint, token, database, input }),
    onSuccess: (resp) => {
      void qc.invalidateQueries({ queryKey: brainsKey(endpoint, database) });
      const fe = resp.first_export;
      toast.success(
        fe?.notes != null
          ? `Brain published — ${fe.notes} notes exported`
          : "Brain published",
      );
    },
    onError: (e) => toast.error(`Publish failed: ${(e as Error).message}`),
  });
}

export function useUpdateBrain(name: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (
      patch: Partial<Pick<BrainConfig, "export_interval_secs" | "gardener" | "visibility_role">>,
    ) => updateBrain({ endpoint, token, database, name, patch }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["brains", endpoint, database] });
      toast.success("Brain updated");
    },
    onError: (e) => toast.error(`Update failed: ${(e as Error).message}`),
  });
}

export function useDeleteBrain() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteBrain({ endpoint, token, database, name, purge: true }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: brainsKey(endpoint, database) });
      toast.success("Brain deleted — memories untouched");
    },
    onError: (e) => toast.error(`Delete failed: ${(e as Error).message}`),
  });
}

export function useTriggerExport(name: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => triggerBrainExport({ endpoint, token, database, name }),
    onSuccess: (r) => {
      void qc.invalidateQueries({ queryKey: ["brains", endpoint, database] });
      toast.success(
        r.noop ? "No changes — vault already up to date" : `Exported ${r.notes ?? 0} notes`,
      );
    },
    onError: (e) => toast.error(`Export failed: ${(e as Error).message}`),
  });
}

export function useBrainTree(name: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["brains", "tree", endpoint, database, name ?? ""],
    queryFn: () => getBrainTree({ endpoint, token, database, name: name! }),
    enabled: Boolean(endpoint && name),
    refetchInterval: 30_000,
  });
}

export function useBrainFile(name: string | null, path: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["brains", "file", endpoint, database, name ?? "", path ?? ""],
    queryFn: () => getBrainFile({ endpoint, token, database, name: name!, path: path! }),
    enabled: Boolean(endpoint && name && path),
    staleTime: 10_000,
  });
}

export function useTriggerGardener(name: string) {
  const { endpoint, token, database } = useSession();
  return useMutation({
    mutationFn: () => triggerBrainGardener({ endpoint, token, database, name }),
    onSuccess: (r) => {
      if (r.deduped) toast.info("A dreaming run is already in flight — try again later");
      else toast.success("Gardener run started — see Memory → Dreaming");
    },
    onError: (e) => toast.error(`Gardener failed: ${(e as Error).message}`),
  });
}
