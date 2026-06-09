import type { TimeRangePreset } from "@/features/tabs/workspace-store";

export type QueryState = { query: string; preset: TimeRangePreset; from?: string; to?: string };

// Single source of truth for valid presets.  Adding a new value to the
// TimeRangePreset union without updating this set produces a TypeScript error.
const VALID_PRESETS = new Set(
  (["none", "5m", "15m", "1h", "6h", "24h", "7d", "30d", "custom"] as const) satisfies readonly TimeRangePreset[],
);

// NOTE: base64url-encoded JSON for a typical query is ~50–200 bytes, well under
// the ~2000-char safe URL limit. Very large KQL queries (multi-KB) could exceed
// browser URL length limits — that case is out of scope for this implementation.

export function encodeQueryState(s: QueryState): string {
  const json = JSON.stringify(s);
  const utf8 = new TextEncoder().encode(json);
  // Chunk the fromCharCode to avoid Safari's ~65k argument limit on large inputs.
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < utf8.length; i += CHUNK) {
    bin += String.fromCharCode(...utf8.subarray(i, i + CHUNK));
  }
  return btoa(bin)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

export function decodeQueryState(encoded: string): QueryState | null {
  try {
    const b64 = encoded.replace(/-/g, "+").replace(/_/g, "/") + "===".slice(0, (4 - (encoded.length % 4)) % 4);
    const json = atob(b64);
    const v = JSON.parse(json);
    if (!v || typeof v.query !== "string" || typeof v.preset !== "string") return null;
    if (!VALID_PRESETS.has(v.preset as TimeRangePreset)) return null;
    if (v.from !== undefined && typeof v.from !== "string") return null;
    if (v.to   !== undefined && typeof v.to   !== "string") return null;
    return v as QueryState;
  } catch {
    return null;
  }
}
