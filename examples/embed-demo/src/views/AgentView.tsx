import { useState } from "react";
import { KymaAgentChat } from "@kyma-ai/react/agent";

/**
 * Agent tab: KymaAgentChat with onMessage callback logged to an on-screen log.
 * Demonstrates that onMessage fires once per completed assistant turn.
 *
 * Note: agent and MCP endpoints return 403 for database-scoped tokens
 * (fail-closed by design). Use a full-access token when testing this view.
 */
export function AgentView() {
  const [log, setLog] = useState<string[]>([]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        style={{
          padding: "6px 16px",
          background: "#fafafa",
          borderBottom: "1px solid #e2e8f0",
          display: "flex",
          alignItems: "center",
          gap: 8,
        }}
      >
        <span className="demo-label">Completed turns:</span>
        <span className="demo-badge">{log.length}</span>
        {log.length > 0 && (
          <button
            className="demo-btn demo-btn-secondary"
            style={{ fontSize: 11, padding: "2px 8px" }}
            onClick={() => setLog([])}
          >
            Clear log
          </button>
        )}
      </div>

      <div style={{ flex: 1, overflow: "hidden" }}>
        <KymaAgentChat
          placeholder="Ask a question about your data…"
          style={{ height: "100%" }}
          onMessage={(m) => {
            console.log("[kyma-embed-demo] agent message:", m);
            setLog((prev) => [
              ...prev,
              `[${new Date().toLocaleTimeString()}] ${m.role}: ${m.text.slice(0, 120)}${m.text.length > 120 ? "…" : ""}`,
            ]);
          }}
        />
      </div>

      {log.length > 0 && (
        <div style={{ padding: "8px 16px", borderTop: "1px solid #e2e8f0" }}>
          <div className="demo-label" style={{ marginBottom: 4 }}>
            Message log (onMessage callback output):
          </div>
          <div className="demo-message-log">{log.join("\n")}</div>
        </div>
      )}
    </div>
  );
}
