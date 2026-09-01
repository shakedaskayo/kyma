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
    return <Calendar className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground" />;
  if (/int|long|real|double|float|decimal|numeric/.test(lo))
    return <Hash className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground" />;
  if (/string|varchar|text|utf/.test(lo))
    return <Type className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground" />;
  if (/bool/.test(lo))
    return <ToggleLeft className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground" />;
  return <Columns3 className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground" />;
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

  if (!schema) return <div className="pv-p-3 pv-text-xs pv-text-muted-foreground">Loading schema…</div>;

  const visibleDbs = schema.databases.filter((db) => dbVisible(db, filter));

  return (
    <div className="pv-flex pv-h-full pv-flex-col pv-overflow-hidden">
      <div className="pv-shrink-0 pv-border-b pv-px-2 pv-py-1.5">
        <input
          type="search"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter tables + columns…"
          className="pv-h-7 pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-2 pv-py-1 pv-text-xs pv-text-foreground placeholder:pv-text-muted-foreground focus:pv-outline-none focus:pv-ring-1 focus:pv-ring-ring"
        />
      </div>
      <div className="pv-flex-1 pv-overflow-auto pv-p-2 pv-text-xs">
        {visibleDbs.length === 0
          ? <div className="pv-p-2 pv-text-muted-foreground">No matches.</div>
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
      <button className="pv-flex pv-w-full pv-items-center pv-gap-1 pv-rounded pv-px-1 pv-py-0.5 hover:pv-bg-accent/50" onClick={() => setOpen(!open)}>
        <ChevronRight className={`pv-h-3 pv-w-3 pv-transition ${open ? "pv-rotate-90" : ""}`} />
        <Database className="pv-h-3.5 pv-w-3.5 pv-text-muted-foreground" />
        <span className="pv-font-medium">{db.name}</span>
        <span className="pv-ml-auto pv-text-[10px] pv-text-muted-foreground">
          {db.tables.length} tables
        </span>
      </button>
      {open && (
        <div className="pv-ml-3 pv-border-l pv-pl-1">
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
    <div className="pv-group/table">
      <div className="pv-flex pv-items-center pv-gap-1">
        <button
          className="pv-flex pv-flex-1 pv-items-center pv-gap-1 pv-rounded pv-px-1 pv-py-0.5 hover:pv-bg-accent/50"
          onClick={() => setOpen(!open)}
          onDoubleClick={() => onInsert(table.name)}
          title="Click to expand; double-click to paste name">
          <ChevronRight className={`pv-h-3 pv-w-3 pv-transition ${shouldOpen ? "pv-rotate-90" : ""}`} />
          <Table2 className="pv-h-3.5 pv-w-3.5 pv-text-muted-foreground" />
          <span>{table.name}</span>
          <span className="pv-ml-auto pv-text-[10px] pv-text-muted-foreground">
            {table.columns.length}c
          </span>
        </button>
        {onReplaceAndRun && (
          <div className="pv-flex pv-opacity-0 pv-transition group-hover/table:pv-opacity-100">
            <IconBtn
              title={`Preview 50 rows of ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | take 50`)}>
              <Eye className="pv-h-3 pv-w-3" />
            </IconBtn>
            <IconBtn
              title={`Count rows in ${table.name}`}
              onClick={(e) => quickAction(e, `${table.name} | count`)}>
              <Sigma className="pv-h-3 pv-w-3" />
            </IconBtn>
            <IconBtn
              title={`Describe ${table.name} schema + sample values`}
              onClick={(e) =>
                quickAction(e, `${table.name} | take 5 | project *`)
              }>
              <FileText className="pv-h-3 pv-w-3" />
            </IconBtn>
          </div>
        )}
      </div>
      {shouldOpen && (
        <div className="pv-ml-3 pv-border-l pv-pl-1 pv-text-[11px]">
          {visibleCols.map((c) => (
            <button key={c.name}
              className="pv-flex pv-w-full pv-items-center pv-gap-1 pv-rounded pv-px-1 pv-py-0.5 pv-text-left hover:pv-bg-accent/50"
              onClick={() => onInsert(c.name)}>
              {iconForType(c.type)}
              <span>{c.name}</span>
              <span className="pv-ml-auto pv-text-muted-foreground">{c.type}</span>
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
      className="pv-rounded pv-p-1 pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground">
      {children}
    </button>
  );
}
