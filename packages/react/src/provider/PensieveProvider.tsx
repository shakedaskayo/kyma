import { useMemo, useRef, useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createPensieveClient, type PensieveAuth } from "@pensieve-ai/client";
import { PensieveContext, isDarkTheme } from "./context";
import { themeToCssVars, type PensieveTheme } from "../theme/tokens";
import { pensieveDark } from "../theme/presets";

export interface PensieveProviderProps {
  endpoint: string;
  auth: PensieveAuth;
  database?: string;
  /** Preset or partial override; merged over pensieveDark. "inherit" maps host vars. */
  theme?: Partial<PensieveTheme> | "inherit";
  /** Reuse the host's React Query client; otherwise an isolated one is created. */
  queryClient?: QueryClient;
  onError?: (err: unknown) => void;
  children: ReactNode;
}

export function PensieveProvider(props: PensieveProviderProps) {
  const { endpoint, auth, database, theme, queryClient, onError, children } = props;

  const client = useMemo(
    () => createPensieveClient({ endpoint, auth, database }),
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
    if (theme === "inherit") return { ...pensieveDark }; // vars come from host CSS; preset values are fallbacks
    return { ...pensieveDark, ...theme };
  }, [theme]);

  // When theme === "inherit" we intentionally set NO inline vars so the host's
  // own --pensieve-* custom properties (set elsewhere in their CSS) are respected.
  // Inline style values would override them at higher specificity.
  const cssVars = useMemo(
    () => (theme === "inherit" ? {} : themeToCssVars(resolvedTheme)),
    [theme, resolvedTheme],
  );

  const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(null);

  const value = useMemo(
    () => ({
      client,
      theme: resolvedTheme as Required<PensieveTheme>,
      isDark: isDarkTheme(resolvedTheme as Required<PensieveTheme>),
      portalContainer,
      onError,
    }),
    [client, resolvedTheme, portalContainer, onError],
  );

  return (
    <PensieveContext.Provider value={value}>
      <QueryClientProvider client={qc}>
        <div className="pensieve-root" style={cssVars as React.CSSProperties}>
          {children}
          {/* Portal target carrying the same class + vars so Radix popovers/
              dialogs keep their theming when they escape the DOM subtree. */}
          <div
            ref={setPortalContainer}
            className="pensieve-root"
            style={cssVars as React.CSSProperties}
          />
        </div>
      </QueryClientProvider>
    </PensieveContext.Provider>
  );
}
