import { useState } from "react";
import { ChevronRight, Database, Table2, Columns3, Calendar, Hash, Type, ToggleLeft } from "lucide-react";
import type { SchemaDoc, ColumnInfo } from "@/sdk/catalog";

// ── type icon helper ──────────────────────────────────────────────────────────

function iconForType(t: string): React.ReactNode {
  const lo = t.toLowerCase();
  if (/time|date|timestamp/.test(lo))
    return <Calendar className="h-3 w-3 shrink-0 text-muted-foreground" />;
  if (/int|long|real|double|float|decimal|numeric/.test(lo))
    return <Hash className="h-3 w-3 shrink-0 text-muted-foreground" />;
  if (/string|varchar|text|utf/.test(lo))
    return <Type className="h-3 w-3 shrink-0 text-muted-foreground" />;
  if (/bool/.test(lo))
    return <ToggleLeft className="h-3 w-3 shrink-0 text-muted-foreground" />;
  return <Columns3 className="h-3 w-3 shrink-0 text-muted-foreground" />;
}

// ── filter helpers ────────────────────────────────────────────────────────────

function matchesFilter(s: string, filter: string): boolean {
  return s.toLowerCase().includes(filter.toLowerCase());
}

function columnVisible(c: ColumnInfo, filter: string): boolean {
  if (!filter) return true;
  return matchesFilter(c.name, filter) || matchesFilter(c.type, filter);
}

function tableVisible(t: SchemaDoc["databases"][number]["tables"][number], filter: string): boolean {
  if (!filter) return true;
  if (matchesFilter(t.name, filter)) return true;
  return t.columns.some((c) => columnVisible(c, filter));
}

function dbVisible(db: SchemaDoc["databases"][number], filter: string): boolean {
  if (!filter) return true;
  if (matchesFilter(db.name, filter)) return true;
  return db.tables.some((t) => tableVisible(t, filter));
}

// ── sub-components ────────────────────────────────────────────────────────────

type Props = { schema?: SchemaDoc; onInsert: (text: string) => void };

export function SchemaBrowser({ schema, onInsert }: Props) {
  const [filter, setFilter] = useState("");

  if (!schema) return <div className="p-3 text-xs text-muted-foreground">Loading schema…</div>;

  const visibleDbs = schema.databases.filter((db) => dbVisible(db, filter));

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="shrink-0 border-b px-2 py-1.5">
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter tables + columns…"
          className="h-7 w-full rounded-md border border-input bg-background px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        />
      </div>
      <div className="flex-1 overflow-auto p-2 text-xs">
        {visibleDbs.length === 0
          ? <div className="p-2 text-muted-foreground">No matches.</div>
          : visibleDbs.map((db) => (
            <DatabaseNode key={db.name} db={db} onInsert={onInsert} filter={filter} />
          ))
        }
      </div>
    </div>
  );
}

function DatabaseNode({
  db, onInsert, filter,
}: {
  db: SchemaDoc["databases"][number];
  onInsert: (t: string) => void;
  filter: string;
}) {
  const [open, setOpen] = useState(true);
  const visibleTables = db.tables.filter((t) => tableVisible(t, filter));

  return (
    <div>
      <button className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50" onClick={() => setOpen(!open)}>
        <ChevronRight className={`h-3 w-3 transition ${open ? "rotate-90" : ""}`} />
        <Database className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="font-medium">{db.name}</span>
      </button>
      {open && (
        <div className="ml-3 border-l pl-1">
          {visibleTables.map((t) => (
            <TableNode key={t.name} table={t} onInsert={onInsert} filter={filter} />
          ))}
        </div>
      )}
    </div>
  );
}

function TableNode({
  table, onInsert, filter,
}: {
  table: SchemaDoc["databases"][number]["tables"][number];
  onInsert: (t: string) => void;
  filter: string;
}) {
  const [open, setOpen] = useState(false);
  const visibleCols = table.columns.filter((c) => columnVisible(c, filter));
  // Auto-expand when a filter is active and there are matches within
  const shouldOpen = open || (Boolean(filter) && visibleCols.length > 0 && !matchesFilter(table.name, filter));

  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50"
        onClick={() => setOpen(!open)}
        onDoubleClick={() => onInsert(table.name)}>
        <ChevronRight className={`h-3 w-3 transition ${shouldOpen ? "rotate-90" : ""}`} />
        <Table2 className="h-3.5 w-3.5 text-muted-foreground" />
        <span>{table.name}</span>
      </button>
      {shouldOpen && (
        <div className="ml-3 border-l pl-1 text-[11px]">
          {visibleCols.map((c) => (
            <button key={c.name}
              className="flex w-full items-center gap-1 rounded px-1 py-0.5 text-left hover:bg-accent/50"
              onClick={() => onInsert(c.name)}>
              {iconForType(c.type)}
              <span>{c.name}</span>
              <span className="ml-auto text-muted-foreground">{c.type}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
