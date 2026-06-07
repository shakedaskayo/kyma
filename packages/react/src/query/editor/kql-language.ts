import type * as MonacoNs from "monaco-editor";

const KEYWORDS = [
  "where","project","project-away","extend","summarize","by","take","limit",
  "sort","order","asc","desc","top","count","distinct","join","kind","on",
  "let","union","range","evaluate","parse","mv-expand","make-series","as","between",
];
const FUNCTIONS = [
  "now","ago","bin","startofhour","startofday","strcat","tolower","toupper","tostring",
  "toint","tolong","todouble","todatetime","isnull","isnotnull","isempty","iff",
  "count","sum","avg","min","max","dcount","percentile","stdev","variance",
];
const TYPES = ["int","long","real","double","bool","string","datetime","timespan","dynamic","guid"];
const STRING_OPS = ["contains","!contains","startswith","!startswith","endswith","!endswith","has","!has","matches","regex"];

// Pipe operators (after |)
const PIPE_OPERATORS: Array<{ label: string; doc: string; snippet?: string }> = [
  { label: "where",       doc: "Filters rows by a predicate expression.",                                    snippet: "where ${1:condition}" },
  { label: "project",     doc: "Select and rename columns.",                                                  snippet: "project ${1:col1}, ${2:col2}" },
  { label: "project-away",doc: "Remove specified columns from the output.",                                    snippet: "project-away ${1:column}" },
  { label: "extend",      doc: "Add computed columns to the result set.",                                      snippet: "extend ${1:alias} = ${2:expression}" },
  { label: "summarize",   doc: "Produce a table with aggregated values.",                                      snippet: "summarize ${1:alias} = ${2:count()} by ${3:column}" },
  { label: "take",        doc: "Return up to N rows from the input (no ordering guarantee).",                  snippet: "take ${1:100}" },
  { label: "limit",       doc: "Alias for take — return up to N rows.",                                        snippet: "limit ${1:100}" },
  { label: "order by",    doc: "Sort rows by one or more columns.",                                            snippet: "order by ${1:column} ${2|asc,desc|}" },
  { label: "sort by",     doc: "Alias for order by — sort rows by one or more columns.",                       snippet: "sort by ${1:column} ${2|asc,desc|}" },
  { label: "top",         doc: "Return the top N rows sorted by a column.",                                    snippet: "top ${1:10} by ${2:column} ${3|asc,desc|}" },
  { label: "distinct",    doc: "Return unique values for the specified columns.",                               snippet: "distinct ${1:column}" },
  { label: "join",        doc: "Join two tables on matching keys.",                                            snippet: "join kind=${1|inner,leftouter,rightouter,fullouter,leftanti|} (${2:other_table}) on ${3:column}" },
  { label: "count",       doc: "Count rows in the result set.",                                                snippet: "count" },
  { label: "union",       doc: "Combine rows from two or more tables.",                                        snippet: "union ${1:other_table}" },
  { label: "let",         doc: "Bind a name to an expression or sub-query.",                                   snippet: "let ${1:name} = ${2:value};" },
  { label: "make-series", doc: "Create a series of aggregated values by a time dimension.",                    snippet: "make-series ${1:count()} default=0 on timestamp from ago(${2:1h}) to now() step ${3:5m} by ${4:col}" },
  { label: "parse",       doc: "Parse a string column into structured fields with a pattern.",                 snippet: "parse ${1:column} with ${2:pattern}" },
  { label: "mv-expand",   doc: "Expand multi-value dynamic columns into individual rows.",                     snippet: "mv-expand ${1:column}" },
  { label: "evaluate",    doc: "Call a plugin function (e.g. basket, autocluster).",                           snippet: "evaluate ${1:plugin}()" },
  { label: "range",       doc: "Generate a sequence of values.",                                               snippet: "range ${1:col} from ${2:1} to ${3:10} step ${4:1}" },
];

