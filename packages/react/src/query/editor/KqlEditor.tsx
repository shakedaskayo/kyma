import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef } from "react";
import { registerKql, registerKqlCompletions, KQL_LANG_ID, type KqlSchema } from "./kql-language";
import type { SchemaDoc } from "@kyma-ai/client";
import { useKymaContext } from "../../provider/context";

export type { KqlSchema };

export type KqlEditorProps = {
  value: string;
  onChange: (v: string) => void;
  /** Called when the user presses Ctrl/Cmd+Enter to run the query. */
  onRun: () => void;
  /** Optional: schema for completion hints. */
  schema?: SchemaDoc;
  /** The active database name — used to scope column completions. */
  database: string;
  /** Optional: map of column name → top-N observed values (for completion hints). */
  knownValues?: Record<string, string[]>;
};

/** Monaco theme name used in dark mode.
 *
 * The theme is defined from KymaProvider theme tokens: we read `isDark` from
 * context and select either this custom dark theme or the built-in "vs" light
 * theme. Colors are matched to the Kyma dark palette (--kyma-card HSL triplet
 * ≈ "217.2 32.6% 9%"). A static dark/light pair is used here rather than
 * dynamically deriving colors from CSS vars because Monaco's theme API requires
 * hex strings resolved at definition time — runtime CSS var reads are not
 * possible. The dark palette is correct for Kyma's default dark theme. */
const KYMA_DARK_THEME = "kyma-dark";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function ensureKymaDarkTheme(monaco: any) {
  monaco.editor.defineTheme(KYMA_DARK_THEME, {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#0f1626",
      "editor.foreground": "#e2e8f0",
      "editorLineNumber.foreground": "#475569",
      "editorLineNumber.activeForeground": "#94a3b8",
      "editorCursor.foreground": "#e2e8f0",
      "editor.lineHighlightBackground": "#1e293b40",
      "editor.selectionBackground": "#334155aa",
      "editorIndentGuide.background1": "#1e293b",
      "editorIndentGuide.activeBackground1": "#334155",
      "editorGutter.background": "#0f1626",
      "editorWidget.background": "#0f1626",
      "editorWidget.border": "#1e293b",
      "editorSuggestWidget.background": "#0f1626",
      "editorSuggestWidget.border": "#1e293b",
      "editorSuggestWidget.selectedBackground": "#1e293b",
      "editorSuggestWidget.highlightForeground": "#60a5fa",
    },
  });
}

/**
 * Controlled Monaco-based KQL editor for the Kyma SDK.
 *
 * Tab/workspace orchestration is the caller's responsibility:
 *   - `value` / `onChange` carry the controlled query text.
 *   - `onRun` is fired when the user presses Ctrl/Cmd+Enter.
 *   - `schema` and `database` power schema-aware completions; pass `undefined`
 *     schema while the catalog is loading — completions degrade gracefully.
 */
export function KqlEditor({ value, onChange, onRun, schema, database, knownValues }: KqlEditorProps) {
  const { isDark } = useKymaContext();
  const onRunRef = useRef(onRun);
  useEffect(() => { onRunRef.current = onRun; }, [onRun]);

  const schemaRef = useRef<{ schema?: SchemaDoc; database: string; knownValues?: Record<string, string[]> }>({ schema, database, knownValues });
  useEffect(() => { schemaRef.current = { schema, database, knownValues }; }, [schema, database, knownValues]);

  const handleMount: OnMount = useCallback((editor, monaco) => {
    registerKql(monaco);
    ensureKymaDarkTheme(monaco);
    registerKqlCompletions(monaco, (): KqlSchema => {
      const { schema: s, database: db, knownValues: kv } = schemaRef.current;
      const dbDoc = s?.databases.find((d) => d.name === db);
      const tables = dbDoc?.tables.map((t) => t.name) ?? [];
      const columnsByTable: Record<string, string[]> = {};
      const columnTypes: Record<string, Record<string, string>> = {};
      dbDoc?.tables.forEach((t) => {
        columnsByTable[t.name] = t.columns.map((c) => c.name);
        columnTypes[t.name] = {};
        t.columns.forEach((c) => { columnTypes[t.name][c.name] = c.type; });
      });
      return { tables, columnsByTable, columnTypes, knownValues: kv };
    });
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => onRunRef.current());
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <MonacoEditor
      height="100%"
      language={KQL_LANG_ID}
      theme={isDark ? KYMA_DARK_THEME : "vs"}
      value={value}
      onChange={(v) => onChange(v ?? "")}
      onMount={handleMount}
      options={{
        minimap: { enabled: false },
        lineNumbers: "on",
        scrollBeyondLastLine: false,
        fontSize: 13,
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        tabSize: 2,
        automaticLayout: true,
        wordWrap: "on",
      }}
    />
  );
}
