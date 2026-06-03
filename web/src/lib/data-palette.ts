/**
 * kyma data palette — the single source of truth for categorical color.
 *
 * Charts (ECharts), result-grid type badges, memory-type chips, and the graph
 * node colors all draw from here so the product reads as one visual system.
 * Hues are tuned to sit on the dark "synapse" substrate while staying legible
 * in light mode. Brand teal/cyan lead the order so the most common series echo
 * the accent.
 */

/** Ordered categorical palette. Index 0/1 echo the brand teal/cyan. */
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

/** Named anchors, handy for semantic maps below. */
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

/** Memory-type colors (Agent / Memory surfaces). */
export const SEMANTIC: Record<string, string> = {
  fact: PALETTE.cyan,
  decision: PALETTE.violet,
  preference: PALETTE.amber,
  learning: PALETTE.emerald,
  procedure: PALETTE.teal,
  summary: PALETTE.slate,
  entity: PALETTE.rose,
};

/** Result-grid column-type colors. */
export const RESULT_TYPE: Record<string, string> = {
  time: PALETTE.cyan,
  numeric: PALETTE.emerald,
  string: PALETTE.slate,
  bool: PALETTE.violet,
};

/** Stable index into the palette from an arbitrary key (FNV-1a-ish hash). */
export function hashIndex(key: string, mod: number = DATA_PALETTE.length): number {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = ((h << 5) - h + key.charCodeAt(i)) | 0;
  return Math.abs(h) % mod;
}

/** Palette color by ordinal index (wraps). */
export function paletteFor(index: number): string {
  return DATA_PALETTE[((index % DATA_PALETTE.length) + DATA_PALETTE.length) % DATA_PALETTE.length];
}

/** Stable palette color for an arbitrary string key. */
export function colorForKey(key: string): string {
  return DATA_PALETTE[hashIndex(key)];
}

/** Look up a semantic/memory color, falling back to a stable palette hue. */
export function semanticColor(kind: string): string {
  return SEMANTIC[kind?.toLowerCase?.() ?? ""] ?? colorForKey(kind ?? "");
}

/** Add alpha to a #RRGGBB or #RGB hex, returning an rgba() string. */
export function withAlpha(hex: string, alpha: number): string {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/**
 * Read a design token as a usable CSS color. Tokens are stored as raw HSL
 * triples (e.g. "213 18% 17%"); we wrap them in hsl(). Falls back to the given
 * value when no DOM is available (tests/SSR).
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
 * Theme colors for ECharts, sourced from the live design tokens so charts track
 * the app palette automatically. `isDark` only drives the fallbacks.
 */
export function chartTheme(isDark: boolean): ChartTheme {
  return {
    palette: DATA_PALETTE,
    axis: readToken("--muted-foreground", isDark ? "hsl(214 16% 64%)" : "hsl(215 16% 42%)"),
    grid: readToken("--border", isDark ? "hsl(213 18% 17%)" : "hsl(214 28% 90%)"),
    text: readToken("--foreground", isDark ? "hsl(200 24% 96%)" : "hsl(214 32% 14%)"),
    tooltipBg: readToken("--popover", isDark ? "hsl(213 22% 13%)" : "hsl(0 0% 100%)"),
    tooltipBorder: readToken("--border", isDark ? "hsl(213 18% 17%)" : "hsl(214 28% 90%)"),
    tooltipText: readToken("--foreground", isDark ? "hsl(200 24% 96%)" : "hsl(214 32% 14%)"),
  };
}
