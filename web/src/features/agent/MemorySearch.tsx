import { useCallback, useState } from "react";
import { Brain, Clock, Link2, Search, Sparkles } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  queryMemory,
  type MemoryQueryResult,
  type RetrievedMemory,
} from "@/sdk/memory";

/**
 * Memory search workspace: drives the graph-aware hybrid recall API
 * (`POST /v1/agent/memory/query`). Returns ranked memories with validity
 * intervals, the connecting graph paths (`via`), connected catalog
 * resources/traces (`linked`), and an optional synthesized `brief`.
 */

const TYPES = [
  "",
  "fact",
  "decision",
  "preference",
  "learning",
  "procedure",
  "summary",
  "entity",
] as const;

const TYPE_COLOR: Record<string, string> = {
  fact: "bg-blue-500",
  decision: "bg-violet-500",
  preference: "bg-amber-500",
  learning: "bg-emerald-500",
  procedure: "bg-cyan-500",
  summary: "bg-slate-500",
  entity: "bg-rose-500",
};

function shortTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function MemorySearch() {
  const { endpoint, token } = useSession();
  const [q, setQ] = useState("");
  const [agentic, setAgentic] = useState(false);
  const [hops, setHops] = useState(1);
  const [type, setType] = useState("");
  const [res, setRes] = useState<MemoryQueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const run = useCallback(async () => {
    if (!endpoint || !token || !q.trim()) return;
    setLoading(true);
    try {
      const r = await queryMemory(
        { endpoint, token },
        {
          query: q.trim(),
          mode: agentic ? "agentic" : "fast",
          expand_hops: hops,
          memory_type: type || undefined,
          limit: 20,
        },
      );
      setRes(r);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [endpoint, token, q, agentic, hops, type]);

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2 text-sm">
        <Search className="h-4 w-4 text-primary" />
        <h1 className="font-semibold tracking-tight">Memory search</h1>
        {res && (
          <span className="ml-auto flex items-center gap-1 text-xs text-muted-foreground">
            <Clock className="h-3.5 w-3.5" /> {res.took_ms} ms · {res.memories.length} hits
          </span>
        )}
      </header>

      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-4xl space-y-4 px-4 py-4">
          {/* Search controls */}
          <div className="space-y-2 rounded-lg border p-3">
            <div className="flex gap-2">
              <input
                value={q}
                onChange={(e) => setQ(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void run();
                  }
                }}
                placeholder="Search memories — decisions, conventions, what an entity relates to…"
                className="flex-1 rounded-md border bg-background px-3 py-1.5 text-sm outline-none focus:ring-1 focus:ring-ring"
              />
              <button
                type="button"
                onClick={() => void run()}
                disabled={loading || !q.trim()}
                className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground disabled:opacity-50"
              >
                <Search className={`h-3.5 w-3.5 ${loading ? "animate-pulse" : ""}`} />
                Search
              </button>
            </div>
            <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
              <label className="flex items-center gap-1">
                Type
                <select
                  value={type}
                  onChange={(e) => setType(e.target.value)}
                  className="rounded border bg-background px-1.5 py-0.5"
                >
                  {TYPES.map((t) => (
                    <option key={t} value={t}>
                      {t === "" ? "any" : t}
                    </option>
                  ))}
                </select>
              </label>
              <label className="flex items-center gap-1">
                Graph hops
                <select
                  value={hops}
                  onChange={(e) => setHops(Number(e.target.value))}
                  className="rounded border bg-background px-1.5 py-0.5"
                >
                  <option value={0}>0</option>
                  <option value={1}>1</option>
                  <option value={2}>2</option>
                </select>
              </label>
              <label className="flex items-center gap-1.5">
                <input
                  type="checkbox"
                  checked={agentic}
                  onChange={(e) => setAgentic(e.target.checked)}
                />
                <Sparkles className="h-3.5 w-3.5" /> Synthesize answer
              </label>
            </div>
          </div>

          {error && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          )}

          {/* Synthesized brief (agentic mode) */}
          {res?.brief && (
            <div className="rounded-lg border bg-accent/30 p-3">
              <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <Sparkles className="h-3.5 w-3.5" /> Synthesized answer
              </div>
              <p className="whitespace-pre-wrap text-sm">{res.brief}</p>
            </div>
          )}

          {/* Ranked memories */}
          {res && res.memories.length > 0 && (
            <div className="space-y-2">
              {res.memories.map((m) => (
                <MemoryCard key={m.id} m={m} />
              ))}
            </div>
          )}

          {res && res.memories.length === 0 && !error && (
            <div className="py-8 text-center text-sm text-muted-foreground">
              No memories matched.
            </div>
          )}

          {/* Connected resources / traces */}
          {res && res.linked.length > 0 && (
            <div className="rounded-lg border p-3">
              <div className="mb-2 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <Link2 className="h-3.5 w-3.5" /> Connected resources &amp; traces
              </div>
              <ul className="space-y-1 text-xs">
                {res.linked.map((l, i) => (
                  <li key={`${l.node_id}-${i}`} className="flex items-center gap-2">
                    <code className="rounded bg-muted px-1 py-0.5">{l.node_id}</code>
                    <span className="text-muted-foreground">
                      {l.target_namespace ? `${l.target_namespace} · ` : ""}
                      {l.edge_type} · hop {l.depth}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {!res && !error && (
            <div className="flex flex-col items-center gap-2 py-12 text-center text-sm text-muted-foreground">
              <Brain className="h-8 w-8 opacity-40" />
              Search your agent&apos;s memory — hybrid semantic + keyword recall,
              graph-expanded over connected memories, resources, and traces.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function MemoryCard({ m }: { m: RetrievedMemory }) {
  const color = TYPE_COLOR[m.memory_type] ?? "bg-slate-500";
  const validity = m.invalid_at
    ? `invalidated ${shortTime(m.invalid_at)}`
    : m.valid_at
      ? `since ${shortTime(m.valid_at)}`
      : "";
  return (
    <div className="rounded-lg border p-3 text-sm">
      <div className="mb-1 flex items-center gap-2 text-xs">
        <span className={`inline-block h-2 w-2 rounded-full ${color}`} />
        <span className="font-medium">{m.memory_type}</span>
        <span className="text-muted-foreground">{m.realm}</span>
        <span className="ml-auto font-mono text-muted-foreground">
          score {m.score.toFixed(2)}
        </span>
      </div>
      {m.title && <div className="font-medium">{m.title}</div>}
      <p className="whitespace-pre-wrap text-muted-foreground">{m.content_preview}</p>
      <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
        {validity && <span>{validity}</span>}
        {m.via?.type && (
          <span className="rounded bg-muted px-1 py-0.5">
            via {m.via.type}
            {typeof m.via.depth === "number" ? ` (hop ${m.via.depth})` : ""}
          </span>
        )}
        {m.distance != null && <span>dist {m.distance.toFixed(3)}</span>}
        {m.graph_proximity > 0 && <span>graph {m.graph_proximity.toFixed(2)}</span>}
        <span className="ml-auto font-mono opacity-60">{m.id}</span>
      </div>
    </div>
  );
}
