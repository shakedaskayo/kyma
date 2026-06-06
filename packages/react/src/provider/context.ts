import { createContext, useContext } from "react";
import type { KymaClient } from "@kyma-ai/client";
import type { KymaTheme } from "../theme/tokens";

export interface KymaContextValue {
  client: KymaClient;
  theme: Required<KymaTheme>;
  /** Element Radix portals must target so they stay inside .kyma-root vars. */
  portalContainer: HTMLElement | null;
  onError?: (err: unknown) => void;
}

export const KymaContext = createContext<KymaContextValue | null>(null);

export function useKymaContext(): KymaContextValue {
  const ctx = useContext(KymaContext);
  if (!ctx) throw new Error("Kyma components must be rendered inside <KymaProvider>");
  return ctx;
}

export function useKymaClient() {
  return useKymaContext().client;
}
