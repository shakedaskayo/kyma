import { useState, useCallback, type ChangeEvent } from "react";
import { PensieveProvider } from "@pensieve-ai/react";
import { pensieveDark, pensieveLight } from "@pensieve-ai/react";
import type { PensieveTheme } from "@pensieve-ai/react";
import type { PensieveAuth } from "@pensieve-ai/client";

import { GraphView } from "./views/GraphView";
import { QueryView } from "./views/QueryView";
import { DiscoverView } from "./views/DiscoverView";
import { DashboardsView } from "./views/DashboardsView";
import { AgentView } from "./views/AgentView";
import { HeadlessView } from "./views/HeadlessView";

// ── Local-storage persistence helpers ─────────────────────────────────────────

const LS_ENDPOINT = "pensieve-demo:endpoint";
const LS_TOKEN = "pensieve-demo:token";
const LS_DATABASE = "pensieve-demo:database";
const LS_USE_MINT = "pensieve-demo:use-mint";

function ls(key: string, fallback = ""): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function lsSet(key: string, val: string): void {
  try {
    localStorage.setItem(key, val);
  } catch {
    /* ignore */
  }
}

// ── Theme definitions ──────────────────────────────────────────────────────────

type ThemeMode = "dark" | "light" | "custom";

/**
 * "custom" theme — pensieveDark base with a purple accent + rounder radius.
 * Demonstrates Partial<PensieveTheme> override pattern.
 */
const pensieveCustom: Partial<PensieveTheme> = {
  ...pensieveDark,
  primary: "270 70% 65%",
  primaryForeground: "270 10% 10%",
  accent: "270 30% 20%",
  accentForeground: "270 10% 96%",
  brandFrom: "270 75% 65%",
  brandTo: "290 80% 70%",
  ring: "270 70% 60%",
  radius: "1rem",
};

type TabId = "graph" | "query" | "discover" | "dashboards" | "agent" | "headless";

const TABS: Array<{ id: TabId; label: string }> = [
  { id: "graph", label: "Graph" },
  { id: "query", label: "Query" },
  { id: "discover", label: "Discover" },
  { id: "dashboards", label: "Dashboards" },
  { id: "agent", label: "Agent" },
  { id: "headless", label: "Headless (L3)" },
];

// ── App ───────────────────────────────────────────────────────────────────────

