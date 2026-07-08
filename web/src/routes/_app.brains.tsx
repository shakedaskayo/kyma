import { createFileRoute, Outlet } from "@tanstack/react-router";
import { BookOpenText } from "lucide-react";

// Layout route for /brains — memory published as Git-clonable Obsidian
// vaults. List in `_app.brains.index.tsx`, detail in `_app.brains.$name.tsx`.
export const Route = createFileRoute("/_app/brains")({
  component: BrainsLayout,
});

function BrainsLayout() {
  return (
    <div className="flex h-full flex-col bg-muted/20">
      <header className="sticky top-0 z-20 border-b border-border/60 bg-background/85 backdrop-blur-md">
        <div className="relative mx-auto w-full max-w-6xl px-6">
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2 pb-3 pt-3">
            <div className="flex items-center gap-2.5">
              <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary ring-1 ring-inset ring-primary/20">
                <BookOpenText className="h-4 w-4" />
              </span>
              <div className="leading-tight">
                <h1 className="text-base font-semibold tracking-tight">Brains</h1>
                <p className="text-2xs text-muted-foreground">
                  Memory published as Git-clonable Obsidian vaults — clone, grep, push edits
                  back.
                </p>
              </div>
            </div>
          </div>
        </div>
      </header>
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-6xl px-6 py-5">
          <Outlet />
        </div>
      </div>
    </div>
  );
}
