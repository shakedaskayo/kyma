import { createFileRoute, Outlet } from "@tanstack/react-router";
import { Database } from "lucide-react";

// Layout route for /data-sources. The unified sources list lives in
// `_app.data-sources.index.tsx`, the detail in `_app.data-sources.$id.tsx`;
// both render through this Outlet under the shared header. (The old
// watchers/sync/memories tabs are gone — their routes redirect.)
export const Route = createFileRoute("/_app/data-sources")({
  component: DataSourcesLayout,
});

function DataSourcesLayout() {
  return (
    <div className="flex h-full flex-col bg-muted/20">
      <header className="sticky top-0 z-20 border-b border-border/60 bg-background/85 backdrop-blur-md">
        <div className="relative mx-auto w-full max-w-6xl px-6">
          <div className="flex flex-wrap items-center gap-x-5 gap-y-2 pb-3 pt-3">
            <div className="flex items-center gap-2.5">
              <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary ring-1 ring-inset ring-primary/20">
                <Database className="h-4 w-4" />
              </span>
              <div className="leading-tight">
                <h1 className="text-base font-semibold tracking-tight">Data Sources</h1>
                <p className="text-2xs text-muted-foreground">
                  Everything feeding the context graph — agent memory, vaults, and connected
                  sources, in one place.
                </p>
              </div>
            </div>
          </div>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Outlet />
      </div>
    </div>
  );
}
