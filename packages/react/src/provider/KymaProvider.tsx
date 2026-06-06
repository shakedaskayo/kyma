import { useMemo, useRef, useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createKymaClient, type KymaAuth } from "@kyma-ai/client";
import { KymaContext } from "./context";
import { themeToCssVars, type KymaTheme } from "../theme/tokens";
import { kymaDark } from "../theme/presets";

export interface KymaProviderProps {
  endpoint: string;
  auth: KymaAuth;
  database?: string;
  /** Preset or partial override; merged over kymaDark. "inherit" maps host vars. */
  theme?: Partial<KymaTheme> | "inherit";
  /** Reuse the host's React Query client; otherwise an isolated one is created. */
  queryClient?: QueryClient;
  onError?: (err: unknown) => void;
  children: ReactNode;
}

export function KymaProvider(props: KymaProviderProps) {
  const { endpoint, auth, database, theme, queryClient, onError, children } = props;

  const client = useMemo(
    () => createKymaClient({ endpoint, auth, database }),
    // auth is intentionally captured once per endpoint/database — token
    // rotation happens via getToken, not by re-creating the client.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [endpoint, database],
  );

  const ownQueryClient = useRef<QueryClient>();
  if (!queryClient && !ownQueryClient.current) {
    ownQueryClient.current = new QueryClient({
      defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 } },
    });
  }
  const qc = queryClient ?? ownQueryClient.current!;

  const resolvedTheme = useMemo(() => {
    if (theme === "inherit") return { ...kymaDark }; // vars come from host CSS; preset values are fallbacks
    return { ...kymaDark, ...theme };
  }, [theme]);

  // When theme === "inherit" we intentionally set NO inline vars so the host's
  // own --kyma-* custom properties (set elsewhere in their CSS) are respected.
  // Inline style values would override them at higher specificity.
  const cssVars = useMemo(
    () => (theme === "inherit" ? {} : themeToCssVars(resolvedTheme)),
    [theme, resolvedTheme],
  );

  const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(null);

  const value = useMemo(
    () => ({ client, theme: resolvedTheme as Required<KymaTheme>, portalContainer, onError }),
    [client, resolvedTheme, portalContainer, onError],
  );

  return (
    <KymaContext.Provider value={value}>
      <QueryClientProvider client={qc}>
        <div className="kyma-root" style={cssVars as React.CSSProperties}>
          {children}
          {/* Portal target carrying the same class + vars so Radix popovers/
              dialogs keep their theming when they escape the DOM subtree. */}
          <div
            ref={setPortalContainer}
            className="kyma-root"
            style={cssVars as React.CSSProperties}
          />
        </div>
      </QueryClientProvider>
    </KymaContext.Provider>
  );
}
