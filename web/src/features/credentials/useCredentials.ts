import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  createCredential,
  deleteCredential,
  listCredentials,
  type CreateCredentialBody,
} from "@/sdk/credentials";

const listKey = (endpoint: string) => ["credentials", endpoint] as const;

export function useCredentials() {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: listKey(endpoint),
    queryFn: () => listCredentials({ endpoint, token }),
    enabled: Boolean(endpoint),
    staleTime: 30_000,
  });
}

export function useCreateCredential() {
  const { endpoint, token } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateCredentialBody) => createCredential({ endpoint, token, body }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint) });
      toast.success("Credential saved.");
    },
    onError: (e) => toast.error(`Save failed: ${(e as Error).message}`),
  });
}

export function useDeleteCredential() {
  const { endpoint, token } = useSession();
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteCredential({ endpoint, token, id }),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: listKey(endpoint) });
      toast.success("Credential deleted.");
    },
    onError: (e) => toast.error(`Delete failed: ${(e as Error).message}`),
  });
}
