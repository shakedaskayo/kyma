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
import type { SchemaDoc, ColumnInfo } from "@pensieve-ai/client";

export type { SchemaDoc };

// ── type icon helper ──────────────────────────────────────────────────────────

function iconForType(t: string): React.ReactNode {
  const lo = t.toLowerCase();
  if (/time|date|timestamp/.test(lo))
    return <Calendar className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />;
  if (/int|long|real|double|float|decimal|numeric/.test(lo))
    return <Hash className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />;
  if (/string|varchar|text|utf/.test(lo))
    return <Type className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />;
  if (/bool/.test(lo))
    return <ToggleLeft className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />;
  return <Columns3 className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground" />;
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

// ── prop types ────────────────────────────────────────────────────────────────

export type SchemaBrowserProps = {
  /** Schema document from the catalog API. Pass undefined while loading. */
  schema?: SchemaDoc;
  /** Called with a token (table or column name) to append to the editor. */
  onInsert: (text: string) => void;
  /** Called with a full KQL pipeline + database to replace and run the query.
   *  Enables one-click "Sample" / "Count" quick actions.
   *  Tab orchestration (which tab receives the query) is the caller's concern. */
  onReplaceAndRun?: (kql: string, database: string) => void;
};

// ── sub-components ────────────────────────────────────────────────────────────

export function SchemaBrowser({ schema, onInsert, onReplaceAndRun }: SchemaBrowserProps) {
  const [filter, setFilter] = useState("");

  if (!schema) return <div className="ky-p-3 ky-text-xs ky-text-muted-foreground">Loading schema…</div>;

  const visibleDbs = schema.databases.filter((db) => dbVisible(db, filter));

  return (
    <div className="ky-flex ky-h-full ky-flex-col ky-overflow-hidden">
      <div className="ky-shrink-0 ky-border-b ky-px-2 ky-py-1.5">
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter tables + columns…"
          className="ky-h-7 ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-2 ky-py-1 ky-text-xs ky-text-foreground placeholder:ky-text-muted-foreground focus:ky-outline-none focus:ky-ring-1 focus:ky-ring-ring"
        />
      </div>
      <div className="ky-flex-1 ky-overflow-auto ky-p-2 ky-text-xs">
        {visibleDbs.length === 0
          ? <div className="ky-p-2 ky-text-muted-foreground">No matches.</div>
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
      <button className="ky-flex ky-w-full ky-items-center ky-gap-1 ky-rounded ky-px-1 ky-py-0.5 hover:ky-bg-accent/50" onClick={() => setOpen(!open)}>
        <ChevronRight className={`ky-h-3 ky-w-3 ky-transition ${open ? "ky-rotate-90" : ""}`} />
        <Database className="ky-h-3.5 ky-w-3.5 ky-text-muted-foreground" />
        <span className="ky-font-medium">{db.name}</span>
        <span className="ky-ml-auto ky-text-[10px] ky-text-muted-foreground">
          {db.tables.length} tables
        </span>
      </button>
      {open && (
        <div className="ky-ml-3 ky-border-l ky-pl-1">
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
  const shouldOpen = open || (Boolean(filter) && visibleCols.length > 0 && !matchesFilter(table.name, filter));

  const quickAction = (
    e: React.MouseEvent,
    kql: string,
  ) => {
    e.stopPropagation();
    onReplaceAndRun?.(kql, database);
  };

  return (
    <div className="ky-group/table">
      <div className="ky-flex ky-items-center ky-gap-1">
        <button
          className="ky-flex ky-flex-1 ky-items-center ky-gap-1 ky-rounded ky-px-1 ky-py-0.5 hover:ky-bg-accent/50"
          onClick={() => setOpen(!open)}
          onDoubleClick={() => onInsert(table.name)}
          title="Click to expand; double-click to paste name">
          <ChevronRight className={`ky-h-3 ky-w-3 ky-transition ${shouldOpen ? "ky-rotate-90" : ""}`} />
          <Table2 className="ky-h-3.5 ky-w-3.5 ky-text-muted-foreground" />
          <span>{table.name}</span>
          <span className="ky-ml-auto ky-text-[10px] ky-text-muted-foreground">
            {table.columns.length}c
          </span>
        </button>
        {onReplaceAndRun && (
          <div className="ky-flex ky-opacity-0 ky-transition group-hover/table:ky-opacity-100">
            <IconBtn
              title={`Preview 50 rows of ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | take 50`)}>
              <Eye className="ky-h-3 ky-w-3" />
            </IconBtn>
            <IconBtn
              title={`Count rows in ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | count`)}>
              <Sigma className="ky-h-3 ky-w-3" />
            </IconBtn>
            <IconBtn
              title={`Describe ${table.name} schema + sample values`}
              onClick={(e) =>
                quickAction(e, `${table.name} | take 5 | project *`)
              }>
              <FileText className="ky-h-3 ky-w-3" />
            </IconBtn>
          </div>
        )}
      </div>
      {shouldOpen && (
        <div className="ky-ml-3 ky-border-l ky-pl-1 ky-text-[11px]">
          {visibleCols.map((c) => (
            <button key={c.name}
              className="ky-flex ky-w-full ky-items-center ky-gap-1 ky-rounded ky-px-1 ky-py-0.5 ky-text-left hover:ky-bg-accent/50"
              onClick={() => onInsert(c.name)}>
              {iconForType(c.type)}
              <span>{c.name}</span>
              <span className="ky-ml-auto ky-text-muted-foreground">{c.type}</span>
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
      className="ky-rounded ky-p-1 ky-text-muted-foreground hover:ky-bg-accent hover:ky-text-foreground">
      {children}
    </button>
  );
}
