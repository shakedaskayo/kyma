import { createFileRoute } from "@tanstack/react-router";
import { AgentConsole } from "@/features/agent/AgentConsole";

export const Route = createFileRoute("/_app/agent")({
  component: AgentPage,
});

function AgentPage() {
  return <AgentConsole />;
}
