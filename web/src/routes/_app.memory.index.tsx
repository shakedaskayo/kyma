import { createFileRoute, redirect } from "@tanstack/react-router";

// /memory has no content of its own — redirect to the Overview landing.
export const Route = createFileRoute("/_app/memory/")({
  beforeLoad: () => {
    throw redirect({ to: "/memory/overview" });
  },
});
