import MonacoEditor, { type OnMount } from "@monaco-editor/react";
import { useCallback, useEffect, useRef } from "react";
import { registerKql, registerKqlCompletions, KQL_LANG_ID } from "./kql-language";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  schema?: SchemaDoc;
  database: string;
};

export function Editor({ value, onChange, onRun, schema, database }: Props) {
  const onRunRef = useRef(onRun);
  useEffect(() => { onRunRef.current = onRun; }, [onRun]);

  const handleMount: OnMount = useCallback((editor, monaco) => {
    registerKql(monaco);
    registerKqlCompletions(monaco, () => {
      const db = schema?.databases.find((d) => d.name === database);
      const tables = db?.tables.map((t) => t.name) ?? [];
      const columnsByTable: Record<string, string[]> = {};
      db?.tables.forEach((t) => { columnsByTable[t.name] = t.columns.map((c) => c.name); });
      return { tables, columnsByTable };
    });
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => onRunRef.current());
  }, [database, schema]);

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
