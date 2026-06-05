import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { Settings as SettingsIcon, LogOut, ChevronDown, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import { useHealth } from "@/sdk/reconnect";
import { useWorkspace } from "@/features/tabs/workspace-store";
import { ReconnectBanner } from "@/features/reconnect/ReconnectBanner";
import { HeaderDatabaseScope } from "./DatabaseSwitcher";
import { ThemeToggle } from "./ThemeToggle";
import { Sidebar } from "./Sidebar";
import { CommandPalette } from "@/features/palette/CommandPalette";
import { logout } from "@/sdk/auth";
import { useUI } from "@/lib/ui-store";
import { StatusDot, StatusPill } from "@/components/ui/status";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu";

// ── query status pill ─────────────────────────────────────────────────────────

function QueryStatusPill() {
  // Mirrors the active Query Editor tab's last result; scoped to /query|/explore
  // so a stale result doesn't leak into other routes' header.
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const activeTab = useWorkspace((s) => s.tabs.find((t) => t.id === s.activeId));
  if (!pathname.startsWith("/query") && !pathname.startsWith("/explore")) return null;
  if (!activeTab || activeTab.kind !== "query") return null;

  const r = activeTab.state.results;
  if (r.kind === "idle") return null;
  if (r.kind === "running") return <StatusPill tone="running" pulse>streaming</StatusPill>;
  if (r.kind === "ok")
    return (
      <StatusPill tone="ok">
        {r.rowCount.toLocaleString()} rows · {r.durationMs.toFixed(0)} ms
      </StatusPill>
    );
  if (r.kind === "error") return <StatusPill tone="error">error</StatusPill>;
  return null;
}

// ── command palette trigger ────────────────────────────────────────────────────

function SearchButton() {
  const toggle = useUI((s) => s.toggleCmd);
  return (
    <button
      type="button"
      onClick={toggle}
      className="group flex items-center gap-2 rounded-md border border-border-strong bg-background px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground"
      aria-label="Open command palette"
    >
      <Search className="h-3.5 w-3.5" />
      <span className="hidden md:inline">Search…</span>
      <kbd className="ml-2 hidden rounded bg-muted px-1.5 py-0.5 font-mono text-[10px] md:inline">⌘K</kbd>
    </button>
  );
}

// ── user badge + logout ───────────────────────────────────────────────────────

function UserMenu() {
  const session = useSession();
  const navigate = useNavigate();
  const [loggingOut, setLoggingOut] = useState(false);

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
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex items-center gap-1.5 rounded-md px-1.5 py-1 text-xs transition-colors hover:bg-accent"
        >
          <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary/15 text-[11px] font-semibold text-primary">
            {initial}
          </span>
          <span className="max-w-[14ch] truncate font-medium text-foreground">{label}</span>
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <div className="flex items-center gap-2 px-2 py-2">
          <span className="flex h-8 w-8 items-center justify-center rounded-full bg-primary/15 text-xs font-semibold text-primary">
            {initial}
          </span>
          <div className="min-w-0">
            <div className="truncate font-medium text-foreground">{label}</div>
            {role && <div className="text-xs capitalize text-muted-foreground">{role}</div>}
          </div>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() => navigate({ to: "/settings", search: { next: "/discover" } })}
        >
          <SettingsIcon /> Settings
        </DropdownMenuItem>
        <DropdownMenuItem destructive disabled={loggingOut} onSelect={(e) => { e.preventDefault(); void handleLogout(); }}>
          <LogOut /> {loggingOut ? "Logging out…" : "Log out"}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// ── shell ─────────────────────────────────────────────────────────────────────

export function Shell() {
  const { endpoint } = useSession();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const startHealth = useHealth((s) => s.start);
  useEffect(() => {
    if (!endpoint) return;
    return startHealth(endpoint);
  }, [endpoint, startHealth]);

  // The UI ships embedded in the server binary; when /health starts reporting
  // a new version mid-session (kyma update / reinstall restarted the server),
  // this tab's loaded assets are stale — nudge for a reload.
  const serverUpdated = useHealth((s) => s.updated);
  useEffect(() => {
    if (!serverUpdated) return;
    toast.info("kyma was updated", {
      id: "kyma-server-updated",
      description: "The server is running a new version — reload to get the latest UI.",
      duration: Infinity,
      action: { label: "Reload", onClick: () => window.location.reload() },
    });
  }, [serverUpdated]);

  return (
    <TooltipProvider delayDuration={250}>
      <div className="flex h-screen overflow-hidden bg-background text-foreground">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-12 items-center gap-3 border-b px-4 text-sm">
            <HeaderDatabaseScope />
            <QueryStatusPill />
            <div className="ml-auto flex items-center gap-3">
              <SearchButton />
              <div className="flex items-center gap-1.5">
                <ConnectionStatus />
                <span className="max-w-[22ch] truncate text-xs text-muted-foreground">
                  {endpoint || "no server"}
                </span>
              </div>
              <ThemeToggle />
              <UserMenu />
            </div>
          </header>
          <ReconnectBanner />
          <main className="flex-1 overflow-hidden">
            <motion.div
              key={pathname}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              transition={{ duration: 0.15, ease: "easeOut" }}
              className="h-full"
            >
              <Outlet />
            </motion.div>
          </main>
        </div>
        <CommandPalette />
      </div>
    </TooltipProvider>
  );
}

function ConnectionStatus() {
  const online = useHealth((s) => s.online);
  return (
    <StatusDot
      tone={online ? "ok" : "error"}
      title={online ? "connected" : "disconnected"}
    />
  );
}
