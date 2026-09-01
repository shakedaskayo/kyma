import { useState } from "react";
import { PensieveQueryEditor } from "@pensieve-ai/react/query";
import type { Column } from "@pensieve-ai/react";

/**
 * Query tab: PensieveQueryEditor with a default KQL query.
 * onResults updates a row-count badge proving the callback fires.
 */
export function QueryView() {
  const [rowCount, setRowCount] = useState<number | null>(null);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {rowCount !== null && (
        <div
          style={{
            padding: "6px 16px",
            background: "#f0fdf4",
            borderBottom: "1px solid #bbf7d0",
            display: "flex",
            alignItems: "center",
            gap: 8,
          }}
        >
          <span className="demo-label">Last result:</span>
          <span className="demo-badge">{rowCount} rows</span>
        </div>
      )}

      <div style={{ flex: 1, overflow: "hidden" }}>
        <PensieveQueryEditor
          language="kql"
          defaultQuery="// Edit this query and press Run\ntrace\n| limit 20"
          showSchemaBrowser
          showResults
          showChart
          showTimeRange
          style={{ height: "100%" }}
          onResults={({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) => {
            void columns; // columns available for further processing
            setRowCount(rows.length);
          }}
          onQueryChange={(_q: string) => {
            // Could persist query externally here
          }}
        />
      </div>
    </div>
  );
}
