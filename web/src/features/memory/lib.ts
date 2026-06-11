// Shared design tokens + small pure helpers for the Memory surface.
// Memory-type and firehose-event-kind colors live here so the Overview pulse,
// store distribution, recent rows, and Search all read from one palette.

export const num = (v: unknown): number => (typeof v === "number" ? v : Number(v) || 0);
export const str = (v: unknown): string => (v == null ? "" : String(v));

/** Memory types in canonical display order. */
export const MEMORY_TYPES = [
  "fact",
  "decision",
  "preference",
  "learning",
  "procedure",
  "summary",
  "entity",
] as const;

export type MemoryType = (typeof MEMORY_TYPES)[number];

/** Per memory-type accents: a dot color, a soft chip, and a raw hsl for bars. */
export const TYPE_STYLE: Record<string, { dot: string; chip: string; hsl: string; label: string }> = {
  fact: { dot: "bg-sky-400", chip: "bg-sky-500/12 text-sky-300", hsl: "199 89% 60%", label: "fact" },
  decision: { dot: "bg-violet-400", chip: "bg-violet-500/12 text-violet-300", hsl: "258 90% 70%", label: "decision" },
  preference: { dot: "bg-amber-400", chip: "bg-amber-500/12 text-amber-300", hsl: "38 92% 60%", label: "preference" },
  learning: { dot: "bg-emerald-400", chip: "bg-emerald-500/12 text-emerald-300", hsl: "160 70% 52%", label: "learning" },
  procedure: { dot: "bg-cyan-400", chip: "bg-cyan-500/12 text-cyan-300", hsl: "187 80% 55%", label: "procedure" },
  summary: { dot: "bg-slate-400", chip: "bg-slate-500/12 text-slate-300", hsl: "215 16% 64%", label: "summary" },
  entity: { dot: "bg-rose-400", chip: "bg-rose-500/12 text-rose-300", hsl: "350 85% 68%", label: "entity" },
};

export const typeStyle = (t: string) => TYPE_STYLE[t] ?? TYPE_STYLE.summary;

/** Firehose event kinds → accent (dot + raw hsl), used for the live pulse. */
export const KIND_STYLE: Record<string, { dot: string; hsl: string; label: string }> = {
  user_prompt: { dot: "bg-sky-400", hsl: "199 89% 60%", label: "prompt" },
  assistant_turn: { dot: "bg-violet-400", hsl: "258 90% 70%", label: "assistant" },
  tool_use: { dot: "bg-amber-400", hsl: "38 92% 60%", label: "tool" },
  tool_result: { dot: "bg-amber-300", hsl: "45 90% 62%", label: "tool result" },
  session_start: { dot: "bg-emerald-400", hsl: "160 70% 52%", label: "session start" },
  session_end: { dot: "bg-rose-400", hsl: "350 85% 68%", label: "session end" },
  "memory.recall": { dot: "bg-teal-400", hsl: "174 70% 55%", label: "recall" },
  "memory.import": { dot: "bg-emerald-400", hsl: "152 70% 50%", label: "memory saved" },
  "memory.export": { dot: "bg-slate-400", hsl: "215 16% 64%", label: "memory export" },
  "agent.query": { dot: "bg-fuchsia-400", hsl: "289 85% 66%", label: "agent query" },
  "ingest.batch": { dot: "bg-lime-400", hsl: "84 70% 55%", label: "ingestion" },
};

export const kindStyle = (k: string) => KIND_STYLE[k] ?? { dot: "bg-primary", hsl: "183 74% 50%", label: k.replace(/_/g, " ") };

/**
 * Deterministic per-session hue so events from the same conversation share a
 * color thread in the live pulse. Hashes the session id into a pleasant band.
 */
export function sessionHue(sessionId: string): number {
  let h = 0;
  for (let i = 0; i < sessionId.length; i++) h = (h * 31 + sessionId.charCodeAt(i)) >>> 0;
  // Bias toward the cool/violet end (160–290) to stay on-brand.
  return 160 + (h % 130);
}

/** Compact "14:47" / "Jun 7 14:47" style timestamp. */
export function clockTime(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function shortDate(iso: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

/** Humanize a byte-ish / large integer count: 1240 → "1.2k". */
export function compactNum(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Interval seconds → "6h" / "30m" / "1d". */
export function intervalLabel(secs: number): string {
  if (secs % 86400 === 0) return `${secs / 86400}d`;
  if (secs % 3600 === 0) return `${secs / 3600}h`;
  if (secs % 60 === 0) return `${secs / 60}m`;
  return `${secs}s`;
}

/** Backfill horizon: events older than this AT ARRIVAL are history, not live. */
export const BACKFILL_THRESHOLD_MS = 5 * 60 * 1000;
/** Stalled-capture horizon: connected but nothing newer than this = stalled. */
export const STALL_THRESHOLD_MS = 10 * 60 * 1000;

/** True when a source timestamp is too old (or unparseable) to animate as live. */
export function isBackfill(iso: string, arrivalNow: number): boolean {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return true;
  return arrivalNow - t > BACKFILL_THRESHOLD_MS;
}

/** Map an `otel_traces` row (memory/agent/ingest op span) to a pulse event. */
export function spanRowToEvent(row: Record<string, unknown>): {
  key: string; ts: string; kind: string; sessionId: string; realm: string; text: string;
} {
  const ts = str(row.end_time) || str(row.start_time) || new Date().toISOString();
  const kind = str(row.name) || "op";
  const sessionId = str(row.subject) || str(row.trace_id) || "—";
  let text = "";
  try {
    const attrs = JSON.parse(str(row.attributes_json) || "{}") as Record<string, unknown>;
    text =
      str(attrs["memory.query"]) ||
      str(attrs["agent.question"]) ||
      (str(attrs["memory.results"]) ? `${str(attrs["memory.results"])} memories` : "");
  } catch { /* attributes are best-effort */ }
  const key = `span|${str(row.trace_id)}|${str(row.span_id)}|${ts}`;
  return { key, ts, kind, sessionId, realm: "", text };
}
