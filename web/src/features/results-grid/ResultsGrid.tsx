import { useMemo, useRef, useState, useCallback } from "react";
import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type SortingState } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRight } from "lucide-react";
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
  time:    { cls: "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300",              label: "time" },
  numeric: { cls: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300",  label: "num"  },
  string:  { cls: "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300",             label: "str"  },
  bool:    { cls: "bg-violet-100 text-violet-700 dark:bg-violet-900/40 dark:text-violet-300",      label: "bool" },
  other:   { cls: "bg-slate-50 text-slate-500 dark:bg-slate-800/50 dark:text-slate-400",           label: "·"    },
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

function CellValue({ value, kind, isFirst }: { value: unknown; kind: ColKind; isFirst?: boolean }) {
  if (value === null || value === undefined) {
    return <span className="italic text-muted-foreground/60">null</span>;
  }
  if (kind === "time" && typeof value === "string" && ISO_RE.test(value)) {
    return (
      <span title={value} className="font-mono tabular-nums text-slate-600 dark:text-slate-300">
        {fmtIso(value)}
      </span>
    );
  }
  const str = String(value);
  const truncated = str.length > 120 ? truncate(str) : str;

  if (isFirst) {
    return <span className="font-semibold text-foreground">{truncated}</span>;
  }
  if (kind === "numeric") {
    return <span className="font-mono tabular-nums">{truncated}</span>;
  }
  return <span className={str !== truncated ? "cursor-help" : ""} title={str !== truncated ? str : undefined}>{truncated}</span>;
}

// ── component ─────────────────────────────────────────────────────────────────

const EXPAND_COL_WIDTH = 22;

export function ResultsGrid({
  columns,
  rows,
  filter,
}: {
  columns: Column[];
  rows: Record<string, unknown>[];
  filter?: string;
}) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [scrolled, setScrolled] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());

  // Client-side filter: substring across all string cells
  const filteredRows = useMemo(() => {
    if (!filter?.trim()) return rows;
    const needle = filter.trim().toLowerCase();
    return rows.filter((row) =>
      columns.some((col) => {
        const v = row[col.name];
        if (v === null || v === undefined) return false;
        return String(v).toLowerCase().includes(needle);
      }),
    );
  }, [rows, columns, filter]);

  const toggleRow = useCallback((idx: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }, []);

  const defs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () =>
      columns.map((c, ci) => ({
        accessorKey: c.name,
        header: () => (
          <span className="inline-flex items-center">
            {c.name}
            <TypeBadge kind={c.kind} />
          </span>
        ),
        cell: (info) => (
          <CellValue value={info.getValue()} kind={c.kind} isFirst={ci === 0} />
        ),
      })),
    [columns],
  );

  const table = useReactTable({
    data: filteredRows,
    columns: defs,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const tableRows = table.getRowModel().rows;
  const colCount = columns.length + 1; // +1 for expand column

  // For virtualizer we count "logical rows" = data rows + expansion rows
  const logicalCount = useMemo(() => {
    let n = tableRows.length;
    for (let i = 0; i < tableRows.length; i++) {
      if (expanded.has(i)) n++;
    }
    return n;
  }, [tableRows.length, expanded]);

  // Map virtual index → { kind: "data" | "expand", dataIndex }
  const rowMap = useMemo(() => {
    const map: Array<{ kind: "data" | "expand"; dataIndex: number }> = [];
    for (let i = 0; i < tableRows.length; i++) {
      map.push({ kind: "data", dataIndex: i });
      if (expanded.has(i)) map.push({ kind: "expand", dataIndex: i });
    }
    return map;
  }, [tableRows.length, expanded]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: logicalCount,
    estimateSize: (vi) => {
      const item = rowMap[vi];
      if (!item) return 25;
      return item.kind === "expand" ? 160 : 25;
    },
    getScrollElement: () => scrollRef.current,
    overscan: 10,
  });

  const handleScroll = () => {
    setScrolled((scrollRef.current?.scrollTop ?? 0) > 0);
  };

  return (
    <div ref={scrollRef} onScroll={handleScroll} className="h-full overflow-auto text-xs">
      <table className="min-w-full border-collapse">
        <thead className={`sticky top-0 z-10 bg-background transition-shadow ${scrolled ? "shadow-sm" : ""}`}>
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="border-b border-muted/60">
              {/* Expand toggle column */}
              <th style={{ width: EXPAND_COL_WIDTH, minWidth: EXPAND_COL_WIDTH }} className="px-1" />
              {hg.headers.map((h, hi) => (
                <th
                  key={h.id}
                  className={`cursor-pointer whitespace-nowrap px-3 py-1 text-left text-[11px] font-semibold text-slate-600 dark:text-slate-300 hover:bg-accent/40 transition-colors ${hi === 0 ? "font-bold" : ""}`}
                  onClick={h.column.getToggleSortingHandler()}
                >
                  {flexRender(h.column.columnDef.header, h.getContext())}
                  {({ asc: " ▲", desc: " ▼" } as Record<string, string>)[h.column.getIsSorted() as string] ?? ""}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((vi) => {
            const item = rowMap[vi.index];
            if (!item) return null;
            const { kind, dataIndex } = item;
            const row = tableRows[dataIndex];

            if (kind === "expand") {
              return (
                <tr
                  key={`expand-${dataIndex}`}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${vi.start}px)`,
                    height: vi.size,
                  }}
                >
                  <td colSpan={colCount} className="px-0 py-0">
                    <pre className="bg-muted/40 p-3 text-xs font-mono whitespace-pre overflow-x-auto border-b border-muted/50 leading-relaxed text-slate-700 dark:text-slate-300">
                      {JSON.stringify(filteredRows[dataIndex], null, 2)}
                    </pre>
                  </td>
                </tr>
              );
            }

            const isExpanded = expanded.has(dataIndex);
            return (
              <tr
                key={row.id}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${vi.start}px)`,
                  height: vi.size,
                }}
                className={`border-b border-muted/50 hover:bg-accent/20 transition-colors cursor-pointer ${isExpanded ? "bg-accent/10" : ""}`}
                onClick={() => toggleRow(dataIndex)}
              >
                {/* Expand chevron cell */}
                <td
                  style={{ width: EXPAND_COL_WIDTH, minWidth: EXPAND_COL_WIDTH }}
                  className="px-1 text-muted-foreground"
                  onClick={(e) => { e.stopPropagation(); toggleRow(dataIndex); }}
                >
                  <ChevronRight
                    className="h-3 w-3 transition-transform"
                    style={{ transform: isExpanded ? "rotate(90deg)" : "rotate(0deg)" }}
                  />
                </td>
                {row.getVisibleCells().map((cell) => (
                  <td key={cell.id} className="px-3 py-0 leading-[25px] truncate max-w-xs">
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
