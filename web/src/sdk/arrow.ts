import type { RecordBatch, Field } from "apache-arrow";
import { DataType } from "apache-arrow";

export type ColKind = "time" | "numeric" | "string" | "bool" | "other";
export type Column = { name: string; kind: ColKind };

export type ChartSpec =
  | { type: "line"    ; x: string; y: string }
  | { type: "bar"     ; x: string; y: string }
  | { type: "scatter" ; x: string; y: string }
  | { type: "stat"    ; y: string }
  | { type: "none"    };

export function columnKind(f: Field): ColKind {
  const t = f.type;
  if (DataType.isTimestamp(t) || DataType.isDate(t))      return "time";
  if (DataType.isInt(t) || DataType.isFloat(t) || DataType.isDecimal(t)) return "numeric";
  if (DataType.isUtf8(t) || DataType.isLargeUtf8(t))      return "string";
  if (DataType.isBool(t))                                  return "bool";
  return "other";
}

export function inferColumnTypes(batch: RecordBatch): Column[] {
  return batch.schema.fields.map((f) => ({ name: f.name, kind: columnKind(f) }));
}

export function batchToRows(batch: RecordBatch): { columns: Column[]; rows: Record<string, unknown>[] } {
  const columns = inferColumnTypes(batch);
  const rows: Record<string, unknown>[] = [];
  for (let i = 0; i < batch.numRows; i++) {
    const row: Record<string, unknown> = {};
    for (const c of columns) {
      const v = batch.getChild(c.name)?.get(i);
      row[c.name] = v instanceof Date ? v.toISOString() : typeof v === "bigint" ? Number(v) : v;
    }
    rows.push(row);
  }
  return { columns, rows };
}

export function autoChartAxes(cols: Column[]): ChartSpec {
  const time    = cols.find((c) => c.kind === "time");
  const numerics = cols.filter((c) => c.kind === "numeric");
  const strings  = cols.filter((c) => c.kind === "string");
  if (time && numerics[0])          return { type: "line",    x: time.name,       y: numerics[0].name };
  if (strings[0] && numerics[0])    return { type: "bar",     x: strings[0].name, y: numerics[0].name };
  if (numerics.length >= 2)         return { type: "scatter", x: numerics[0].name, y: numerics[1].name };
  if (numerics.length === 1 && cols.length === 1) return { type: "stat", y: numerics[0].name };
  return { type: "none" };
}
