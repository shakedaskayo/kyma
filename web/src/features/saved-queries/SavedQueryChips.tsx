type Props = { onPick: (query: string) => void };

const QUERIES = [
  {
    label: "Errors by service",
    q: `otel_logs | where severity_text == "ERROR" | where timestamp > ago(1h) | summarize n=count() by service_name | order by n desc`,
  },
  {
    label: "Slow traces (>1s)",
    q: `otel_traces | where duration_ms > 1000 | where timestamp > ago(1h) | take 50`,
  },
  {
    label: "p99 latency",
    q: `otel_metrics | where metric_name == "p99_latency_ms" | where timestamp > ago(2h) | take 200`,
  },
  {
    label: "Recent deploys",
    q: `deploys | where timestamp > ago(24h) | project timestamp, service_name, version, author`,
  },
  {
    label: "LLM cost by model",
    q: `llm_calls | summarize total=sum(cost_usd) by model | order by total desc`,
  },
];

export function SavedQueryChips({ onPick }: Props) {
  return (
    <div className="flex flex-wrap gap-1.5 px-4 py-2 border-b border-muted/40 bg-muted/10">
      <span className="self-center text-[10px] font-medium text-muted-foreground mr-1 shrink-0">Quick:</span>
      {QUERIES.map((sq) => (
        <button
          key={sq.label}
          onClick={() => onPick(sq.q)}
          className="inline-flex items-center rounded-full border border-muted/60 bg-background px-2.5 py-0.5 text-[11px] font-medium text-muted-foreground shadow-sm transition-colors hover:border-primary/60 hover:bg-primary/5 hover:text-primary focus:outline-none focus:ring-2 focus:ring-primary/30"
        >
          {sq.label}
        </button>
      ))}
    </div>
  );
}
