import { createFileRoute, redirect } from "@tanstack/react-router";
import { Shell } from "@/app/shell";
import { useSession } from "@/sdk/session";

export const Route = createFileRoute("/_app")({
  beforeLoad: ({ location }) => {
    const s = useSession.getState();
    if (!s.isConfigured() && location.pathname !== "/settings") {
      throw redirect({ to: "/settings", search: { next: location.pathname } });
    }
  },
  component: Shell,
});
