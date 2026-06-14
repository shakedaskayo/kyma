import { createFileRoute, redirect } from "@tanstack/react-router";

// The Memories tab moved to the Memory section.
export const Route = createFileRoute("/_app/data-sources/memories")({
  beforeLoad: () => {
    throw redirect({ to: "/memory/search" });
  },
});
