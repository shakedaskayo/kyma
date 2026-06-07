import { useState } from "react";
import { KymaDashboard } from "@kyma-ai/react/dashboards";
import { useKymaDashboards } from "@kyma-ai/react";
import type { DashboardWithPanels } from "@kyma-ai/react";

/**
 * Dashboards tab: lists dashboards via useKymaDashboards hook,
 * lets the user pick one by ID or from the list, then renders KymaDashboard.
 *
 * Demonstrates:
 * - useKymaDashboards headless hook for listing
 * - KymaDashboard with onPanelClick, onSaveSuccess, onSaveError callbacks
 * - dashboardId controlled by host state
 */
export function DashboardsView() {
  const [dashboardId, setDashboardId] = useState("");
  const [inputId, setInputId] = useState("");
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  const { dashboards, isLoading } = useKymaDashboards();

  const handleLoad = () => {
    const id = inputId.trim();
    if (id) setDashboardId(id);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="demo-dashboard-toolbar">
        <span className="demo-label">Dashboard ID</span>
        <input
          className="demo-input"
          type="text"
          value={inputId}
          onChange={(e) => setInputId(e.target.value)}
          placeholder="paste dashboard id …"
          style={{ minWidth: 260 }}
          onKeyDown={(e) => e.key === "Enter" && handleLoad()}
        />
        <button className="demo-btn" onClick={handleLoad} disabled={!inputId.trim()}>
          Load
        </button>

        {isLoading ? (
          <span className="demo-loading">Loading list…</span>
        ) : dashboards.length > 0 ? (
          <>
            <span className="demo-label">or pick:</span>
            <select
              className="demo-select"
              value=""
              onChange={(e) => {
                const id = e.target.value;
                if (id) {
                  setInputId(id);
                  setDashboardId(id);
                }
              }}
            >
              <option value="">— select —</option>
              {dashboards.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </select>
          </>
        ) : (
          <span className="demo-loading">No dashboards found.</span>
        )}

        {saveMsg && (
          <span className="demo-badge" style={{ marginLeft: "auto" }}>
            {saveMsg}
          </span>
        )}
      </div>

      <div className="demo-dashboard-view" style={{ flex: 1, overflow: "auto" }}>
        {dashboardId ? (
          <KymaDashboard
            key={dashboardId}
            dashboardId={dashboardId}
            editable={false}
            onPanelClick={(panelId) => {
              console.log("[kyma-embed-demo] panel clicked:", panelId);
            }}
            onSaveSuccess={(dashboard: DashboardWithPanels) => {
              setSaveMsg(`Saved: ${dashboard.name}`);
              setTimeout(() => setSaveMsg(null), 3000);
            }}
            onSaveError={(err) => {
              console.error("[kyma-embed-demo] save error:", err);
              setSaveMsg("Save failed");
              setTimeout(() => setSaveMsg(null), 3000);
            }}
          />
        ) : (
          <div className="demo-instructions" style={{ height: "auto", paddingTop: 60 }}>
            <p>Enter a dashboard ID above or pick one from the list to render it here.</p>
          </div>
        )}
      </div>
    </div>
  );
}
