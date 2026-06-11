import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  createDataSource,
  deleteDataSource,
  getDataSource,
  getDataSourceCatalog,
  listDataSources,
  listGitHubRepos,
  patchDataSource,
  pauseDataSource,
  resumeDataSource,
  triggerDataSource,
  type DataSourceUpdate,
  type CreateDataSourceBody,
} from "@/sdk/datasources";

// Query keys are scoped to (endpoint, database) — never the bearer or any PAT.
const listKey = (endpoint: string, database: string) =>
  ["data-sources", endpoint, database] as const;
const detailKey = (endpoint: string, database: string, id: string) =>
  ["data-sources", endpoint, database, id] as const;

/**
 * The engine-driven data source catalog (available + coming-soon vendors). Cached
 * aggressively — it's static metadata that only changes on an engine deploy.
 */
export function useDataSourceCatalog() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["data-sources", "catalog", endpoint],
    queryFn: () => getDataSourceCatalog({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    staleTime: 10 * 60_000,
  });
}

export function useDataSources() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: listKey(endpoint, database),
    queryFn: () => listDataSources({ endpoint, token, database }),
    // Token optional — gate only on the endpoint (matches the graph hooks).
    enabled: Boolean(endpoint),
    staleTime: 30_000,
  });
}

/** Lazy per-row hydration of metrics/status (list carries none). */
export function useDataSource(id: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: detailKey(endpoint, database, id ?? ""),
    queryFn: () => getDataSource({ endpoint, token, database, id: id! }),
    enabled: Boolean(endpoint && id),
    staleTime: 15_000,
  });
}

export function useCreateDataSource() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateDataSourceBody) =>
      createDataSource({ endpoint, token, database, body }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      toast.success("Data source created.");
    },
    onError: (e) => toast.error(`Create failed: ${(e as Error).message}`),
  });
}

export function usePatchDataSource(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: DataSourceUpdate) =>
      patchDataSource({ endpoint, token, database, id, patch }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Data source updated.");
    },
    onError: (e) => toast.error(`Update failed: ${(e as Error).message}`),
  });
}

export function usePauseDataSource(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => pauseDataSource({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Data source paused.");
    },
    onError: (e) => toast.error(`Pause failed: ${(e as Error).message}`),
  });
}

export function useResumeDataSource(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => resumeDataSource({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Data source resumed.");
    },
    onError: (e) => toast.error(`Resume failed: ${(e as Error).message}`),
  });
}

export function useTriggerDataSource(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => triggerDataSource({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Sync triggered.");
    },
    onError: (e) => toast.error(`Trigger failed: ${(e as Error).message}`),
  });
}

export function useDeleteDataSource() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteDataSource({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      toast.success("Data source deleted.");
    },
    onError: (e) => toast.error(`Delete failed: ${(e as Error).message}`),
  });
}

/**
 * Imperative repo fetch for the wizard. Returned via useMutation so the PAT is
 * passed as a call argument and never stored in a react-query cache key.
 */
export function useGitHubRepos() {
  const { endpoint, token, database } = useSession();
  return useMutation({
    mutationFn: (pat: string) => listGitHubRepos({ endpoint, token, database, pat }),
    onError: (e) => toast.error(`Could not fetch repos: ${(e as Error).message}`),
  });
}