export function App() {
  // Connection state
  const [endpoint, setEndpoint] = useState(ls(LS_ENDPOINT, "http://localhost:8080"));
  const [token, setToken] = useState(ls(LS_TOKEN));
  const [database, setDatabase] = useState(ls(LS_DATABASE));
  const [useMint, setUseMint] = useState(ls(LS_USE_MINT) === "1");
  const [connected, setConnected] = useState(false);

  // Committed connection (applied on Connect)
  const [activeEndpoint, setActiveEndpoint] = useState("");
  const [activeAuth, setActiveAuth] = useState<PensieveAuth | null>(null);
  const [activeDatabase, setActiveDatabase] = useState<string | undefined>(undefined);

  // UI state
  const [tab, setTab] = useState<TabId>("graph");
  const [themeMode, setThemeMode] = useState<ThemeMode>("dark");

  // ── Connection ──────────────────────────────────────────────────────────────

  const handleConnect = useCallback(() => {
    lsSet(LS_ENDPOINT, endpoint);
    lsSet(LS_TOKEN, token);
    lsSet(LS_DATABASE, database);
    lsSet(LS_USE_MINT, useMint ? "1" : "0");

    let auth: PensieveAuth;
    if (useMint) {
      /**
       * Multi-tenant mint-server pattern:
       * The host app's server mints a short-lived token from its IdP
       * (or from PENSIEVE_TOKEN for dev). The SDK never sees the raw credential.
       *
       * Production variant: replace with your OIDC token endpoint, e.g.:
       *   getToken: () => fetch("/api/auth/pensieve-token").then(r => r.text())
       *
       * OIDC variant: mint a JWT with pensieve_role/pensieve_databases claims
       * signed by an issuer listed in PENSIEVE_OIDC_ISSUERS on the server.
       */
      auth = {
        getToken: () =>
          fetch("http://localhost:8788/api/pensieve-token").then((r) => {
            if (!r.ok) throw new Error(`mint-server returned ${r.status}`);
            return r.text();
          }),
      };
    } else {
      auth = { token };
    }

    setActiveEndpoint(endpoint);
    setActiveAuth(auth);
    setActiveDatabase(database.trim() || undefined);
    setConnected(true);
  }, [endpoint, token, database, useMint]);

  const handleDisconnect = useCallback(() => {
    setConnected(false);
    setActiveAuth(null);
    setActiveEndpoint("");
    setActiveDatabase(undefined);
  }, []);

  // ── Theme resolution ────────────────────────────────────────────────────────

  const resolvedTheme: Partial<PensieveTheme> =
    themeMode === "light"
      ? pensieveLight
      : themeMode === "custom"
        ? pensieveCustom
        : pensieveDark;

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="demo-shell">
      {/* ── Topbar ─────────────────────────────────────────────────────────── */}
      <div className="demo-topbar">
        <span className="demo-topbar-title">Pensieve Embed Demo</span>

        <label className="demo-label">Endpoint</label>
        <input
          className="demo-input"
          type="url"
          value={endpoint}
          onChange={(e: ChangeEvent<HTMLInputElement>) => setEndpoint(e.target.value)}
          placeholder="http://localhost:8080"
          disabled={connected}
          style={{ minWidth: 220 }}
        />

        <label className="demo-label">Database (opt.)</label>
        <input
          className="demo-input"
          type="text"
          value={database}
          onChange={(e: ChangeEvent<HTMLInputElement>) => setDatabase(e.target.value)}
          placeholder="default"
          disabled={connected}
          style={{ minWidth: 120 }}
        />

        {!useMint && (
          <>
            <label className="demo-label">Token</label>
            <input
              className="demo-input"
              type="password"
              value={token}
              onChange={(e: ChangeEvent<HTMLInputElement>) => setToken(e.target.value)}
              placeholder="pensieve token …"
              disabled={connected}
              style={{ minWidth: 160 }}
            />
          </>
        )}

        <label className="demo-toggle-row demo-label">
          <input
            type="checkbox"
            checked={useMint}
            onChange={(e) => setUseMint(e.target.checked)}
            disabled={connected}
          />
          Use mint server
        </label>

        {!connected ? (
          <button className="demo-btn" onClick={handleConnect}>
            Connect
          </button>
        ) : (
          <button className="demo-btn demo-btn-secondary" onClick={handleDisconnect}>
            Disconnect
          </button>
        )}

        <span style={{ flex: 1 }} />

        <label className="demo-label">Theme</label>
        <select
          className="demo-select"
          value={themeMode}
          onChange={(e) => setThemeMode(e.target.value as ThemeMode)}
        >
          <option value="dark">pensieveDark</option>
          <option value="light">pensieveLight</option>
          <option value="custom">Custom (purple + rounded)</option>
        </select>
      </div>

      {/* ── Tab nav ────────────────────────────────────────────────────────── */}
      {connected && (
        <nav className="demo-nav">
          {TABS.map((t) => (
            <button
              key={t.id}
              className={`demo-nav-btn${tab === t.id ? " active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      )}

      {/* ── Main ───────────────────────────────────────────────────────────── */}
      <main className="demo-main">
        {!connected || !activeAuth ? (
          <UnconnectedInstructions />
        ) : (
          /*
           * PensieveProvider is keyed on endpoint|database so switching either
           * tears down and recreates the isolated React Query + Zustand
           * stores — no stale data from a previous connection leaks through.
           */
          <PensieveProvider
            key={`${activeEndpoint}|${activeDatabase ?? ""}`}
            endpoint={activeEndpoint}
            auth={activeAuth}
            {...(activeDatabase !== undefined ? { database: activeDatabase } : {})}
            theme={resolvedTheme}
          >
            <div className="demo-view demo-full-height">
              {tab === "graph" && <GraphView />}
              {tab === "query" && <QueryView />}
              {tab === "discover" && <DiscoverView />}
              {tab === "dashboards" && <DashboardsView />}
              {tab === "agent" && <AgentView />}
              {tab === "headless" && <HeadlessView />}
            </div>
          </PensieveProvider>
        )}
      </main>
    </div>
  );
}

function UnconnectedInstructions() {
  return (
    <div className="demo-instructions">
      <h2>Pensieve Embed Demo</h2>
      <p>
        Enter the endpoint URL and a token (or enable the mint server), then click{" "}
        <strong>Connect</strong> to start exploring the five embeddable Pensieve components.
      </p>
      <p>
        Prerequisites: a running Pensieve server with{" "}
        <code>PENSIEVE_CORS_ALLOWED_ORIGINS=http://localhost:5173</code> and a valid bearer token.
      </p>
      <p>
        For the <em>mint server</em> pattern, run{" "}
        <code>PENSIEVE_TOKEN=... pnpm --filter pensieve-embed-demo mint-token</code> in a separate
        terminal.
      </p>
    </div>
  );
}
