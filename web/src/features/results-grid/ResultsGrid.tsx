import { useMemo, useRef, useState } from "react";
import { flexRender, getCoreRowModel, getSortedRowModel, useReactTable, type ColumnDef, type SortingState } from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Column } from "@/sdk/arrow";

export function ResultsGrid({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const defs = useMemo<ColumnDef<Record<string, unknown>>[]>(
    () => columns.map((c) => ({
      accessorKey: c.name,
      header: c.name,
      cell: (info) => String(info.getValue() ?? ""),
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

  return (
    <div ref={scrollRef} className="h-full overflow-auto text-xs font-mono">
      <table className="min-w-full">
        <thead className="sticky top-0 z-10 bg-background">
          {table.getHeaderGroups().map((hg) => (
            <tr key={hg.id} className="border-b">
              {hg.headers.map((h) => (
                <th
                  key={h.id}
                  className="cursor-pointer px-2 py-1 text-left hover:bg-accent/50"
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
            return (
              <tr key={row.id} style={{
                position: "absolute", top: 0, left: 0, width: "100%",
                transform: `translateY(${vi.start}px)`,
              }} className="border-b hover:bg-accent/30">
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
