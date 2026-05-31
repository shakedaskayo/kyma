import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  createConnector,
  deleteConnector,
  getConnector,
  getConnectorCatalog,
  listConnectors,
  listGitHubRepos,
  patchConnector,
  pauseConnector,
  resumeConnector,
  triggerConnector,
  type ConnectorUpdate,
  type CreateConnectorBody,
} from "@/sdk/connectors";

// Query keys are scoped to (endpoint, database) — never the bearer or any PAT.
const listKey = (endpoint: string, database: string) =>
  ["connectors", endpoint, database] as const;
const detailKey = (endpoint: string, database: string, id: string) =>
  ["connectors", endpoint, database, id] as const;

/**
 * The engine-driven connector catalog (available + coming-soon vendors). Cached
 * aggressively — it's static metadata that only changes on an engine deploy.
 */
export function useConnectorCatalog() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: ["connectors", "catalog", endpoint],
    queryFn: () => getConnectorCatalog({ endpoint, token, database }),
    enabled: Boolean(endpoint),
    staleTime: 10 * 60_000,
  });
}

export function useConnectors() {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: listKey(endpoint, database),
    queryFn: () => listConnectors({ endpoint, token, database }),
    // Token optional — gate only on the endpoint (matches the graph hooks).
    enabled: Boolean(endpoint),
    staleTime: 30_000,
  });
}

/** Lazy per-row hydration of metrics/status (list carries none). */
export function useConnector(id: string | null) {
  const { endpoint, token, database } = useSession();
  return useQuery({
    queryKey: detailKey(endpoint, database, id ?? ""),
    queryFn: () => getConnector({ endpoint, token, database, id: id! }),
    enabled: Boolean(endpoint && id),
    staleTime: 15_000,
  });
}

export function useCreateConnector() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateConnectorBody) =>
      createConnector({ endpoint, token, database, body }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      toast.success("Connector created.");
    },
    onError: (e) => toast.error(`Create failed: ${(e as Error).message}`),
  });
}

export function usePatchConnector(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: ConnectorUpdate) =>
      patchConnector({ endpoint, token, database, id, patch }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Connector updated.");
    },
    onError: (e) => toast.error(`Update failed: ${(e as Error).message}`),
  });
}

export function usePauseConnector(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => pauseConnector({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Connector paused.");
    },
    onError: (e) => toast.error(`Pause failed: ${(e as Error).message}`),
  });
}

export function useResumeConnector(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => resumeConnector({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Connector resumed.");
    },
    onError: (e) => toast.error(`Resume failed: ${(e as Error).message}`),
  });
}

export function useTriggerConnector(id: string) {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => triggerConnector({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: detailKey(endpoint, database, id) });
      toast.success("Sync triggered.");
    },
    onError: (e) => toast.error(`Trigger failed: ${(e as Error).message}`),
  });
}

export function useDeleteConnector() {
  const { endpoint, token, database } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteConnector({ endpoint, token, database, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint, database) });
      toast.success("Connector deleted.");
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
