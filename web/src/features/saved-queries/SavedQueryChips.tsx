import { useMemo } from "react";
import type { SchemaDoc } from "@/sdk/catalog";

type Props = {
  onPick: (query: string) => void;
  schema?: SchemaDoc;
  database: string;
};

// Column names that make good "group by" pivots when present.
const CATEGORICAL = [
  "type", "labels", "kind", "status", "state", "level", "severity",
  "severity_text", "name", "language", "role", "src", "dst",
];

/**
 * Build starter queries from the *actual* schema of the selected database,
 * so the chips always reference real tables/columns (not a fixed otel demo).
 */
function buildTemplates(schema: SchemaDoc | undefined, database: string): { label: string; q: string }[] {
  const db = schema?.databases.find((d) => d.name === database);
  if (!db || db.tables.length === 0) return [];

  const out: { label: string; q: string }[] = [];
  const t0 = db.tables[0].name;
  out.push({ label: `Sample ${t0}`, q: `${t0} | take 50` });
  out.push({ label: `Count ${t0}`, q: `${t0} | count` });

  // A group-by on the first categorical-looking column we find.
  for (const t of db.tables.slice(0, 5)) {
    const col = t.columns.find((c) => CATEGORICAL.includes(c.name.toLowerCase()));
    if (col) {
      out.push({
        label: `${t.name} by ${col.name}`,
        q: `${t.name} | summarize n=count() by ${col.name} | order by n desc`,
      });
      break;
    }
  }

  // A sample of a second table, if any.
  if (db.tables[1]) {
    out.push({ label: `Sample ${db.tables[1].name}`, q: `${db.tables[1].name} | take 50` });
  }
  return out.slice(0, 6);
}

export function SavedQueryChips({ onPick, schema, database }: Props) {
  const queries = useMemo(() => buildTemplates(schema, database), [schema, database]);
  if (queries.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-1.5 border-b border-muted/40 bg-muted/10 px-4 py-2">
      <span className="mr-1 shrink-0 self-center text-[10px] font-medium text-muted-foreground">Quick:</span>
      {queries.map((sq) => (
        <button
          key={sq.label}
          onClick={() => onPick(sq.q)}
          title={sq.q}
          className="inline-flex items-center rounded-full border border-muted/60 bg-background px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground shadow-sm transition-colors hover:border-primary/60 hover:bg-primary/5 hover:text-primary focus:outline-none focus:ring-2 focus:ring-primary/30"
        >
          {sq.label}
        </button>
      ))}
    </div>
  );
}
