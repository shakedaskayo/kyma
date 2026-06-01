import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { Brain, Sparkles } from "lucide-react";
import { AgentConsole } from "@/features/agent/AgentConsole";
import { MemoryPanel } from "@/features/agent/MemoryPanel";

export const Route = createFileRoute("/_app/agent")({
  component: AgentPage,
});

type Tab = "console" | "memory";

function AgentPage() {
  const [tab, setTab] = useState<Tab>("console");
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 border-b px-2 py-1">
        <TabButton
          active={tab === "console"}
          onClick={() => setTab("console")}
          icon={<Sparkles className="h-3.5 w-3.5" />}
        >
          Console
        </TabButton>
        <TabButton
          active={tab === "memory"}
          onClick={() => setTab("memory")}
          icon={<Brain className="h-3.5 w-3.5" />}
        >
          Memory
        </TabButton>
      </div>
      <div className="min-h-0 flex-1">
        {tab === "console" ? <AgentConsole /> : <MemoryPanel />}
      </div>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex items-center gap-1.5 rounded-md px-3 py-1 text-sm transition-colors ${
        active
          ? "bg-accent font-medium text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}
