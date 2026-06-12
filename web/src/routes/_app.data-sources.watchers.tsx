import { createFileRoute, redirect } from "@tanstack/react-router";

// The File Watchers tab merged into the unified /data-sources page.
export const Route = createFileRoute("/_app/data-sources/watchers")({
  beforeLoad: () => {
    throw redirect({ to: "/data-sources" });
  },
});
