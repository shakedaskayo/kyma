import type { LiveStatus } from "../../sdk/discover-live";

export function formatSummary(args: {
  sourcesSearched: number;
  windowLabel: string;
  eventCount: number;
  finishedAt: number | null;
  status: "idle" | "running" | "done" | "error" | "live";
  liveStatus?: LiveStatus | null;
}): string {
  const { sourcesSearched, windowLabel, eventCount, finishedAt, status } = args;
  const head = `Searched ${sourcesSearched} source${sourcesSearched === 1 ? "" : "s"} · ${windowLabel} · ${eventCount.toLocaleString()} events`;
  if (status === "running") return `${head} · searching…`;
  if (status === "live") return `${head} · live`;
  if (finishedAt != null) {
    const t = new Date(finishedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    return `${head} · as of ${t}`;
  }
  return head;
}

type StatusDotProps = { liveStatus: LiveStatus | null };

function StatusDot({ liveStatus }: StatusDotProps) {
  if (!liveStatus || liveStatus === "closed") return null;

  const dotClass =
    liveStatus === "live"
      ? "bg-green-500 animate-pulse"
      : liveStatus === "error"
        ? "bg-red-500"
        : "bg-amber-500 animate-pulse"; // connecting | backfilling

  return (
    <span
      className={`inline-block w-2 h-2 rounded-full mr-1 ${dotClass}`}
      aria-label={liveStatus}
    />
  );
}

export function SummaryLine(props: Parameters<typeof formatSummary>[0]) {
  return (
    <div className="px-3 py-1 text-xs text-muted-foreground border-b bg-surface/50 flex items-center gap-1">
      <StatusDot liveStatus={props.liveStatus ?? null} />
      {formatSummary(props)}
    </div>
  );
}
