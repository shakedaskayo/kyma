import { createRootRoute, Outlet } from "@tanstack/react-router";
import { Toaster } from "sonner";
import { useTheme } from "@/lib/theme";

function RootShell() {
  // Drive sonner's theme from the resolved app theme so toasts (esp. the
  // dense red error toasts) pick up the dark palette instead of staying on
  // sonner's default light cards.
  const resolved = useTheme((s) => s.resolved);
  return (
    <>
      <Outlet />
      <Toaster
        richColors
        position="top-right"
        theme={resolved}
        toastOptions={{ classNames: { toast: "shadow-elev-3 backdrop-blur-md" } }}
      />
    </>
  );
}

export const Route = createRootRoute({
  component: RootShell,
});
