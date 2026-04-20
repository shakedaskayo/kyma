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

export const KQL_LANG_ID = "kql";

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

export function registerKqlCompletions(
  monaco: typeof MonacoNs,
  getSchema: () => { tables: string[]; columnsByTable: Record<string, string[]> },
) {
  monaco.languages.registerCompletionItemProvider(KQL_LANG_ID, {
    triggerCharacters: [" ", "|", "."],
    provideCompletionItems(model, position) {
      const schema = getSchema();
      const lineText = model.getLineContent(position.lineNumber).slice(0, position.column - 1);
      const word = model.getWordUntilPosition(position);
      const range = new monaco.Range(position.lineNumber, word.startColumn, position.lineNumber, word.endColumn);

      // After `| ` → suggest operators
      if (/\|\s*$/.test(lineText)) {
        return { suggestions: KEYWORDS.map((k) => ({
          label: k, kind: monaco.languages.CompletionItemKind.Keyword,
          insertText: k, range,
        })) };
      }
      // After `project `, `where `, `by `, `extend ` → suggest columns of the leading source
      const tableMatch = model.getValue().match(/^\s*([A-Za-z_][A-Za-z0-9_]*)/);
      const leadingTable = tableMatch?.[1];
      if (/(where|project|project-away|by|extend|summarize)\s+(\w*[,\s]*)*$/.test(lineText) && leadingTable) {
        const cols = schema.columnsByTable[leadingTable] ?? [];
        return { suggestions: cols.map((c) => ({
          label: c, kind: monaco.languages.CompletionItemKind.Field, insertText: c, range,
        })) };
      }
      // Default: offer tables + keywords + functions
      return {
        suggestions: [
          ...schema.tables.map((t) => ({
            label: t, kind: monaco.languages.CompletionItemKind.Class, insertText: t, range,
          })),
          ...KEYWORDS.map((k)  => ({ label: k, kind: monaco.languages.CompletionItemKind.Keyword,  insertText: k, range })),
          ...FUNCTIONS.map((f) => ({ label: f, kind: monaco.languages.CompletionItemKind.Function, insertText: `${f}()`, range })),
        ],
      };
    },
  });
}
