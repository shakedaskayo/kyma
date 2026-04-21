import { useMemo, useRef, useState } from "react";
import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type SortingState } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Column, ColKind } from "@/sdk/arrow";

// ── helpers ──────────────────────────────────────────────────────────────────

const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

/** Format an ISO-8601 timestamp as "MMM dd HH:mm:ss" (no library) */
export function fmtIso(raw: string): string {
  const d = new Date(raw);
  if (isNaN(d.getTime())) return raw;
  const mon = MONTHS[d.getMonth()];
  const day = String(d.getDate()).padStart(2, "0");
  const hh  = String(d.getHours()).padStart(2, "0");
  const mm  = String(d.getMinutes()).padStart(2, "0");
  const ss  = String(d.getSeconds()).padStart(2, "0");
  return `${mon} ${day} ${hh}:${mm}:${ss}`;
}

const ISO_RE = /^\d{4}-\d{2}-\d{2}T[\d:.Z+-]+$/;

/** Truncate a string to maxLen, appending "…" if needed */
function truncate(s: string, maxLen = 120): string {
  return s.length > maxLen ? s.slice(0, maxLen - 3) + "…" : s;
}

// ── type badge ────────────────────────────────────────────────────────────────

const KIND_BADGE: Record<ColKind, { cls: string; label: string }> = {
  time:    { cls: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300",       label: "time" },
  numeric: { cls: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300", label: "num" },
  string:  { cls: "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300",      label: "str" },
  bool:    { cls: "bg-violet-100 text-violet-700 dark:bg-violet-900/40 dark:text-violet-300", label: "bool" },
  other:   { cls: "bg-slate-50 text-slate-500 dark:bg-slate-800/50 dark:text-slate-400",    label: "·" },
};

function TypeBadge({ kind }: { kind: ColKind }) {
  const { cls, label } = KIND_BADGE[kind];
  return (
    <span className={`ml-1.5 inline-block rounded-full px-1.5 py-0 text-[9px] font-normal leading-4 ${cls}`}>
      {label}
    </span>
  );
}

// ── cell renderer ─────────────────────────────────────────────────────────────

function CellValue({ value, kind }: { value: unknown; kind: ColKind }) {
  if (value === null || value === undefined) {
    return <span className="italic text-muted-foreground">null</span>;
  }
  if (kind === "time" && typeof value === "string" && ISO_RE.test(value)) {
    return <span title={value}>{fmtIso(value)}</span>;
  }
  const str = String(value);
  if (typeof value === "string" && str.length > 120) {
    return <span title={str}>{truncate(str)}</span>;
  }
  return <>{str}</>;
}

// ── component ─────────────────────────────────────────────────────────────────

export function ResultsGrid({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [scrolled, setScrolled] = useState(false);

  const defs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () => columns.map((c) => ({
      accessorKey: c.name,
      header: () => (
        <span className="inline-flex items-center">
          {c.name}
          <TypeBadge kind={c.kind} />
        </span>
      ),
      cell: (info) => <CellValue value={info.getValue()} kind={c.kind} />,
    })),
    [columns],
  );

  const table = useReactTable({
    data: rows,
    columns: defs,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: table.getRowModel().rows.length,
    estimateSize: () => 28,
    getScrollElement: () => scrollRef.current,
    overscan: 12,
  });

  const handleScroll = () => {
    setScrolled((scrollRef.current?.scrollTop ?? 0) > 0);
  };

  return (
    <div ref={scrollRef} onScroll={handleScroll} className="h-full overflow-auto text-xs font-mono">
      <table className="min-w-full">
        <thead className={`sticky top-0 z-10 bg-background transition-shadow ${scrolled ? "shadow-sm" : ""}`}>
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="border-b-2">
              {hg.headers.map((h) => (
                <th
                  key={h.id}
                  className="cursor-pointer whitespace-nowrap px-2 py-1 text-left text-xs font-medium hover:bg-accent/50"
                  onClick={h.column.getToggleSortingHandler()}>
                  {flexRender(h.column.columnDef.header, h.getContext())}
                  {({ asc: " ▲", desc: " ▼" } as Record<string, string>)[h.column.getIsSorted() as string] ?? ""}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const row = table.getRowModel().rows[vi.index];
            const isOdd = vi.index % 2 === 1;
            return (
              <tr
                key={row.id}
                style={{
                  position: "absolute", top: 0, left: 0, width: "100%",
                  transform: `translateY(${vi.start}px)`,
                }}
                className={`border-b hover:bg-accent/30 ${isOdd ? "bg-muted/20" : ""}`}
              >
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="px-2 py-1">
                    {flexRender(cell.column.columnDef.cell, cell.getContext())}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function exportCsv(columns: Column[], rows: Record<string, unknown>[]): Blob {
  const header = columns.map((c) => csvCell(c.name)).join(",");
  const body = rows.map((r) => columns.map((c) => csvCell(String(r[c.name] ?? ""))).join(",")).join("\n");
  return new Blob([header, "\n", body], { type: "text/csv" });
}

function csvCell(s: string): string {
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}
