import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { KymaProvider, kymaDark, kymaLight } from "@kyma-ai/react";
import { router } from "./router";
import { useSession } from "@/sdk/session";
import { sessionGetToken } from "@/sdk/client";
import { useTheme } from "@/lib/theme";

const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false } },
});

/**
 * Bridges the app's session store into KymaProvider.
 *
 * - When endpoint is empty (logged out) we render children without KymaProvider
 *   so login/setup routes continue to work without a KymaClient.
 * - getToken reads the live Zustand store at call time so token rotation by
 *   installAuthFetch stays effective without recreating the client.
 * - Provider is re-keyed on endpoint+database change so the KymaClient is
 *   recreated when the user switches servers or databases.
 */
function KymaProviderBridge({ children }: { children: React.ReactNode }) {
  const endpoint = useSession((s) => s.endpoint);
  const database = useSession((s) => s.database);
  const resolved = useTheme((s) => s.resolved);
  const theme = resolved === "dark" ? kymaDark : kymaLight;

  if (!endpoint) {
    return <>{children}</>;
  }

  return (
    <KymaProvider
      key={`${endpoint}|${database}`}
      endpoint={endpoint}
      auth={{ getToken: sessionGetToken }}
      database={database}
      theme={theme}
      queryClient={queryClient}
      onError={console.error}
    >
      {children}
    </KymaProvider>
  );
}

export function Providers() {
  return (
    <QueryClientProvider client={queryClient}>
      <KymaProviderBridge>
        <RouterProvider router={router} />
      </KymaProviderBridge>
      <ReactQueryDevtools />
    </QueryClientProvider>
  );
}