// Column operators (after a known column name)
const COLUMN_OPERATORS: Array<{ label: string; doc: string }> = [
  { label: "==",              doc: "Equals — exact match." },
  { label: "!=",              doc: "Not equals." },
  { label: ">",               doc: "Greater than." },
  { label: ">=",              doc: "Greater than or equal." },
  { label: "<",               doc: "Less than." },
  { label: "<=",              doc: "Less than or equal." },
  { label: "between",         doc: "between(low, high) — inclusive range check." },
  { label: "contains",        doc: "Case-insensitive substring match." },
  { label: "!contains",       doc: "Negated contains — does not contain substring." },
  { label: "startswith",      doc: "String starts with value." },
  { label: "!startswith",     doc: "String does not start with value." },
  { label: "endswith",        doc: "String ends with value." },
  { label: "!endswith",       doc: "String does not end with value." },
  { label: "has",             doc: "Whole-term match (token boundary aware)." },
  { label: "!has",            doc: "Negated has." },
  { label: "matches regex",   doc: "Regular expression match." },
  { label: "in",              doc: "in(v1, v2, …) — value is one of a list." },
  { label: "!in",             doc: "Value is NOT in the list." },
];

// Function documentation
const FUNCTION_DOCS: Record<string, string> = {
  ago:          "ago(TIMESPAN) — a datetime a given timespan before now. e.g. ago(1h)",
  now:          "now() — the current UTC datetime.",
  bin:          "bin(value, roundTo) — rounds value down to the nearest multiple of roundTo.",
  startofhour:  "startofhour(datetime) — returns the start of the hour containing the given datetime.",
  startofday:   "startofday(datetime) — returns the start of the day containing the given datetime.",
  strcat:       "strcat(s1, s2, …) — concatenate strings.",
  tolower:      "tolower(string) — converts string to lower case.",
  toupper:      "toupper(string) — converts string to upper case.",
  tostring:     "tostring(value) — converts value to string representation.",
  toint:        "toint(value) — converts value to int.",
  tolong:       "tolong(value) — converts value to long.",
  todouble:     "todouble(value) — converts value to double.",
  todatetime:   "todatetime(value) — parses value as a datetime.",
  isnull:       "isnull(value) — returns true if value is null.",
  isnotnull:    "isnotnull(value) — returns true if value is not null.",
  isempty:      "isempty(value) — returns true if value is empty or null.",
  iff:          "iff(condition, ifTrue, ifFalse) — conditional value — returns ifTrue if condition is true.",
  count:        "count() — counts rows in the group.",
  sum:          "sum(column) — sum of values in column.",
  avg:          "avg(column) — average of values in column.",
  min:          "min(column) — minimum value in column.",
  max:          "max(column) — maximum value in column.",
  dcount:       "dcount(column[, accuracy]) — distinct count of values in column.",
  percentile:   "percentile(column, percent) — returns the percentile of values in column.",
  stdev:        "stdev(column) — standard deviation of values in column.",
  variance:     "variance(column) — variance of values in column.",
};

// Keyword documentation
const KEYWORD_DOCS: Record<string, string> = {
  where:        "Filters rows by predicate.",
  project:      "Selects and renames columns.",
  "project-away": "Removes specified columns.",
  extend:       "Adds computed columns.",
  summarize:    "Aggregates rows, producing one row per group.",
  by:           "Groups rows for summarize/order/sort operations.",
  take:         "Returns up to N rows (no ordering guarantee).",
  limit:        "Alias for take.",
  sort:         "Sorts rows by specified columns.",
  "order":      "Sorts rows by specified columns.",
  "order by":   "Sorts rows by specified columns.",
  asc:          "Sort in ascending order.",
  desc:         "Sort in descending order.",
  top:          "Returns the top N rows by a sort column.",
  count:        "Counts rows in the result.",
  distinct:     "Returns distinct values for columns.",
  join:         "Joins two tables on matching column keys.",
  kind:         "Join kind: inner, leftouter, rightouter, fullouter, leftanti.",
  on:           "Specifies join key column(s).",
  let:          "Binds a name to an expression or sub-query.",
  union:        "Combines rows from two or more tables.",
  range:        "Generates a table with a sequence of values.",
  evaluate:     "Invokes a plugin function.",
  parse:        "Parses a string column using a pattern.",
  "mv-expand":  "Expands dynamic array column into rows.",
  "make-series": "Creates a time-series aggregation.",
  as:           "Assigns an alias.",
  between:      "Inclusive range filter: col between (low .. high).",
};

// Signature help definitions
interface SigParam { name: string; doc: string; }
interface SigDef { doc: string; params: SigParam[]; }

