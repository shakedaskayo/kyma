import { useState } from "react";
import { KymaDiscover } from "@kyma-ai/react/discover";

/**
 * Discover tab: KymaDiscover with onRowOpen -> side-panel JSON detail.
 * Also wires onExportKql to console.log so the KQL export path is exercised.
 */
export function DiscoverView() {
  const [openRow, setOpenRow] = useState<{
    row: Record<string, unknown>;
    source: string;
  } | null>(null);

  return (
    <div style={{ display: "flex", height: "100%" }}>
      <div style={{ flex: 1, overflow: "hidden" }}>
        <KymaDiscover
          defaultQuery=""
          style={{ height: "100%" }}
          onRowOpen={(row, source) => {
            console.log("[kyma-embed-demo] row opened", { source, row });
            setOpenRow({ row, source });
          }}
          onExportKql={(kql) => {
            console.log("[kyma-embed-demo] KQL export:", kql);
          }}
          onSearchChange={(_search) => {
            // Could sync search text to URL params here
          }}
          onScopeChange={(_scope) => {
            // Could sync scope to URL params here
          }}
        />
      </div>

      {openRow && (
        <div className="demo-side-panel">
          <h3>
            Row detail — <em>{openRow.source}</em>
            <button
              onClick={() => setOpenRow(null)}
              style={{
                float: "right",
                background: "none",
                border: "none",
                cursor: "pointer",
                fontSize: 16,
                color: "#64748b",
              }}
            >
              ×
            </button>
          </h3>
          <pre>{JSON.stringify(openRow.row, null, 2)}</pre>
        </div>
      )}
    </div>
  );
}
