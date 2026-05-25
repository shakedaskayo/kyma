import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { Database, Settings as SettingsIcon, Compass, LayoutDashboard, Sparkles, Network } from "lucide-react";
import { useEffect } from "react";
import { useSession } from "@/sdk/session";
import { useHealth } from "@/sdk/reconnect";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { ReconnectBanner } from "@/features/reconnect/ReconnectBanner";

// ── query status pill ─────────────────────────────────────────────────────────

function QueryStatusPill() {
  const activeTab = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId));
  if (!activeTab) return null;

  const r = activeTab.results;

  if (r.kind === "idle") return null;

  if (r.kind === "running") {
    return (
      <div className="flex items-center gap-1.5 rounded-full border border-amber-300 bg-amber-50 px-2 py-0.5 text-[11px] text-amber-700 dark:border-amber-700/50 dark:bg-amber-950/40 dark:text-amber-400">
        <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-amber-500" />
        streaming
      </div>
    );
  }

  if (r.kind === "ok") {
    return (
      <div className="flex items-center gap-1.5 rounded-full border border-emerald-300 bg-emerald-50 px-2 py-0.5 text-[11px] tabular-nums text-emerald-700 dark:border-emerald-700/50 dark:bg-emerald-950/40 dark:text-emerald-400">
        <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
        {r.rowCount.toLocaleString()} rows · {r.durationMs.toFixed(0)} ms
      </div>
    );
  }

  if (r.kind === "error") {
    return (
      <div className="flex items-center gap-1.5 rounded-full border border-destructive/40 bg-destructive/5 px-2 py-0.5 text-[11px] text-destructive">
        <span className="h-1.5 w-1.5 rounded-full bg-destructive" />
        error
      </div>
    );
  }

  return null;
}

// ── connection dot ────────────────────────────────────────────────────────────

function ConnectionDot() {
  const online = useHealth((s) => s.online);
  return (
    <span
      title={online ? "connected" : "disconnected"}
      className={`relative inline-block h-2 w-2 rounded-full ring-2 ${
        online
          ? "bg-emerald-500 ring-emerald-300 dark:ring-emerald-700"
          : "bg-red-500 ring-red-300 dark:ring-red-700"
      }`}
    />
  );
}

// ── shell ─────────────────────────────────────────────────────────────────────

export function Shell() {
  const { endpoint, database } = useSession();
  const active = useRouterState().location.pathname;
  const startHealth = useHealth((s) => s.start);
  useEffect(() => {
    if (!endpoint) return;
    return startHealth(endpoint);
  }, [endpoint, startHealth]);

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex h-12 items-center gap-3 border-b px-4 text-sm">
        <div className="font-semibold tracking-tight">kyma</div>
        <div className="flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs">
          <Database className="h-3.5 w-3.5" /> {database || "—"}
        </div>
        <QueryStatusPill />
        <nav className="ml-auto flex items-center gap-1">
          <Link to="/explore" search={{ q: undefined }} className={btn(active.startsWith("/explore"))}>
            <Compass className="h-4 w-4" /> Explore
          </Link>
          <Link to="/dashboards" className={btn(active.startsWith("/dashboards"))}>
            <LayoutDashboard className="h-4 w-4" /> Dashboards
          </Link>
          <Link to="/agent" className={btn(active.startsWith("/agent"))}>
            <Sparkles className="h-4 w-4" /> Ask Kyma
          </Link>
          <Link to="/graph" className={btn(active.startsWith("/graph"))}>
            <Network className="h-4 w-4" /> Graph
          </Link>
          <Link to="/settings" search={{ next: active }} className={btn(active === "/settings")}>
            <SettingsIcon className="h-4 w-4" /> Settings
          </Link>
        </nav>
        <div className="ml-2 flex items-center gap-1.5">
          <ConnectionDot />
          <span className="max-w-[22ch] truncate text-xs text-muted-foreground">{endpoint || "no server"}</span>
        </div>
      </header>
      <ReconnectBanner />
      <main className="flex-1 overflow-hidden">
        <Outlet />
      </main>
    </div>
  );
}

const btn = (on: boolean) =>
  `inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs transition ${
    on ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/60"
  }`;