const SIGNATURE_DEFS: Record<string, SigDef> = {
  ago: {
    doc: "Returns a datetime offset from now by the given timespan.",
    params: [{ name: "timespan", doc: "e.g. 1h, 30m, 7d — the duration to go back." }],
  },
  bin: {
    doc: "Rounds value down to the nearest multiple of roundTo.",
    params: [
      { name: "value",   doc: "The numeric or datetime value to round." },
      { name: "roundTo", doc: "The rounding interval, e.g. 5m, 1h, 100." },
    ],
  },
  count: {
    doc: "Counts rows in the current group.",
    params: [],
  },
  sum: {
    doc: "Sums numeric values in a column.",
    params: [{ name: "column", doc: "Name of the numeric column to sum." }],
  },
  avg: {
    doc: "Computes the average of numeric values in a column.",
    params: [{ name: "column", doc: "Name of the numeric column to average." }],
  },
  min: {
    doc: "Returns the minimum value in a column.",
    params: [{ name: "column", doc: "Name of the column." }],
  },
  max: {
    doc: "Returns the maximum value in a column.",
    params: [{ name: "column", doc: "Name of the column." }],
  },
  percentile: {
    doc: "Returns the value at the given percentile in a column.",
    params: [
      { name: "column",  doc: "Name of the numeric column." },
      { name: "percent", doc: "Percentile to compute, e.g. 95, 99." },
    ],
  },
  dcount: {
    doc: "Returns the estimated distinct count of values in a column.",
    params: [
      { name: "column",   doc: "Column to count distinct values in." },
      { name: "accuracy", doc: "Optional: accuracy level 0–4 (default 1)." },
    ],
  },
  startofhour: {
    doc: "Returns the start of the hour containing the given datetime.",
    params: [{ name: "datetime", doc: "The datetime expression." }],
  },
  startofday: {
    doc: "Returns the start of the day containing the given datetime.",
    params: [{ name: "datetime", doc: "The datetime expression." }],
  },
  strcat: {
    doc: "Concatenates multiple strings.",
    params: [
      { name: "string1", doc: "First string argument." },
      { name: "...",     doc: "Additional string arguments." },
    ],
  },
  tolower: {
    doc: "Converts a string to lower case.",
    params: [{ name: "string", doc: "The string to convert." }],
  },
  toupper: {
    doc: "Converts a string to upper case.",
    params: [{ name: "string", doc: "The string to convert." }],
  },
  tostring: {
    doc: "Converts a value to its string representation.",
    params: [{ name: "value", doc: "The value to stringify." }],
  },
  toint: {
    doc: "Converts a value to int.",
    params: [{ name: "value", doc: "The value to convert." }],
  },
  todouble: {
    doc: "Converts a value to double.",
    params: [{ name: "value", doc: "The value to convert." }],
  },
  iff: {
    doc: "Returns ifTrue if condition is true, else ifFalse.",
    params: [
      { name: "condition", doc: "Boolean expression." },
      { name: "ifTrue",    doc: "Value returned when condition is true." },
      { name: "ifFalse",   doc: "Value returned when condition is false." },
    ],
  },
};

export const KQL_LANG_ID = "kql";

/**
 * Register the KQL language grammar and configuration on the Monaco singleton.
 * Guards with monaco.languages.getLanguages() so it is safe to call from
 * multiple mounted editors — only the first call performs real work.
 */
export function registerKql(monaco: typeof MonacoNs) {
  if (monaco.languages.getLanguages().some((l) => l.id === KQL_LANG_ID)) return;

  monaco.languages.register({ id: KQL_LANG_ID, extensions: [".kql"], aliases: ["KQL","Kusto"] });

  monaco.languages.setMonarchTokensProvider(KQL_LANG_ID, {
    defaultToken: "",
    tokenPostfix: ".kql",
    keywords: KEYWORDS,
    functions: FUNCTIONS,
    typeKeywords: TYPES,
    stringOps: STRING_OPS,
    tokenizer: {
      root: [
        [/\/\/.*$/, "comment"],
        [/\|/, "operator"],
        [/"([^"\\]|\\.)*"/, "string"],
        [/'([^'\\]|\\.)*'/, "string"],
        [/\b\d+(\.\d+)?\b/, "number"],
        [/\b(true|false|null)\b/, "keyword"],
        [/\b([A-Za-z_][A-Za-z0-9_]*)\b/, {
          cases: {
            "@keywords":     "keyword",
            "@functions":    "type.identifier",
            "@typeKeywords": "type",
            "@stringOps":    "keyword.operator",
            "@default":      "identifier",
          },
        }],
        [/[=!<>]=?|[+\-*/]|\band\b|\bor\b|\bnot\b/, "operator"],
      ],
    },
  });

  monaco.languages.setLanguageConfiguration(KQL_LANG_ID, {
    comments: { lineComment: "//" },
    brackets: [["(", ")"], ["[", "]"], ["{", "}"]],
    autoClosingPairs: [
      { open: "(", close: ")" },
      { open: "[", close: "]" },
      { open: "{", close: "}" },
      { open: '"', close: '"' },
      { open: "'", close: "'" },
    ],
  });
}

