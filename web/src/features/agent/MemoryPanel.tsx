import { useCallback, useEffect, useState } from "react";
import { Activity, Brain, Clock, Database, RefreshCw, Workflow } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  fetchMemoryOverview,
  type MemoryOverview,
  type PipelineRun,
  type Row,
} from "@/sdk/memory";

/**
 * Agent-tab Memory panel: realtime visibility into the memory ingestion
 * pipeline — the conversation firehose (`claude_code_events`), the memory store,
 * and the consolidation ("dreaming") pipeline runs. Polls every 5s.
 */

const num = (v: unknown): number => (typeof v === "number" ? v : Number(v) || 0);
const str = (v: unknown): string => (v == null ? "" : String(v));

function shortTime(iso: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// Aligned with the shared data palette (cyan-led "synapse" set) so memory
// chips read as part of the same system as charts / graph / badges.
const KIND_COLOR: Record<string, string> = {
  user_prompt: "bg-sky-500",
  assistant_turn: "bg-violet-500",
  tool_use: "bg-amber-500",
  session_start: "bg-emerald-500",
  session_end: "bg-rose-500",
};

export function MemoryPanel() {
  const { endpoint, token } = useSession();
  const [data, setData] = useState<MemoryOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    if (!endpoint || !token) return;
    setLoading(true);
    try {
      const d = await fetchMemoryOverview({ endpoint, token });
      setData(d);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [endpoint, token]);

  useEffect(() => {
    void load();
    const t = setInterval(() => void load(), 5000);
    return () => clearInterval(t);
  }, [load]);

  const fh = data?.firehose;
  const totalEvents = (fh?.by_kind ?? []).reduce((s, r) => s + num(r.n), 0);
  const totalMemories = (data?.memory.counts ?? []).reduce((s, r) => s + num(r.n), 0);
  const lastRun = data?.pipeline_runs?.[0] ?? null;

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2 text-sm">
        <Brain className="h-4 w-4 text-primary" />
        <h1 className="font-semibold tracking-tight">Memory ingestion</h1>
        <span className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
          <span className="flex items-center gap-1">
            <Activity className="h-3.5 w-3.5" /> {totalEvents} events
          </span>
          <span className="flex items-center gap-1">
            <Database className="h-3.5 w-3.5" /> {totalMemories} memories
          </span>
          <button
            type="button"
            onClick={() => void load()}
            className="flex items-center gap-1 rounded border px-1.5 py-0.5 hover:bg-accent"
          >
            <RefreshCw className={`h-3 w-3 ${loading ? "animate-spin" : ""}`} /> Refresh
          </button>
        </span>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-5xl space-y-4 px-4 py-4">
          {error && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {error}
            </div>
          )}

          {/* Pipeline status */}
          <Section icon={<Workflow className="h-4 w-4" />} title="Consolidation pipeline">
            <PipelineStatus run={lastRun} runs={data?.pipeline_runs ?? []} />
          </Section>

          {/* Firehose ingestion */}
          <div className="grid gap-4 md:grid-cols-2">
            <Section icon={<Activity className="h-4 w-4" />} title="Firehose by kind">
              <KindBars rows={fh?.by_kind ?? []} />
              <Timeline rows={fh?.timeline ?? []} />
            </Section>
            <Section icon={<Clock className="h-4 w-4" />} title="Recent sessions">
              <SessionTable rows={fh?.sessions ?? []} />
            </Section>
          </div>

          {/* Recent events feed */}
          <Section icon={<Activity className="h-4 w-4" />} title="Live event feed">
            <EventFeed rows={fh?.recent ?? []} />
          </Section>

          {/* Memory store */}
          <Section icon={<Brain className="h-4 w-4" />} title="Memory store">
            <MemoryCounts rows={data?.memory.counts ?? []} />
            <MemoryList rows={data?.memory.recent ?? []} />
          </Section>
        </div>
      </div>
    </div>
  );
}

function Section({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border bg-card">
      <div className="flex items-center gap-2 border-b px-3 py-2 text-xs font-semibold text-muted-foreground">
        {icon} {title}
      </div>
      <div className="space-y-3 p-3">{children}</div>
    </section>
  );
}

function StatusDot({ status }: { status: string }) {
  const color =
    status === "success"
      ? "bg-emerald-500"
      : status === "error"
        ? "bg-rose-500"
        : status === "running"
          ? "bg-amber-500 animate-pulse"
          : "bg-muted-foreground";
  return <span className={`inline-block h-2 w-2 rounded-full ${color}`} />;
}

