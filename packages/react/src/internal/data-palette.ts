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
