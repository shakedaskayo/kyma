import { useState, type ChangeEvent } from "react";
import { usePensieveGraph, usePensieveQuery } from "@pensieve-ai/react";
import { usePensieveClient } from "@pensieve-ai/react";

/**
 * Headless tab (L3 proof):
 * Builds a completely custom UI using usePensieveGraph + usePensieveQuery hooks
 * without rendering any Pensieve component. Proves the headless layer is usable
 * independently of the opinionated component shells.
 *
 * usePensieveGraph: shows node/edge counts + label distribution.
 * usePensieveQuery: lets the user run arbitrary KQL and shows raw results in a table.
 */
export function HeadlessView() {
  return (
    <div className="demo-headless">
      <div>
        <h2>Headless API (L3)</h2>
        <p>
          This tab uses <code>usePensieveGraph</code> and <code>usePensieveQuery</code> hooks directly —
          no Pensieve components rendered. Proves that host apps can build fully custom UIs against the
          headless layer.
        </p>
      </div>

      <GraphStats />
      <QueryBuilder />
    </div>
  );
}

// ── Graph stats panel ──────────────────────────────────────────────────────────

function GraphStats() {
  const { nodes, edges, stats, isLoading, isError, error, refetch } = usePensieveGraph({
    discover: "all-databases",
  });

  return (
    <div className="demo-headless-card">
      <h3>
        usePensieveGraph — all databases{" "}
        <button
          className="demo-btn demo-btn-secondary"
          style={{ fontSize: 11, padding: "2px 8px", float: "right" }}
          onClick={refetch}
        >
          Refresh
        </button>
      </h3>

      {isLoading && <div className="demo-loading">Loading graph data…</div>}
      {isError && (
        <div className="demo-error">
          Failed to load graph: {String(error instanceof Error ? error.message : error)}
        </div>
      )}

      {!isLoading && !isError && (
        <>
          <div className="demo-stat-grid" style={{ marginBottom: 16 }}>
            <div className="demo-stat-cell">
              <div className="stat-val">{nodes.length}</div>
              <div className="stat-key">Nodes</div>
            </div>
            <div className="demo-stat-cell">
              <div className="stat-val">{edges.length}</div>
              <div className="stat-key">Edges</div>
            </div>
            <div className="demo-stat-cell">
              <div className="stat-val">
                {stats ? Object.keys(stats.label_counts).length : 0}
              </div>
              <div className="stat-key">Label types</div>
            </div>
            <div className="demo-stat-cell">
              <div className="stat-val">
                {stats ? Object.keys(stats.relationship_type_counts).length : 0}
              </div>
              <div className="stat-key">Rel. types</div>
            </div>
          </div>

          {stats && Object.keys(stats.label_counts).length > 0 && (
            <>
              <div className="demo-label" style={{ marginBottom: 6 }}>
                Label distribution
              </div>
              <table className="demo-table">
                <thead>
                  <tr>
                    <th>Label</th>
                    <th>Count</th>
                  </tr>
                </thead>
                <tbody>
                  {Object.entries(stats.label_counts)
                    .sort(([, a], [, b]) => b - a)
                    .slice(0, 10)
                    .map(([label, count]) => (
                      <tr key={label}>
                        <td>{label}</td>
                        <td>{count}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </>
          )}

          {nodes.length === 0 && !isLoading && (
            <div className="demo-loading">No graph data found — check that graphs exist in this deployment.</div>
          )}
        </>
      )}
    </div>
  );
}

// ── Custom query builder ───────────────────────────────────────────────────────

function QueryBuilder() {
  const client = usePensieveClient();
  const defaultDb = client.transport.database ?? "default";

  const [query, setQuery] = useState("trace\n| limit 10");
  const [database, setDatabase] = useState(defaultDb);
  const { columns, rows, isRunning, error, execute, cancel } = usePensieveQuery();

  const handleRun = () => {
    void execute({ database, query, language: "kql" });
  };

  return (
    <div className="demo-headless-card">
      <h3>usePensieveQuery — custom query UI</h3>

      <div className="demo-query-input">
        <input
          type="text"
          value={database}
          onChange={(e: ChangeEvent<HTMLInputElement>) => setDatabase(e.target.value)}
          placeholder="database"
          style={{ maxWidth: 140 }}
        />
        <input
          type="text"
          value={query}
          onChange={(e: ChangeEvent<HTMLInputElement>) => setQuery(e.target.value)}
          placeholder="KQL query …"
          onKeyDown={(e) => e.key === "Enter" && !isRunning && handleRun()}
        />
        {isRunning ? (
          <button className="demo-btn demo-btn-secondary" onClick={cancel}>
            Cancel
          </button>
        ) : (
          <button className="demo-btn" onClick={handleRun} disabled={!query.trim()}>
            Run
          </button>
        )}
      </div>

      {isRunning && <div className="demo-loading">Running…</div>}

      {Boolean(error) && (
        <div className="demo-error">
          {String(error instanceof Error ? error.message : String(error))}
        </div>
      )}

      {!isRunning && rows.length > 0 && (
        <>
          <div className="demo-label" style={{ marginBottom: 6 }}>
            {rows.length} row{rows.length !== 1 ? "s" : ""} — {columns.length} column
            {columns.length !== 1 ? "s" : ""}
          </div>
          <div style={{ overflowX: "auto" }}>
            <table className="demo-table">
              <thead>
                <tr>
                  {columns.map((c) => (
                    <th key={c.name}>{c.name}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 50).map((row, i) => (
                  <tr key={i}>
                    {columns.map((c) => (
                      <td key={c.name}>
                        {row[c.name] === null || row[c.name] === undefined
                          ? ""
                          : String(row[c.name]).slice(0, 80)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length > 50 && (
              <div className="demo-label" style={{ padding: "6px 10px" }}>
                Showing 50 of {rows.length} rows.
              </div>
            )}
          </div>
        </>
      )}

      {!isRunning && rows.length === 0 && !error && (
        <div className="demo-loading">No results yet — press Run.</div>
      )}
    </div>
  );
}
