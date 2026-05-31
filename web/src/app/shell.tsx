import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { Settings as SettingsIcon, LayoutDashboard, Sparkles, Network, Plug, KeyRound, LogOut, ChevronDown, Search, Code } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useSession } from "@/sdk/session";
import { useHealth } from "@/sdk/reconnect";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { ReconnectBanner } from "@/features/reconnect/ReconnectBanner";
import { DatabaseSwitcher } from "./DatabaseSwitcher";
import { ThemeToggle } from "./ThemeToggle";
import { logout } from "@/sdk/auth";

// ── query status pill ─────────────────────────────────────────────────────────

function QueryStatusPill() {
  // The pill mirrors the *active Explore tab's* last query result. Showing
  // it from any other route was misleading — a stale "error" from a search
  // run hours ago sat in the header on Connectors/Graph/etc. as if the
  // current page had failed. Scope to /explore.
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const activeTab = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId));
  // Scope to the Query Editor: the discriminated workspace store carries
  // multiple tab kinds now (query / discover), and the pill only mirrors
  // the active *query* tab's last result. Also gate by route so the pill
  // doesn't leak into Connectors / Graph / etc.
  if (!pathname.startsWith("/query") && !pathname.startsWith("/explore")) return null;
  if (!activeTab || activeTab.kind !== "query") return null;

  const r = activeTab.state.results;

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

// ── user badge + logout ───────────────────────────────────────────────────────

function UserMenu() {
  const session = useSession();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  if (!session.token) return null;

  const label = session.user?.username ?? "Signed in";
  const role = session.user?.role;
  const initial = (session.user?.username ?? "?").charAt(0).toUpperCase();

  const handleLogout = async () => {
    setLoggingOut(true);
    if (session.token) {
      await logout({ endpoint: session.endpoint, token: session.token }).catch(() => {});
    }
    session.reset();
    navigate({ to: "/login", search: { next: undefined } });
  };

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs hover:bg-accent transition-colors"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary/10 text-[11px] font-semibold text-primary">
          {initial}
        </span>
        <span className="max-w-[14ch] truncate font-medium text-foreground">{label}</span>
        <ChevronDown className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <div
          role="menu"
          className="absolute right-0 z-50 mt-1 w-56 overflow-hidden rounded-md border bg-popover p-1 text-sm shadow-md"
        >
          <div className="flex items-center gap-2 px-2 py-2">
            <span className="flex h-8 w-8 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
              {initial}
            </span>
            <div className="min-w-0">
              <div className="truncate font-medium text-foreground">{label}</div>
              {role && <div className="text-xs capitalize text-muted-foreground">{role}</div>}
            </div>
          </div>
          <div className="my-1 h-px bg-border" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setOpen(false);
              navigate({ to: "/settings", search: { next: "/discover" } });
            }}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <SettingsIcon className="h-4 w-4" /> Settings
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={handleLogout}
            disabled={loggingOut}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-destructive hover:bg-destructive/10 disabled:opacity-60"
          >
            <LogOut className="h-4 w-4" /> {loggingOut ? "Logging out…" : "Log out"}
          </button>
        </div>
      )}
    </div>
  );
}

// ── shell ─────────────────────────────────────────────────────────────────────

export function Shell() {
  const { endpoint } = useSession();
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
        <DatabaseSwitcher />
        <QueryStatusPill />
        <nav className="ml-auto flex items-center gap-1">
          <Link to="/discover" className={btn(active.startsWith("/discover"))}>
            <Search className="h-4 w-4" /> Discover
          </Link>
          <Link to="/query" search={{ q: undefined }} className={btn(active.startsWith("/query") || active.startsWith("/explore"))}>
            <Code className="h-4 w-4" /> Query Editor
          </Link>
          <Link to="/dashboards" className={btn(active.startsWith("/dashboards"))}>
            <LayoutDashboard className="h-4 w-4" /> Dashboards
          </Link>
          <Link to="/agent" className={btn(active.startsWith("/agent"))}>
            <Sparkles className="h-4 w-4" /> Agent
          </Link>
          <Link to="/graph" className={btn(active.startsWith("/graph"))}>
            <Network className="h-4 w-4" /> Graph
          </Link>
          <Link to="/connectors" className={btn(active.startsWith("/connectors"))}>
            <Plug className="h-4 w-4" /> Connectors
          </Link>
          <Link to="/credentials" className={btn(active.startsWith("/credentials"))}>
            <KeyRound className="h-4 w-4" /> Credentials
          </Link>
          <Link to="/settings" search={{ next: active }} className={btn(active === "/settings")}>
            <SettingsIcon className="h-4 w-4" /> Settings
          </Link>
        </nav>
        <div className="ml-2 flex items-center gap-1.5">
          <ConnectionDot />
          <span className="max-w-[22ch] truncate text-xs text-muted-foreground">{endpoint || "no server"}</span>
        </div>
        <ThemeToggle />
        <UserMenu />
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
