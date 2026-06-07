import { useQuery } from "@tanstack/react-query";
import { useSession } from "@/sdk/session";
import { fetchMemoryOverview, type MemoryOverview } from "@/sdk/memory";

/**
 * The memory overview snapshot (store counts/recent, firehose aggregates,
 * pipeline runs). Polls every 5s so the store-at-a-glance + dreaming status
 * stay current alongside the live event pulse.
 */
export function useMemoryOverview() {
  const { endpoint, token } = useSession();
  return useQuery<MemoryOverview>({
    queryKey: ["memory-overview", endpoint],
    queryFn: () => fetchMemoryOverview({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    refetchInterval: 5000,
    staleTime: 2500,
  });
}
