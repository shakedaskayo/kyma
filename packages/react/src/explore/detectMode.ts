/**
 * Auto-detect whether an Explore input is a keyword search (Discover grammar)
 * or a raw query (KQL/SQL). Keeps the single smart input unambiguous in the
 * common cases and lets the caller surface (and let the user override) the
 * detected mode for the ambiguous ones.
 */

export type ExploreMode = "search" | "kql" | "sql";

const SQL_LEADING = /^\s*(select|with|show|explain)\b/i;
const FIRST_IDENT = /^\s*([A-Za-z_][A-Za-z0-9_]*)/;

/**
 * Decide the mode for `input`, given the set of known table names (used to
 * recognize a bare `table` / `table | …` as KQL rather than a keyword).
 *
 * Rules, in order:
 *   1. empty → "search"
 *   2. starts with SELECT/WITH/SHOW/EXPLAIN → "sql"
 *   3. contains a `|` pipe → "kql"
 *   4. first word is a known table name, and the input is either just that word
 *      or that word followed by `|` / a KQL operator → "kql"
 *   5. otherwise → "search"
 */
export function detectMode(input: string, tableNames: Iterable<string> = []): ExploreMode {
  const t = input.trim();
  if (t === "") return "search";
  if (SQL_LEADING.test(t)) return "sql";
  if (t.includes("|")) return "kql";

  const first = FIRST_IDENT.exec(t)?.[1];
  if (first) {
    const tables = new Set(Array.from(tableNames, (n) => n.toLowerCase()));
    if (tables.has(first.toLowerCase())) {
      // Bare table name, or a table name followed by a KQL operator keyword.
      const rest = t.slice(first.length).trimStart();
      if (rest === "" || /^(\||where|project|summarize|take|limit|order|extend|count|distinct|top|sort|join|union|search|render)\b/i.test(rest)) {
        return "kql";
      }
    }
  }
  return "search";
}

/** Short label for the mode badge. */
export function modeLabel(mode: ExploreMode): string {
  return mode === "search" ? "Search" : mode === "kql" ? "KQL" : "SQL";
}
