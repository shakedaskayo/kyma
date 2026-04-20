import type { TimeRangePreset } from "@/features/tabs/workspace-store";

export type QueryState = { query: string; preset: TimeRangePreset; from?: string; to?: string };

// NOTE: base64url-encoded JSON for a typical query is ~50–200 bytes, well under
// the ~2000-char safe URL limit. Very large KQL queries (multi-KB) could exceed
// browser URL length limits — that case is out of scope for this implementation.

export function encodeQueryState(s: QueryState): string {
  const json = JSON.stringify(s);
  const utf8 = new TextEncoder().encode(json);
  // base64url, no padding
  return btoa(String.fromCharCode(...utf8))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

export function decodeQueryState(encoded: string): QueryState | null {
  try {
    const b64 =
      encoded.replace(/-/g, "+").replace(/_/g, "/") +
      "===".slice(0, (4 - (encoded.length % 4)) % 4);
    const json = atob(b64);
    return JSON.parse(json) as QueryState;
  } catch {
    return null;
  }
}
