import { createContext, useContext } from "react";
import type { PensieveClient } from "@pensieve-ai/client";
import type { PensieveTheme } from "../theme/tokens";

export interface PensieveContextValue {
  client: PensieveClient;
  theme: Required<PensieveTheme>;
  /**
   * True when the current theme is dark (background lightness < 50%).
   * Used by graph canvas and other visual components to adjust rendering.
   */
  isDark: boolean;
  /** Element Radix portals must target so they stay inside .pensieve-root vars. */
  portalContainer: HTMLElement | null;
  onError?: (err: unknown) => void;
}

/**
 * Derive dark mode from the background HSL string ("L H% B%").
 * A lightness value below 50 indicates a dark theme.
 */
export function isDarkTheme(theme: Required<PensieveTheme>): boolean {
  // HSL format: "213 26% 7%" — third token is lightness%.
  const parts = theme.background.trim().split(/\s+/);
  const lightness = parseFloat(parts[2] ?? "100");
  return lightness < 50;
}

export const PensieveContext = createContext<PensieveContextValue | null>(null);

export function usePensieveContext(): PensieveContextValue {
  const ctx = useContext(PensieveContext);
  if (!ctx) throw new Error("Pensieve components must be rendered inside <PensieveProvider>");
  return ctx;
}

export function usePensieveClient() {
  return usePensieveContext().client;
}
