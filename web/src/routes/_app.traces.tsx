import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { X } from "lucide-react";
import { TracesList } from "@/features/traces/TracesList";
import { TraceWaterfall } from "@/features/traces/TraceWaterfall";

type TracesSearch = { trace?: string };

export const Route = createFileRoute("/_app/traces")({
  validateSearch: (search: Record<string, unknown>): TracesSearch => ({
    trace: typeof search.trace === "string" ? search.trace : undefined,
  }),
  component: TracesPage,
});

function TracesPage() {
  const { trace } = Route.useSearch();
  const navigate = useNavigate({ from: "/traces" });
  const select = (traceId: string | null) =>
    navigate({ search: { trace: traceId ?? undefined }, replace: true });

  return (
    <div className="flex h-full">
      <div className="min-w-0 flex-1 p-4">
        <h1 className="mb-3 text-sm font-medium text-foreground/90">Traces</h1>
        <TracesList selected={trace ?? null} onSelect={select} />
      </div>
      {trace && (
        <aside className="flex w-[34rem] shrink-0 flex-col border-l border-border/60 bg-surface">
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
            <span className="font-mono text-xs text-muted-foreground" title={trace}>
              trace {trace.slice(0, 16)}…
            </span>
            <button
              onClick={() => select(null)}
              className="ml-auto text-muted-foreground hover:text-foreground"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <TraceWaterfall traceId={trace} />
          </div>
        </aside>
      )}
    </div>
  );
}
