import { createFileRoute, redirect } from "@tanstack/react-router";
import { Shell } from "@/app/shell";
import { useSession } from "@/sdk/session";

export const Route = createFileRoute("/_app")({
  beforeLoad: ({ location }) => {
    const s = useSession.getState();
    if (!s.isConfigured() && location.pathname !== "/login") {
      throw redirect({ to: "/login", search: { next: location.pathname } });
    }
  },
  component: Shell,
});
