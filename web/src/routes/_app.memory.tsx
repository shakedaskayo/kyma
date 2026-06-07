import { createFileRoute, Link, Outlet, useRouterState } from "@tanstack/react-router";
import { Moon, Search, SlidersHorizontal } from "lucide-react";

export const Route = createFileRoute("/_app/memory")({
  component: MemoryLayout,
});

function MemoryLayout() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const isSearch = pathname.startsWith("/memory/search");
  const isDreaming = pathname.startsWith("/memory/dreaming");
  const isSettings = pathname.startsWith("/memory/settings");

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 border-b px-2 py-1">
        <TabLink to="/memory/search" active={isSearch} icon={<Search className="h-3.5 w-3.5" />}>
          Search
        </TabLink>
        <TabLink to="/memory/dreaming" active={isDreaming} icon={<Moon className="h-3.5 w-3.5" />}>
          Dreaming
        </TabLink>
        <TabLink
          to="/memory/settings"
          active={isSettings}
          icon={<SlidersHorizontal className="h-3.5 w-3.5" />}
        >
          Settings
        </TabLink>
      </div>
      <div className="min-h-0 flex-1">
        <Outlet />
      </div>
    </div>
  );
}

function TabLink({
  to,
  active,
  icon,
  children,
}: {
  to: string;
  active: boolean;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <Link
      to={to}
      className={`flex items-center gap-1.5 rounded-md px-3 py-1 text-sm transition-colors ${
        active
          ? "bg-accent font-medium text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50"
      }`}
    >
      {icon}
      {children}
    </Link>
  );
}
