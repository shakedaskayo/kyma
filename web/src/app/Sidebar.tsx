import { Link, useRouterState } from "@tanstack/react-router";
import { motion } from "framer-motion";
import {
  Search,
  Network,
  LayoutDashboard,
  Plug,
  KeyRound,
  Sparkles,
  Brain,
  Cloud,
  Settings as SettingsIcon,
  PanelLeftClose,
  PanelLeftOpen,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useUI } from "@/lib/ui-store";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { useCapabilities, type Capabilities } from "@/sdk/capabilities";

interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  /** Search params required by the destination route, if any. */
  search?: Record<string, unknown>;
  /** Capability flag this surface needs; a cloud badge marks it when the
   * connected server (e.g. local mode) doesn't have it. */
  requires?: keyof Omit<Capabilities, "mode">;
}

interface NavGroup {
  label: string;
  items: NavItem[];
}

const GROUPS: NavGroup[] = [
  {
    label: "Explore",
    items: [
      { to: "/explore", label: "Explore", icon: Search, search: { q: undefined } },
      { to: "/graph", label: "Graph", icon: Network },
    ],
  },
  {
    label: "Build",
    items: [
      { to: "/dashboards", label: "Dashboards", icon: LayoutDashboard },
      { to: "/connectors", label: "Connectors", icon: Plug, requires: "connectors" },
      { to: "/credentials", label: "Credentials", icon: KeyRound, requires: "credentials" },
    ],
  },
  {
    label: "Intelligence",
    items: [
      { to: "/agent", label: "Agent", icon: Sparkles },
      { to: "/memory", label: "Memory", icon: Brain },
    ],
  },
];

function NavLinkRow({
  item,
  active,
  collapsed,
}: {
  item: NavItem;
  active: boolean;
  collapsed: boolean;
}) {
  const Icon = item.icon;
  const caps = useCapabilities();
  const cloudOnly = item.requires ? !caps[item.requires] : false;
  const row = (
    <Link
      to={item.to}
      search={item.search as never}
      className={cn(
        "group relative flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium transition-colors",
        collapsed && "justify-center px-0",
        active
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
      )}
      aria-current={active ? "page" : undefined}
    >
      {active && (
        <motion.span
          layoutId="nav-active-bar"
          className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full bg-primary"
          transition={{ type: "spring", stiffness: 500, damping: 38 }}
        />
      )}
      <Icon className={cn("h-[1.05rem] w-[1.05rem] shrink-0", active && "text-primary")} />
      {!collapsed && <span className="truncate">{item.label}</span>}
      {!collapsed && cloudOnly && (
        <SimpleTooltip label="Runs on the kyma control plane" side="right">
          <Cloud className="ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        </SimpleTooltip>
      )}
    </Link>
  );
  return collapsed ? (
    <SimpleTooltip label={cloudOnly ? `${item.label} — control plane` : item.label} side="right">
      {row}
    </SimpleTooltip>
  ) : (
    row
  );
}

export function Sidebar() {
  const collapsed = useUI((s) => s.sidebarCollapsed);
  const toggle = useUI((s) => s.toggleSidebar);
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const isActive = (to: string) =>
    to === "/explore"
      ? pathname.startsWith("/explore") ||
        pathname.startsWith("/query") ||
        pathname.startsWith("/discover")
      : pathname.startsWith(to);

  return (
    <aside
      className={cn(
        "flex h-full shrink-0 flex-col border-r bg-surface transition-[width] duration-200",
        collapsed ? "w-[3.75rem]" : "w-56",
      )}
    >
      {/* Brand */}
      <div className={cn("flex h-14 items-center gap-2 px-3", collapsed && "justify-center px-0")}>
        <img src="/icons/kyma-mark.svg" alt="kyma" className="h-7 w-7 shrink-0" />
        {!collapsed && <span className="text-base font-semibold tracking-tight">kyma</span>}
      </div>

      {/* Nav groups */}
      <nav className="flex-1 space-y-4 overflow-y-auto px-2 py-2">
        {GROUPS.map((group) => (
          <div key={group.label} className="space-y-0.5">
            {!collapsed && (
              <div className="px-2.5 pb-1 text-2xs font-semibold uppercase tracking-wider text-muted-foreground/70">
                {group.label}
              </div>
            )}
            {group.items.map((item) => (
              <NavLinkRow
                key={item.to}
                item={item}
                active={isActive(item.to)}
                collapsed={collapsed}
              />
            ))}
          </div>
        ))}
      </nav>

      {/* Footer — Settings + collapse toggle */}
      <div className="space-y-0.5 border-t px-2 py-2">
        <NavLinkRow
          item={{ to: "/settings", label: "Settings", icon: SettingsIcon, search: { next: pathname } }}
          active={pathname === "/settings"}
          collapsed={collapsed}
        />
        <button
          type="button"
          onClick={toggle}
          className={cn(
            "flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground",
            collapsed && "justify-center px-0",
          )}
          title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        >
          {collapsed ? (
            <PanelLeftOpen className="h-[1.05rem] w-[1.05rem]" />
          ) : (
            <>
              <PanelLeftClose className="h-[1.05rem] w-[1.05rem]" />
              <span>Collapse</span>
            </>
          )}
        </button>
      </div>
    </aside>
  );
}
