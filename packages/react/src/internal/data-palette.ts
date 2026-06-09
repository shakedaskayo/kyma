/**
 * kyma data palette — categorical colors for graph nodes, charts, and badges.
 * Copied from web/src/lib/data-palette.ts (do not import from web).
 */

export const DATA_PALETTE = [
  "#2dd4bf", // teal
  "#38bdf8", // cyan
  "#a78bfa", // violet
  "#fbbf24", // amber
  "#34d399", // emerald
  "#fb7185", // rose
  "#818cf8", // indigo
  "#94a3b8", // slate
] as const;

export type DataColor = (typeof DATA_PALETTE)[number];

export const PALETTE = {
  teal: "#2dd4bf",
  cyan: "#38bdf8",
  violet: "#a78bfa",
  amber: "#fbbf24",
  emerald: "#34d399",
  rose: "#fb7185",
  indigo: "#818cf8",
  slate: "#94a3b8",
} as const;

export function hashIndex(key: string, mod: number = DATA_PALETTE.length): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = ((h << 5) - h + key.charCodeAt(i)) | 0;
  return Math.abs(h) % mod;
}

export function paletteFor(index: number): string {
  return DATA_PALETTE[((index % DATA_PALETTE.length) + DATA_PALETTE.length) % DATA_PALETTE.length];
}

export function colorForKey(key: string): string {
  return DATA_PALETTE[hashIndex(key)];
}

/**
 * Read a CSS custom property as a usable color. The SDK emits tokens as raw
 * HSL triples (e.g. "213 18% 17%"); we wrap them in hsl(). Falls back to the
 * given value when DOM is unavailable (tests/SSR).
 */
export function readToken(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return raw ? `hsl(${raw})` : fallback;
}

export interface ChartTheme {
  palette: readonly string[];
  axis: string;
  grid: string;
  text: string;
  tooltipBg: string;
  tooltipBorder: string;
  tooltipText: string;
}

/**
 * Theme colors for ECharts. Maps Kyma CSS custom properties so charts track
 * the embedded SDK palette automatically. `isDark` only drives fallbacks.
 */
export function chartTheme(isDark: boolean): ChartTheme {
  return {
    palette: DATA_PALETTE,
    axis: readToken("--kyma-muted-foreground", isDark ? "hsl(214 16% 64%)" : "hsl(215 16% 42%)"),
    grid: readToken("--kyma-border", isDark ? "hsl(213 18% 17%)" : "hsl(214 28% 90%)"),
    text: readToken("--kyma-foreground", isDark ? "hsl(200 24% 96%)" : "hsl(214 32% 14%)"),
    tooltipBg: readToken("--kyma-popover", isDark ? "hsl(213 22% 13%)" : "hsl(0 0% 100%)"),
    tooltipBorder: readToken("--kyma-border", isDark ? "hsl(213 18% 17%)" : "hsl(214 28% 90%)"),
    tooltipText: readToken("--kyma-foreground", isDark ? "hsl(200 24% 96%)" : "hsl(214 32% 14%)"),
  };
}
