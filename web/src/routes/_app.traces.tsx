import { useState } from "react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { Check, Copy, X } from "lucide-react";
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
      <div className="flex min-w-0 flex-1 flex-col p-4">
        <div className="mb-3">
          <h1 className="text-sm font-medium text-foreground/90">Traces</h1>
          <p className="text-xs text-muted-foreground">
            Pensieve's own operations, end to end — plus anything shipped to the OTLP endpoint.
          </p>
        </div>
        <div className="min-h-0 flex-1">
          <TracesList selected={trace ?? null} onSelect={select} />
        </div>
      </div>
      {trace && (
        <aside className="flex w-[36rem] shrink-0 flex-col border-l border-border/60 bg-surface">
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
            <TraceId id={trace} />
            <button
              onClick={() => select(null)}
              className="ml-auto rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
              aria-label="Close trace panel"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="min-h-0 flex-1">
            <TraceWaterfall traceId={trace} />
          </div>
        </aside>
      )}
    </div>
  );
}

function TraceId({ id }: { id: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => {
        navigator.clipboard
          .writeText(id)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          })
          .catch(() => {});
      }}
      className="group flex items-center gap-1.5 font-mono text-xs text-muted-foreground transition-colors hover:text-foreground"
      title={`copy ${id}`}
    >
      trace {id.slice(0, 16)}…
      {copied ? (
        <Check className="h-3 w-3 text-emerald-400" />
      ) : (
        <Copy className="h-3 w-3 opacity-0 transition-opacity group-hover:opacity-100" />
      )}
    </button>
  );
}
