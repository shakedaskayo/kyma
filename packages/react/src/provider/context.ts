import { createContext, useContext } from "react";
import type { KymaClient } from "@kyma-ai/client";
import type { KymaTheme } from "../theme/tokens";

export interface KymaContextValue {
  client: KymaClient;
  theme: Required<KymaTheme>;
  /**
   * True when the current theme is dark (background lightness < 50%).
   * Used by graph canvas and other visual components to adjust rendering.
   */
  isDark: boolean;
  /** Element Radix portals must target so they stay inside .kyma-root vars. */
  portalContainer: HTMLElement | null;
  onError?: (err: unknown) => void;
}

/**
 * Derive dark mode from the background HSL string ("L H% B%").
 * A lightness value below 50 indicates a dark theme.
 */
export function isDarkTheme(theme: Required<KymaTheme>): boolean {
  // HSL format: "213 26% 7%" — third token is lightness%.
  const parts = theme.background.trim().split(/\s+/);
  const lightness = parseFloat(parts[2] ?? "100");
  return lightness < 50;
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