function PipelineStatus({ run, runs }: { run: PipelineRun | null; runs: PipelineRun[] }) {
  if (!run) {
    return (
      <p className="text-xs text-muted-foreground">
        No consolidation runs yet. The pipeline distills new conversation activity into durable
        summary memories on a schedule.
      </p>
    );
  }
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 text-sm">
        <StatusDot status={run.status} />
        <span className="font-medium capitalize">{run.status}</span>
        <span className="text-xs text-muted-foreground">
          {shortTime(run.started_at)} · {run.events_scanned} events scanned ·{" "}
          {run.memories_written} memories written
        </span>
      </div>
      {run.error && <div className="text-xs text-rose-500">{run.error}</div>}
      <div className="divide-y rounded border text-xs">
        {runs.slice(0, 6).map((r) => (
          <div key={r.id} className="flex items-center gap-2 px-2 py-1">
            <StatusDot status={r.status} />
            <span className="text-muted-foreground">{shortTime(r.started_at)}</span>
            <span className="ml-auto tabular-nums text-muted-foreground">
              {r.events_scanned} ev → {r.memories_written} mem
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function KindBars({ rows }: { rows: Row[] }) {
  const max = Math.max(1, ...rows.map((r) => num(r.n)));
  if (rows.length === 0)
    return <p className="text-xs text-muted-foreground">No firehose events yet.</p>;
  return (
    <div className="space-y-1.5">
      {rows.map((r) => {
        const kind = str(r.kind);
        const n = num(r.n);
        return (
          <div key={kind} className="flex items-center gap-2 text-xs">
            <span className="w-28 shrink-0 text-muted-foreground">{kind}</span>
            <div className="h-3 flex-1 rounded bg-muted">
              <div
                className={`h-3 rounded ${KIND_COLOR[kind] ?? "bg-primary"}`}
                style={{ width: `${(n / max) * 100}%` }}
              />
            </div>
            <span className="w-10 shrink-0 text-right tabular-nums">{n}</span>
          </div>
        );
      })}
    </div>
  );
}

function Timeline({ rows }: { rows: Row[] }) {
  if (rows.length === 0) return null;
  const max = Math.max(1, ...rows.map((r) => num(r.n)));
  return (
    <div className="mt-2">
      <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
        Events / hour
      </div>
      <div className="flex h-12 items-end gap-0.5">
        {rows.slice(-48).map((r, i) => (
          <div
            key={i}
            title={`${str(r.bucket)}: ${num(r.n)}`}
            className="flex-1 rounded-sm bg-primary/70"
            style={{ height: `${Math.max(4, (num(r.n) / max) * 100)}%` }}
          />
        ))}
      </div>
    </div>
  );
}

function SessionTable({ rows }: { rows: Row[] }) {
  if (rows.length === 0)
    return <p className="text-xs text-muted-foreground">No sessions captured yet.</p>;
  return (
    <div className="divide-y text-xs">
      {rows.map((r, i) => (
        <div key={i} className="flex items-center gap-2 py-1">
          <span className="truncate font-mono text-[11px] text-muted-foreground" title={str(r.session_id)}>
            {str(r.session_id).slice(0, 8)}
          </span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-[10px]">{str(r.realm)}</span>
          <span className="ml-auto tabular-nums text-muted-foreground">{num(r.events)} ev</span>
          <span className="w-28 shrink-0 text-right text-muted-foreground">
            {shortTime(str(r.last_seen))}
          </span>
        </div>
      ))}
    </div>
  );
}

function EventFeed({ rows }: { rows: Row[] }) {
  if (rows.length === 0)
    return <p className="text-xs text-muted-foreground">No events yet — start a Claude Code session with the kyma-memory plugin.</p>;
  return (
    <div className="max-h-72 space-y-1 overflow-auto">
      {rows.map((r, i) => {
        const kind = str(r.kind);
        const text = str(r.text) || (r.tool_name ? `tool: ${str(r.tool_name)}` : "");
        return (
          <div key={i} className="flex items-start gap-2 text-xs">
            <span className={`mt-1 h-2 w-2 shrink-0 rounded-full ${KIND_COLOR[kind] ?? "bg-primary"}`} />
            <span className="w-24 shrink-0 text-muted-foreground">{kind}</span>
            <span className="flex-1 truncate" title={text}>
              {text}
            </span>
            <span className="w-24 shrink-0 text-right text-muted-foreground">
              {shortTime(str(r.ts))}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function MemoryCounts({ rows }: { rows: Row[] }) {
  // Aggregate by memory_type across realms/statuses.
  const byType = new Map<string, number>();
  for (const r of rows) {
    const t = str(r.memory_type) || "memory";
    byType.set(t, (byType.get(t) ?? 0) + num(r.n));
  }
  const entries = [...byType.entries()];
  if (entries.length === 0)
    return <p className="text-xs text-muted-foreground">No memories stored yet.</p>;
  return (
    <div className="flex flex-wrap gap-2">
      {entries.map(([t, n]) => (
        <span key={t} className="rounded-full border bg-muted/40 px-2 py-0.5 text-xs">
          <span className="font-medium">{n}</span> <span className="text-muted-foreground">{t}</span>
        </span>
      ))}
    </div>
  );
}

function MemoryList({ rows }: { rows: Row[] }) {
  if (rows.length === 0) return null;
  return (
    <div className="divide-y text-xs">
      {rows.map((r, i) => (
        <div key={i} className="py-1.5">
          <div className="flex items-center gap-2">
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px]">{str(r.memory_type)}</span>
            <span className="rounded bg-muted/50 px-1.5 py-0.5 text-[10px] text-muted-foreground">
              {str(r.realm)}
            </span>
            <span className="ml-auto text-[10px] text-muted-foreground">
              {shortTime(str(r.created_at))}
            </span>
          </div>
          <p className="mt-1 line-clamp-2 text-muted-foreground">{str(r.content_preview)}</p>
        </div>
      ))}
    </div>
  );
}
