import { createFileRoute, redirect } from "@tanstack/react-router";

// Discover merged into the unified `/explore` page (keyword search is the
// default mode there). Redirect for back-compat + existing links.
export const Route = createFileRoute("/_app/discover")({
  beforeLoad: () => {
    throw redirect({ to: "/explore", search: { q: undefined } });
  },
  component: () => null,
});
