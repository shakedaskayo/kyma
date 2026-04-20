import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { Database, Settings as SettingsIcon, Compass } from "lucide-react";
import { useSession } from "@/sdk/session";

export function Shell() {
  const { endpoint, database } = useSession();
  const active = useRouterState().location.pathname;
  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <header className="flex h-12 items-center gap-4 border-b px-4 text-sm">
        <div className="font-semibold tracking-tight">kyma</div>
        <div className="flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs">
          <Database className="h-3.5 w-3.5" /> {database || "—"}
        </div>
        <nav className="ml-auto flex items-center gap-1">
          <Link to="/explore" className={btn(active.startsWith("/explore"))}>
            <Compass className="h-4 w-4" /> Explore
          </Link>
          <Link to="/settings" search={{ next: active }} className={btn(active === "/settings")}>
            <SettingsIcon className="h-4 w-4" /> Settings
          </Link>
        </nav>
        <div className="ml-2 max-w-[22ch] truncate text-xs text-muted-foreground">{endpoint || "no server"}</div>
      </header>
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
