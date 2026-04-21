import { useState } from "react";
import {
  ChevronRight,
  Database,
  Table2,
  Columns3,
  Calendar,
  Hash,
  Type,
  ToggleLeft,
  Eye,
  Sigma,
  FileText,
} from "lucide-react";
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

type Props = {
  schema?: SchemaDoc;
  /** Append a token (table or column name) to the current editor tab. */
  onInsert: (text: string) => void;
  /** Replace the active tab's query (or open a new tab) with a full KQL
   *  pipeline and run it. Enables one-click "Sample" / "Count" actions. */
  onReplaceAndRun?: (kql: string, database: string) => void;
};

export function SchemaBrowser({ schema, onInsert, onReplaceAndRun }: Props) {
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
            <DatabaseNode
              key={db.name}
              db={db}
              onInsert={onInsert}
              onReplaceAndRun={onReplaceAndRun}
              filter={filter} />
          ))
        }
      </div>
    </div>
  );
}

function DatabaseNode({
  db, onInsert, onReplaceAndRun, filter,
}: {
  db: SchemaDoc["databases"][number];
  onInsert: (t: string) => void;
  onReplaceAndRun?: (kql: string, database: string) => void;
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
        <span className="ml-auto text-[10px] text-muted-foreground">
          {db.tables.length} tables
        </span>
      </button>
      {open && (
        <div className="ml-3 border-l pl-1">
          {visibleTables.map((t) => (
            <TableNode
              key={t.name}
              table={t}
              database={db.name}
              onInsert={onInsert}
              onReplaceAndRun={onReplaceAndRun}
              filter={filter} />
          ))}
        </div>
      )}
    </div>
  );
}

function TableNode({
  table, database, onInsert, onReplaceAndRun, filter,
}: {
  table: SchemaDoc["databases"][number]["tables"][number];
  database: string;
  onInsert: (t: string) => void;
  onReplaceAndRun?: (kql: string, database: string) => void;
  filter: string;
}) {
  const [open, setOpen] = useState(false);
  const visibleCols = table.columns.filter((c) => columnVisible(c, filter));
  // Auto-expand when a filter is active and there are matches within
  const shouldOpen = open || (Boolean(filter) && visibleCols.length > 0 && !matchesFilter(table.name, filter));

  const quickAction = (
    e: React.MouseEvent,
    kql: string,
  ) => {
    e.stopPropagation();
    onReplaceAndRun?.(kql, database);
  };

  return (
    <div className="group/table">
      <div className="flex items-center gap-1">
        <button
          className="flex flex-1 items-center gap-1 rounded px-1 py-0.5 hover:bg-accent/50"
          onClick={() => setOpen(!open)}
          onDoubleClick={() => onInsert(table.name)}
          title="Click to expand; double-click to paste name">
          <ChevronRight className={`h-3 w-3 transition ${shouldOpen ? "rotate-90" : ""}`} />
          <Table2 className="h-3.5 w-3.5 text-muted-foreground" />
          <span>{table.name}</span>
          <span className="ml-auto text-[10px] text-muted-foreground">
            {table.columns.length}c
          </span>
        </button>
        {onReplaceAndRun && (
          <div className="flex opacity-0 transition group-hover/table:opacity-100">
            <IconBtn
              title={`Preview 50 rows of ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | take 50`)}>
              <Eye className="h-3 w-3" />
            </IconBtn>
            <IconBtn
              title={`Count rows in ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | count`)}>
              <Sigma className="h-3 w-3" />
            </IconBtn>
            <IconBtn
              title={`Describe ${table.name} schema + sample values`}
              onClick={(e) =>
                quickAction(e, `${table.name} | take 5 | project *`)
              }>
              <FileText className="h-3 w-3" />
            </IconBtn>
          </div>
        )}
      </div>
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

function IconBtn({
  children,
  title,
  onClick,
}: {
  children: React.ReactNode;
  title: string;
  onClick: (e: React.MouseEvent) => void;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground">
      {children}
    </button>
  );
}
