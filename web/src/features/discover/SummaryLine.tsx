export function formatSummary(args: {
  sourcesSearched: number;
  windowLabel: string;
  eventCount: number;
  finishedAt: number | null;
  status: "idle" | "running" | "done" | "error";
}): string {
  const { sourcesSearched, windowLabel, eventCount, finishedAt, status } = args;
  const head = `Searched ${sourcesSearched} source${sourcesSearched === 1 ? "" : "s"} · ${windowLabel} · ${eventCount.toLocaleString()} events`;
  if (status === "running") return `${head} · searching…`;
  if (finishedAt != null) {
    const t = new Date(finishedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    return `${head} · as of ${t}`;
  }
  return head;
}

export function SummaryLine(props: Parameters<typeof formatSummary>[0]) {
  return (
    <div className="px-3 py-1 text-xs text-muted-foreground border-b bg-surface/50">
      {formatSummary(props)}
    </div>
  );
}
