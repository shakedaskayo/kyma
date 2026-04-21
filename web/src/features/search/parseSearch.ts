/**
 * parseSearch — translates a simple text query into a KQL tail expression.
 *
 * Grammar:
 *   - Bare words          → substring across all STRING-typed columns
 *   - field:value         → field == "value"
 *   - -field:value        → field != "value"
 *   - -word               → NOT (col1 contains "word" or …)
 *   - "multi word"        → treated as a single bare-word term
 *   - Multiple terms      → AND'd together
 *
 * @param input        The raw search string typed by the user.
 * @param leadingTable The KQL table name extracted from the editor (e.g. "otel_logs").
 * @param schema       Schema descriptor so we can know string columns.
 * @returns A full KQL string ready to be sent to the server, or the leading table alone if input is blank.
 */

export interface SearchSchema {
  /** All tables in the database */
  tables: string[];
  /** Column names keyed by table name */
  columnsByTable: Record<string, string[]>;
  /** Column types keyed by table name → column name → type string */
  columnTypes?: Record<string, Record<string, string>>;
}

export type ParsedTerm =
  | { kind: "bare"; word: string; negate: boolean }
  | { kind: "field"; field: string; value: string; negate: boolean };

/** Tokenize the raw search input into ParsedTerms */
export function tokenize(input: string): ParsedTerm[] {
  const terms: ParsedTerm[] = [];
  // Regex: optional leading -, then either "quoted string" or field:value or bare word
  const re = /(-?)(?:"([^"]*)"|([\w.-]+):((?:"[^"]*"|[^\s]+))|([^\s"]+))/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(input)) !== null) {
    const negate = m[1] === "-";
    if (m[2] !== undefined) {
      // Quoted bare word (e.g. "multi word")
      terms.push({ kind: "bare", word: m[2], negate });
    } else if (m[3] !== undefined) {
      // field:value
      const field = m[3];
      let value = m[4];
      // Strip surrounding quotes from value if present
      if (value.startsWith('"') && value.endsWith('"')) {
        value = value.slice(1, -1);
      }
      terms.push({ kind: "field", field, value, negate });
    } else if (m[5] !== undefined) {
      // Bare unquoted word
      terms.push({ kind: "bare", word: m[5], negate });
    }
  }
  return terms;
}

/** Return the string-typed column names for a given table */
function stringColumnsFor(table: string, schema: SearchSchema): string[] {
  const cols = schema.columnsByTable[table] ?? [];
  if (!schema.columnTypes) return cols; // fallback: treat all as string
  const types = schema.columnTypes[table] ?? {};
  return cols.filter((c) => {
    const t = (types[c] ?? "string").toLowerCase();
    // Include string-like types; exclude numeric, bool, datetime, etc.
    return t === "string" || t === "str" || t === "" || (!["int","long","real","double","bool","datetime","timespan","dynamic","guid"].includes(t));
  });
}

/**
 * Build a KQL predicate clause from a single term.
 * Returns null if the term cannot be mapped (e.g. unknown field with no string cols).
 */
function termToKql(term: ParsedTerm, table: string, schema: SearchSchema): string | null {
  if (term.kind === "field") {
    const val = term.value.replace(/"/g, '\\"');
    if (term.negate) {
      return `${term.field} != "${val}"`;
    }
    return `${term.field} == "${val}"`;
  }

  // bare word — substring across all string columns
  const strCols = stringColumnsFor(table, schema);
  if (strCols.length === 0) return null;

  const word = term.word.replace(/"/g, '\\"');
  const parts = strCols.map((c) => `${c} contains "${word}"`).join(" or ");
  const predicate = strCols.length > 1 ? `(${parts})` : parts;

  if (term.negate) {
    return `not ${predicate}`;
  }
  return predicate;
}

/**
 * Translate a simple search string into a full KQL query.
 *
 * @param input        Raw search text (may be empty/blank).
 * @param leadingTable The table name (e.g. "otel_logs").
 * @param schema       Schema info for column discovery.
 * @param limit        Max rows to take (default 500).
 * @returns Full KQL string.
 */
export function parseSearch(
  input: string,
  leadingTable: string,
  schema: SearchSchema,
  limit = 500,
): string {
  const trimmed = input.trim();
  if (!trimmed) {
    return `${leadingTable} | take ${limit}`;
  }

  const terms = tokenize(trimmed);
  const clauses: string[] = [];

  for (const term of terms) {
    const clause = termToKql(term, leadingTable, schema);
    if (clause) clauses.push(clause);
  }

  if (clauses.length === 0) {
    return `${leadingTable} | take ${limit}`;
  }

  const whereChain = clauses.map((c) => `| where ${c}`).join("\n");
  return `${leadingTable}\n${whereChain}\n| take ${limit}`;
}

/**
 * Extract the leading table name from a KQL query string.
 * Returns null if the query is blank or starts with a non-identifier.
 */
export function extractLeadingTable(query: string): string | null {
  const trimmed = query.trim();
  if (!trimmed) return null;
  const m = trimmed.match(/^([A-Za-z_][A-Za-z0-9_]*)/);
  return m ? m[1] : null;
}
