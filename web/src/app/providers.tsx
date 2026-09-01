import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { PensieveProvider, pensieveDark, pensieveLight } from "@pensieve-ai/react";
import { router } from "./router";
import { useSession } from "@/sdk/session";
import { sessionGetToken } from "@/sdk/client";
import { useTheme } from "@/lib/theme";

const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false } },
});

/**
 * Bridges the app's session store into PensieveProvider.
 *
 * - When endpoint is empty (logged out) we render children without PensieveProvider
 *   so login/setup routes continue to work without a PensieveClient.
 * - getToken delegates to sessionGetToken which handles single-flight refresh
 *   and dead-session redirect — the transport layer owns auth end to end.
 * - Provider is re-keyed on endpoint+database change so the PensieveClient is
 *   recreated when the user switches servers or databases.
 */
function PensieveProviderBridge({ children }: { children: React.ReactNode }) {
  const endpoint = useSession((s) => s.endpoint);
  const database = useSession((s) => s.database);
  const resolved = useTheme((s) => s.resolved);
  const theme = resolved === "dark" ? pensieveDark : pensieveLight;

  if (!endpoint) {
    return <>{children}</>;
  }

  return (
    <PensieveProvider
      key={`${endpoint}|${database}`}
      endpoint={endpoint}
      auth={{ getToken: sessionGetToken }}
      database={database}
      theme={theme}
      queryClient={queryClient}
      onError={console.error}
    >
      {children}
    </PensieveProvider>
  );
}

export function Providers() {
  return (
    <QueryClientProvider client={queryClient}>
      <PensieveProviderBridge>
        <RouterProvider router={router} />
      </PensieveProviderBridge>
      <ReactQueryDevtools />
    </QueryClientProvider>
  );
}