export interface KqlSchema {
  tables: string[];
  columnsByTable: Record<string, string[]>;
  columnTypes?: Record<string, Record<string, string>>;
  knownValues?: Record<string, string[]>;
}

export function registerKqlCompletions(
  monaco: typeof MonacoNs,
  getSchema: () => KqlSchema,
) {
  monaco.languages.registerCompletionItemProvider(KQL_LANG_ID, {
    triggerCharacters: [" ", "|", ".", "(", ",", "="],
    provideCompletionItems(model, position) {
      const schema = getSchema();
      const lineText = model.getLineContent(position.lineNumber).slice(0, position.column - 1);
      const word = model.getWordUntilPosition(position);
      const range = new monaco.Range(
        position.lineNumber, word.startColumn,
        position.lineNumber, word.endColumn,
      );

      // Extract leading table from the entire document
      const tableMatch = model.getValue().match(/^\s*([A-Za-z_][A-Za-z0-9_]*)/);
      const leadingTable = tableMatch?.[1] ?? "";
      const cols = leadingTable ? (schema.columnsByTable[leadingTable] ?? []) : [];
      const colTypes = leadingTable ? (schema.columnTypes?.[leadingTable] ?? {}) : {};

      // ── After `| ` → pipe operator suggestions ──────────────────────────────
      if (/\|\s*\w*$/.test(lineText) && !/\|\s*\w+\s+/.test(lineText)) {
        return {
          suggestions: PIPE_OPERATORS.map((op) => ({
            label: op.label,
            kind: monaco.languages.CompletionItemKind.Keyword,
            insertText: op.snippet ?? op.label,
            insertTextRules: op.snippet ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet : undefined,
            detail: "KQL operator",
            documentation: op.doc,
            range,
          })),
        };
      }

      // ── After a known column name + space → column operators ────────────────
      const colOpMatch = lineText.match(/\b([A-Za-z_][A-Za-z0-9_]*)\s+$/);
      if (colOpMatch) {
        const maybeCol = colOpMatch[1];
        if (cols.includes(maybeCol)) {
          return {
            suggestions: COLUMN_OPERATORS.map((op) => ({
              label: op.label,
              kind: monaco.languages.CompletionItemKind.Operator,
              insertText: op.label,
              detail: "operator",
              documentation: op.doc,
              range,
            })),
          };
        }
      }

      // ── After `col == ` or `col contains ` → value hints ──────────────────
      const valueHintMatch = lineText.match(
        /\b([A-Za-z_][A-Za-z0-9_]*)\s+(?:==|!=|contains|has|startswith|endswith)\s+["']?(\w*)$/,
      );
      if (valueHintMatch) {
        const colName = valueHintMatch[1];
        const knownVals = schema.knownValues?.[colName];
        if (knownVals && knownVals.length > 0) {
          return {
            suggestions: knownVals.map((v) => ({
              label: `"${v}"`,
              kind: monaco.languages.CompletionItemKind.Value,
              insertText: `"${v}"`,
              detail: `known value for ${colName}`,
              range,
            })),
          };
        }
      }

      // ── After `where `, `project `, `by `, `extend `, `summarize `, `distinct ` → columns ──
      if (
        /(where|project|project-away|by|extend|summarize|distinct|sort\s+by|order\s+by|top\s+\d+\s+by)\s+(\w*[,\s]*)*$/.test(lineText) &&
        leadingTable
      ) {
        const colSuggestions = cols.map((c) => ({
          label: c,
          kind: monaco.languages.CompletionItemKind.Field,
          insertText: c,
          detail: colTypes[c] ?? "column",
          documentation: `Column '${c}' of type ${colTypes[c] ?? "unknown"} from ${leadingTable}`,
          range,
        }));
        const funcSuggestions = FUNCTIONS.map((f) => ({
          label: f,
          kind: monaco.languages.CompletionItemKind.Function,
          insertText: `${f}(\${1})`,
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "function",
          documentation: FUNCTION_DOCS[f] ?? f,
          range,
        }));
        return { suggestions: [...colSuggestions, ...funcSuggestions] };
      }

      // ── Default: tables + keywords + snippets + functions ──────────────────
      const tableSuggestions = schema.tables.map((t) => ({
        label: t,
        kind: monaco.languages.CompletionItemKind.Class,
        insertText: t,
        detail: "table",
        documentation: `Table: ${t}`,
        range,
      }));

      const keywordSuggestions = KEYWORDS.map((k) => ({
        label: k,
        kind: monaco.languages.CompletionItemKind.Keyword,
        insertText: k,
        detail: "keyword",
        documentation: KEYWORD_DOCS[k] ?? k,
        range,
      }));

      const snippetSuggestions: MonacoNs.languages.CompletionItem[] = [
        {
          label: "summarize (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "summarize ${1:alias} = ${2:count()} by ${3:column}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Insert a summarize with aggregation and grouping.",
          range,
        },
        {
          label: "project (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "project ${1:col1}, ${2:col2}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Select specific columns.",
          range,
        },
        {
          label: "extend (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "extend ${1:alias} = ${2:expression}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Add a computed column.",
          range,
        },
        {
          label: "join (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "join kind=${1|inner,leftouter,rightouter,fullouter,leftanti|} (${2:other_table}) on ${3:column}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Join another table.",
          range,
        },
        {
          label: "order (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "order by ${1:column} ${2|asc,desc|}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Sort rows.",
          range,
        },
        {
          label: "let (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "let ${1:name} = ${2:value};",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Bind a name to an expression.",
          range,
        },
        {
          label: "top (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "top ${1:10} by ${2:column} ${3|asc,desc|}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Return top N rows.",
          range,
        },
        {
          label: "make-series (snippet)",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: "make-series ${1:count()} default=0 on timestamp from ago(${2:1h}) to now() step ${3:5m} by ${4:col}",
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: "snippet",
          documentation: "Create a time-series aggregation.",
          range,
        },
      ];

      const funcSuggestions = FUNCTIONS.map((f) => ({
        label: f,
        kind: monaco.languages.CompletionItemKind.Function,
        insertText: `${f}(\${1})`,
        insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
        detail: "function",
        documentation: FUNCTION_DOCS[f] ?? f,
        range,
      }));

      return {
        suggestions: [
          ...tableSuggestions,
          ...keywordSuggestions,
          ...snippetSuggestions,
          ...funcSuggestions,
        ],
      };
    },
  });

  // ── Signature Help Provider ─────────────────────────────────────────────────
  monaco.languages.registerSignatureHelpProvider(KQL_LANG_ID, {
    signatureHelpTriggerCharacters: ["(", ","],
    signatureHelpRetriggerCharacters: [","],
    provideSignatureHelp(model, position) {
      const textBeforeCursor = model.getValueInRange({
        startLineNumber: 1,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });

      let depth = 0;
      let funcStart = -1;
      let commaCount = 0;

      for (let i = textBeforeCursor.length - 1; i >= 0; i--) {
        const ch = textBeforeCursor[i];
        if (ch === ")") depth++;
        else if (ch === "(") {
          if (depth === 0) {
            funcStart = i;
            break;
          }
          depth--;
        } else if (ch === "," && depth === 0) {
          commaCount++;
        }
      }

      if (funcStart < 0) return null;

      const before = textBeforeCursor.slice(0, funcStart);
      const fnMatch = before.match(/([A-Za-z_][A-Za-z0-9_]*)$/);
      if (!fnMatch) return null;

      const fnName = fnMatch[1].toLowerCase();
      const sigDef = SIGNATURE_DEFS[fnName];
      if (!sigDef) return null;

      const paramLabels = sigDef.params.map((p) => p.name);
      const sigLabel =
        sigDef.params.length === 0
          ? `${fnName}()`
          : `${fnName}(${paramLabels.join(", ")})`;

      const activeParam = Math.min(commaCount, sigDef.params.length - 1);

      const sigHelp: MonacoNs.languages.SignatureHelp = {
        signatures: [
          {
            label: sigLabel,
            documentation: sigDef.doc,
            parameters: sigDef.params.map((p) => ({
              label: p.name,
              documentation: p.doc,
            })),
          },
        ],
        activeSignature: 0,
        activeParameter: Math.max(0, activeParam),
      };

      return { value: sigHelp, dispose: () => {} };
    },
  });
}
