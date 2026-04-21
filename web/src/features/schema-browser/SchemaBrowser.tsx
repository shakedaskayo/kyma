import { useState } from "react";
import { ChevronRight, Database, Table2, Columns3 } from "lucide-react";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = { schema?: SchemaDoc; onInsert: (text: string) => void };

export function SchemaBrowser({ schema, onInsert }: Props) {
  if (!schema) return <div className="p-3 text-xs text-muted-foreground">Loading schema…</div>;
  return (
    <div className="h-full overflow-auto p-2 text-xs">
      {schema.databases.map((db) => <DatabaseNode key={db.name} db={db} onInsert={onInsert} />)}
    </div>
  );
}

function DatabaseNode({ db, onInsert }: { db: SchemaDoc["databases"][number]; onInsert: (t: string) => void }) {
  const [open, setOpen] = useState(true);
  return (
    <div>
      <button className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50" onClick={() => setOpen(!open)}>
        <ChevronRight className={`h-3 w-3 transition ${open ? "rotate-90" : ""}`} />
        <Database className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="font-medium">{db.name}</span>
      </button>
      {open && <div className="ml-3 border-l pl-1">{db.tables.map((t) => <TableNode key={t.name} table={t} onInsert={onInsert} />)}</div>}
    </div>
  );
}

function TableNode({ table, onInsert }: { table: SchemaDoc["databases"][number]["tables"][number]; onInsert: (t: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50"
        onClick={() => setOpen(!open)}
        onDoubleClick={() => onInsert(table.name)}>
        <ChevronRight className={`h-3 w-3 transition ${open ? "rotate-90" : ""}`} />
        <Table2 className="h-3.5 w-3.5 text-muted-foreground" />
        <span>{table.name}</span>
      </button>
      {open && (
        <div className="ml-3 border-l pl-1 text-[11px]">
          {table.columns.map((c) => (
            <button key={c.name}
              className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-left hover:bg-accent/50"
              onClick={() => onInsert(c.name)}>
              <Columns3 className="h-3 w-3 text-muted-foreground" />
              <span>{c.name}</span>
              <span className="ml-auto text-muted-foreground">{c.type}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
