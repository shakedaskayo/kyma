import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef } from "react";
import { registerKql, registerKqlCompletions, KQL_LANG_ID, type KqlSchema } from "./kql-language";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  schema?: SchemaDoc;
  database: string;
  /** Optional: map of column name → top-N observed values (for completion hints) */
  knownValues?: Record<string, string[]>;
};

export function Editor({ value, onChange, onRun, schema, database, knownValues }: Props) {
  const onRunRef = useRef(onRun);
  useEffect(() => { onRunRef.current = onRun; }, [onRun]);

  // Keep a stable ref to the latest schema/knownValues so the completion provider
  // always reads the freshest data without needing re-registration.
  const schemaRef = useRef<{ schema?: SchemaDoc; database: string; knownValues?: Record<string, string[]> }>({ schema, database, knownValues });
  useEffect(() => { schemaRef.current = { schema, database, knownValues }; }, [schema, database, knownValues]);

  const handleMount: OnMount = useCallback((editor, monaco) => {
    registerKql(monaco);
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
      theme="vs"
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
