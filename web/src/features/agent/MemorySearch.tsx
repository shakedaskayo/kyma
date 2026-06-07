import { useCallback, useMemo, useState } from "react";
import {
  Clock,
  CornerDownLeft,
  Link2,
  Search,
  Sparkles,
  Workflow,
} from "lucide-react";
import { motion } from "framer-motion";
import { useSession } from "@/sdk/session";
import {
  queryMemory,
  type MemoryQueryResult,
  type RetrievedMemory,
} from "@/sdk/memory";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { MEMORY_TYPES, typeStyle } from "@/features/memory/lib";

/**
 * Memory search workspace — drives the graph-aware hybrid recall API
 * (`POST /v1/agent/memory/query`): hybrid semantic + keyword candidates, RRF
 * fusion, 1–2 hop graph expansion, blended scoring. Shows ranked memories with
 * validity intervals, the connecting graph path, connected resources/traces,
 * and an optional synthesized answer. Lives under the shared Memory header.
 */

const TYPES = MEMORY_TYPES;

const EXAMPLES = [
  "what did we decide about embeddings?",
  "how do we run a release?",
  "conventions for migrations",
];

function shortTime(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

export function MemorySearch() {
  const { endpoint, token } = useSession();
  const reduce = useReducedMotion();
  const [q, setQ] = useState("");
  const [agentic, setAgentic] = useState(false);
  const [hops, setHops] = useState(1);
  const [type, setType] = useState<string>("");
  const [res, setRes] = useState<MemoryQueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const run = useCallback(
    async (query?: string) => {
      const text = (query ?? q).trim();
      if (!endpoint || !token || !text) return;
      setQ(text);
      setLoading(true);
      try {
        const r = await queryMemory(
          { endpoint, token },
          {
            query: text,
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
    },
    [endpoint, token, q, agentic, hops, type],
  );

  const maxScore = useMemo(
    () => Math.max(0.0001, ...(res?.memories ?? []).map((m) => m.score)),
    [res],
  );

  return (
    <div className="flex h-full flex-col">
      {/* Search bar + filters */}
      <div className="border-b border-border/60 bg-surface/40">
        <div className="mx-auto w-full max-w-3xl px-6 py-4">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void run();
                }
              }}
              aria-label="Search memories"
              placeholder="Search memories — decisions, conventions, what an entity relates to…"
              className="h-12 w-full rounded-xl border border-border/70 bg-card/60 pl-10 pr-28 text-sm shadow-elev-1 outline-none transition-shadow focus-visible:border-violet-400/40 focus-visible:ring-2 focus-visible:ring-violet-400/30"
            />
            <Button
              size="sm"
              onClick={() => void run()}
              disabled={loading || !q.trim()}
              className="absolute right-2 top-1/2 -translate-y-1/2"
            >
              {loading ? "Searching…" : "Search"}
              {!loading && <CornerDownLeft className="h-3.5 w-3.5 opacity-70" />}
            </Button>
          </div>

          {/* Filters */}
          <div className="mt-3 flex flex-wrap items-center gap-1.5 text-xs">
            <Pill active={!type} onClick={() => setType("")}>
              All types
            </Pill>
            {TYPES.map((t) => (
              <Pill key={t} active={type === t} onClick={() => setType(type === t ? "" : t)}>
                <span className={cn("mr-1 inline-block h-1.5 w-1.5 rounded-full", typeStyle(t).dot)} />
                {t}
              </Pill>
            ))}
            <span className="mx-1 h-4 w-px bg-border" />
            <span className="flex items-center gap-1 text-muted-foreground">
              <Workflow className="h-3.5 w-3.5" /> hops
            </span>
            <Segmented value={hops} onChange={setHops} options={[0, 1, 2]} />
            <span className="mx-1 h-4 w-px bg-border" />
            <Pill active={agentic} onClick={() => setAgentic(!agentic)}>
              <Sparkles className="mr-1 h-3.5 w-3.5" /> Synthesize
            </Pill>
          </div>
        </div>
      </div>

      {/* Results */}
      <div className="flex-1 overflow-auto">
        <div className="mx-auto w-full max-w-3xl px-6 py-5">
          {error && (
            <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error}
            </div>
          )}

          {!res && !error && <EmptyState onPick={(ex) => void run(ex)} />}

          {res && (
            <>
              <div className="mb-3 flex items-center gap-3 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">
                  {res.memories.length} {res.memories.length === 1 ? "result" : "results"}
                </span>
                <span className="flex items-center gap-1">
                  <Clock className="h-3.5 w-3.5" /> {res.took_ms} ms
                </span>
                {res.linked.length > 0 && (
                  <span className="flex items-center gap-1">
                    <Link2 className="h-3.5 w-3.5" /> {res.linked.length} linked
                  </span>
                )}
              </div>

              {res.brief && (
                <div className="mb-4 rounded-xl border border-violet-400/25 bg-violet-500/[0.06] p-4">
                  <div className="mb-1.5 flex items-center gap-1.5 text-2xs font-medium uppercase tracking-[0.14em] text-violet-300">
                    <Sparkles className="h-3.5 w-3.5" /> Synthesized answer
                  </div>
                  <p className="whitespace-pre-wrap text-sm leading-relaxed text-foreground/90">
                    {res.brief}
                  </p>
                </div>
              )}

              <div className="space-y-2.5">
                {res.memories.map((m, i) => (
                  <MemoryCard key={m.id} m={m} rel={m.score / maxScore} index={i} reduce={Boolean(reduce)} />
                ))}
              </div>

              {res.memories.length === 0 && (
                <div className="py-12 text-center text-sm text-muted-foreground">
                  No memories matched “{q}”. Try fewer filters or a broader query.
                </div>
              )}

              {res.linked.length > 0 && (
                <div className="mt-5 rounded-xl border border-border/60 bg-card/40 p-4">
                  <div className="mb-2.5 flex items-center gap-1.5 text-2xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
                    <Link2 className="h-3.5 w-3.5" /> Connected resources &amp; traces
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {res.linked.map((l, i) => (
                      <span
                        key={`${l.node_id}-${i}`}
                        title={`${l.target_namespace ?? ""} · ${l.edge_type} · hop ${l.depth}`}
                        className="inline-flex items-center gap-1 rounded-md border border-border/60 bg-background/60 px-2 py-1 font-mono text-[11px]"
                      >
                        <span className="h-1.5 w-1.5 rounded-full bg-teal-400" />
                        {l.node_id}
                      </span>
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function MemoryCard({
  m,
  rel,
  index,
  reduce,
}: {
  m: RetrievedMemory;
  rel: number;
  index: number;
  reduce: boolean;
}) {
  const st = typeStyle(m.memory_type);
  const invalid = !!m.invalid_at;
  return (
    <motion.div
      initial={reduce ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.24, ease: [0.16, 1, 0.3, 1], delay: Math.min(index * 0.03, 0.24) }}
      className="group rounded-xl border border-border/60 bg-card/40 p-4 transition-colors hover:border-border-strong/70 hover:bg-card/70"
    >
      <div className="mb-1.5 flex items-center gap-2">
        <span className={cn("rounded-md px-1.5 py-0.5 text-[11px] font-medium", st.chip)}>
          {m.memory_type}
        </span>
        {m.realm && <span className="text-[11px] text-muted-foreground">{m.realm}</span>}
        {m.via?.type && (
          <Badge variant="secondary" className="gap-1 text-[10px]">
            <Workflow className="h-3 w-3" /> via {m.via.type}
            {typeof m.via.depth === "number" ? ` · hop ${m.via.depth}` : ""}
          </Badge>
        )}
        <span className="ml-auto flex items-center gap-1.5">
          <span className="font-mono text-[11px] tabular-nums text-muted-foreground">
            {m.score.toFixed(2)}
          </span>
          <span className="h-1.5 w-12 overflow-hidden rounded-full bg-muted">
            <span
              className="block h-full rounded-full bg-violet-400/80"
              style={{ width: `${Math.max(6, Math.round(rel * 100))}%` }}
            />
          </span>
        </span>
      </div>

      {m.title && <div className="text-sm font-medium">{m.title}</div>}
      <p className="text-sm leading-relaxed text-foreground/90">{m.content_preview}</p>

      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
        {invalid ? (
          <span className="flex items-center gap-1 text-amber-400">
            <span className="h-1.5 w-1.5 rounded-full bg-amber-400" /> invalidated{" "}
            {shortTime(m.invalid_at)}
          </span>
        ) : (
          <span className="flex items-center gap-1 text-emerald-400/90">
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" /> valid
            {m.valid_at ? ` since ${shortTime(m.valid_at)}` : ""}
          </span>
        )}
        {m.distance != null && <span>semantic {(1 - m.distance).toFixed(2)}</span>}
        {m.kw_score != null && m.kw_score > 0 && <span>keyword {m.kw_score}</span>}
        {m.graph_proximity > 0 && <span>graph {m.graph_proximity.toFixed(2)}</span>}
        <span className="ml-auto font-mono opacity-50">{m.id.replace("memory:", "")}</span>
      </div>
    </motion.div>
  );
}

function EmptyState({ onPick }: { onPick: (ex: string) => void }) {
  return (
    <div className="flex flex-col items-center gap-4 py-16 text-center">
      <div className="relative flex h-14 w-14 items-center justify-center rounded-2xl bg-violet-500/10 ring-1 ring-inset ring-violet-400/20">
        <Search className="h-6 w-6 text-violet-300" />
      </div>
      <div className="max-w-md space-y-1.5">
        <h2 className="text-base font-semibold">Recall anything the engine remembers</h2>
        <p className="text-sm text-muted-foreground">
          Hybrid semantic + keyword recall, graph-expanded over connected memories,
          catalog resources, and traces — ranked and near-realtime.
        </p>
      </div>
      <div className="flex flex-wrap justify-center gap-2">
        {EXAMPLES.map((ex) => (
          <button
            key={ex}
            type="button"
            onClick={() => onPick(ex)}
            className="rounded-full border border-border/60 bg-card/50 px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:border-border-strong/70 hover:text-foreground"
          >
            {ex}
          </button>
        ))}
      </div>
    </div>
  );
}

function Pill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center rounded-full border px-2.5 py-1 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "border-violet-400/40 bg-violet-500/12 text-foreground"
          : "border-border/60 bg-background/50 text-muted-foreground hover:bg-accent hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

function Segmented({
  value,
  onChange,
  options,
}: {
  value: number;
  onChange: (v: number) => void;
  options: number[];
}) {
  return (
    <div className="inline-flex overflow-hidden rounded-full border border-border/60">
      {options.map((o) => (
        <button
          key={o}
          type="button"
          onClick={() => onChange(o)}
          className={cn(
            "px-2.5 py-1 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            value === o
              ? "bg-violet-500/12 text-foreground"
              : "bg-background/50 text-muted-foreground hover:bg-accent",
          )}
        >
          {o}
        </button>
      ))}
    </div>
  );
}
