import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { useSession } from "@/sdk/session";
import { User2, Plug, Cpu, Sparkles, Terminal, Palette } from "lucide-react";

import { AccountSettings } from "@/features/settings/AccountSettings";
import { ConnectionSettings } from "@/features/settings/ConnectionSettings";
import { EngineSettings } from "@/features/settings/EngineSettings";
import { SkillsSettings } from "@/features/settings/SkillsSettings";
import { ConnectAgentSettings } from "@/features/settings/ConnectAgentSettings";
import { AppearanceSettings } from "@/features/settings/AppearanceSettings";

export const Route = createFileRoute("/settings")({
  validateSearch: (s: Record<string, unknown>) => ({
    next: typeof s.next === "string" ? s.next : "/explore",
  }),
  component: Settings,
});

type SectionId = "account" | "connection" | "engine" | "skills" | "connect-agent" | "appearance";

interface Section {
  id: SectionId;
  label: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
  component: React.ComponentType;
  requiresLogin?: boolean;
}

const SECTIONS: Section[] = [
  {
    id: "account",
    label: "Account",
    description: "Who you're signed in as",
    icon: User2,
    component: AccountSettings,
    requiresLogin: true,
  },
  {
    id: "connection",
    label: "Connection",
    description: "Server URL, default database, API token",
    icon: Plug,
    component: ConnectionSettings,
  },
  {
    id: "engine",
    label: "Agent engine",
    description: "Which model the agent uses",
    icon: Cpu,
    component: EngineSettings,
  },
  {
    id: "skills",
    label: "Skills",
    description: "Skill upstreams the agent can use",
    icon: Sparkles,
    component: SkillsSettings,
  },
  {
    id: "connect-agent",
    label: "Connect agent",
    description: "Install the CLI so coding agents (Claude Code, …) can query this server",
    icon: Terminal,
    component: ConnectAgentSettings,
  },
  {
    id: "appearance",
    label: "Appearance",
    description: "Theme — light, dark, system",
    icon: Palette,
    component: AppearanceSettings,
  },
];

function readHash(): SectionId {
  if (typeof window === "undefined") return "account";
  const raw = window.location.hash.replace(/^#/, "");
  const match = SECTIONS.find((s) => s.id === raw);
  return match?.id ?? "account";
}

function Settings() {
  const session = useSession();
  const [active, setActive] = useState<SectionId>(readHash);

  // Keep the URL hash in sync with the selected section so /settings#engine
  // deep-links and back/forward navigation works as expected.
  useEffect(() => {
    if (window.location.hash.replace(/^#/, "") !== active) {
      window.history.replaceState(null, "", `#${active}`);
    }
  }, [active]);

  useEffect(() => {
    const onHashChange = () => setActive(readHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const sections = SECTIONS.filter((s) => !s.requiresLogin || session.user);
  const current = sections.find((s) => s.id === active) ?? sections[0];
  const Body = current.component;

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col gap-6 p-6 md:flex-row">
      {/* Sidebar nav */}
      <aside className="md:w-56 md:shrink-0">
        <div className="mb-4 hidden md:block">
          <h1 className="text-lg font-semibold tracking-tight">Settings</h1>
          <p className="text-xs text-muted-foreground">
            {session.user
              ? `${session.user.username} · ${session.user.role}`
              : "Not signed in"}
          </p>
        </div>

        {/* Horizontal scroll-strip on small screens */}
        <nav className="-mx-1 flex flex-row gap-1 overflow-x-auto pb-2 md:flex-col md:overflow-x-visible">
          {sections.map((s) => {
            const Icon = s.icon;
            const isActive = s.id === active;
            return (
              <button
                key={s.id}
                type="button"
                onClick={() => setActive(s.id)}
                className={cn(
                  "flex shrink-0 items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors",
                  "md:w-full",
                  isActive
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/60 hover:text-foreground",
                )}
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span className="flex-1 truncate">{s.label}</span>
              </button>
            );
          })}
        </nav>
      </aside>

      {/* Content pane */}
      <div className="min-w-0 flex-1">
        <div className="mb-4">
          <h2 className="text-base font-semibold tracking-tight">{current.label}</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">{current.description}</p>
        </div>
        <div className="space-y-4">
          <Body />
        </div>
      </div>
    </div>
  );
}
