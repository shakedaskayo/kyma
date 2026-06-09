import { useCallback, useEffect, useState } from "react";
import { Brain, Moon, RotateCcw, Save, Search, ShieldCheck, SlidersHorizontal } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  getMemorySettings,
  putMemorySettings,
  type DreamingSettings,
  type HitlPolicy,
  type MemoryOp,
  type MemorySettings,
  type OpMode,
} from "@/sdk/memory";

/**
 * Memory → Settings: tune the ingestion pipeline and the agentic-memory recall
 * ranking at runtime (persisted via PUT /v1/agent/memory/settings; read by the
 * consolidator and the recall orchestrator). No redeploy needed.
 */

const DREAMING_DEFAULTS: DreamingSettings = {
  enabled: false,
  interval_secs: 21_600, // 6h
  mode: "full",
  realm_scope: [],
  max_tool_calls: 40,
  wall_clock_secs: 300,
  connector_read_budget: 20,
  connector_read_max_bytes: 1_000_000,
  mutation_cap: 100,
};

const HITL_DEFAULTS: HitlPolicy = {
  enabled: false,
  ops: {
    add: "auto",
    update: "post_hoc",
    invalidate: "gate",
    merge: "gate",
    archive: "gate",
    link_entity_cross_realm: "gate",
    relationship_write: "post_hoc",
    promote_file_candidate: "auto",
  },
  confidence_threshold: 0.6,
  realm_scope: [],
  type_scope: [],
};

const DEFAULTS: MemorySettings = {
  extraction_enabled: true,
  min_events: 1,
  default_limit: 8,
  default_expand_hops: 1,
  ann_threshold: 0,
  w_rrf: 1.0,
  w_semantic: 0.6,
  w_keyword: 0.3,
  w_graph: 0.5,
  w_importance: 0.4,
  w_recency: 0.3,
  half_life_days: 30,
  rrf_k: 60,
  dreaming: DREAMING_DEFAULTS,
  hitl: HITL_DEFAULTS,
};

// The op classes surfaced in the approval-policy editor, ordered by risk.
const MEMORY_OPS: { op: MemoryOp; label: string; hint: string }[] = [
  { op: "invalidate", label: "Invalidate", hint: "Mark a memory contradicted / superseded." },
  { op: "merge", label: "Merge", hint: "Fold duplicate/overlapping memories together." },
  { op: "archive", label: "Archive", hint: "Retire a stale memory (hidden from recall)." },
  { op: "link_entity_cross_realm", label: "Cross-realm link", hint: "Link a memory to a node in another realm/namespace." },
  { op: "update", label: "Update", hint: "Rewrite a memory in place." },
  { op: "relationship_write", label: "Relationship", hint: "Write a same-realm relationship/verdict edge." },
  { op: "add", label: "Add", hint: "Create a brand-new memory." },
  { op: "promote_file_candidate", label: "Promote candidate", hint: "Promote a file/symbol candidate into the live graph (high volume)." },
];

// Interval presets (1h / 6h / 12h / 24h) ↔ interval_secs.
const INTERVAL_PRESETS: { label: string; secs: number }[] = [
  { label: "1h", secs: 3_600 },
  { label: "6h", secs: 21_600 },
  { label: "12h", secs: 43_200 },
  { label: "24h", secs: 86_400 },
];

const DREAMING_MODES: { value: DreamingSettings["mode"]; label: string }[] = [
  { value: "full", label: "Full" },
  { value: "housekeeping_only", label: "Housekeeping only" },
  { value: "sources", label: "Sources" },
];

const WEIGHTS: { key: keyof MemorySettings; label: string; hint: string }[] = [
  { key: "w_rrf", label: "RRF fusion", hint: "Weight of reciprocal-rank fusion across the vector + keyword lists." },
  { key: "w_semantic", label: "Semantic", hint: "Weight of vector (cosine) similarity." },
  { key: "w_keyword", label: "Keyword", hint: "Weight of keyword/token overlap." },
  { key: "w_graph", label: "Graph proximity", hint: "Weight of how closely a memory connects to the seed set." },
  { key: "w_importance", label: "Importance", hint: "Weight of a memory's stored importance." },
  { key: "w_recency", label: "Recency", hint: "Weight of how recently a memory was created." },
];

export function MemorySettingsPanel() {
  const { endpoint, token } = useSession();
  const [s, setS] = useState<MemorySettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!endpoint || !token) return;
    getMemorySettings({ endpoint, token })
      .then((d) => {
        // Defensively backfill the dreaming block so the form (and the
        // whole-object PUT) never drops it if an older server omits it.
        setS({
          ...d,
          dreaming: { ...DREAMING_DEFAULTS, ...(d.dreaming ?? {}) },
          hitl: {
            ...HITL_DEFAULTS,
            ...(d.hitl ?? {}),
            ops: { ...HITL_DEFAULTS.ops, ...(d.hitl?.ops ?? {}) },
          },
        });
        setError(null);
      })
      .catch((e) => setError((e as Error).message));
  }, [endpoint, token]);

  const set = useCallback(<K extends keyof MemorySettings>(k: K, v: MemorySettings[K]) => {
    setS((cur) => (cur ? { ...cur, [k]: v } : cur));
    setStatus(null);
  }, []);

  const setDreaming = useCallback(
    <K extends keyof DreamingSettings>(k: K, v: DreamingSettings[K]) => {
      setS((cur) => (cur ? { ...cur, dreaming: { ...cur.dreaming, [k]: v } } : cur));
      setStatus(null);
    },
    [],
  );

  const setHitl = useCallback(<K extends keyof HitlPolicy>(k: K, v: HitlPolicy[K]) => {
    setS((cur) => (cur ? { ...cur, hitl: { ...cur.hitl, [k]: v } } : cur));
    setStatus(null);
  }, []);

  const setOpMode = useCallback((op: MemoryOp, mode: OpMode) => {
    setS((cur) =>
      cur ? { ...cur, hitl: { ...cur.hitl, ops: { ...cur.hitl.ops, [op]: mode } } } : cur,
    );
    setStatus(null);
  }, []);

  const save = useCallback(async () => {
    if (!endpoint || !token || !s) return;
    setSaving(true);
    try {
      await putMemorySettings({ endpoint, token }, s);
      setStatus("Saved");
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  }, [endpoint, token, s]);

  if (!s) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        {error ? <span className="text-destructive">{error}</span> : "Loading settings…"}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header className="sticky top-0 z-10 flex items-center gap-2 border-b border-border/60 bg-surface/50 px-6 py-2.5 text-sm backdrop-blur-md">
        <SlidersHorizontal className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="text-2xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Tune the pipeline &amp; recall
        </span>
        <div className="ml-auto flex items-center gap-2">
          {status && <span className="text-xs text-emerald-400">{status}</span>}
          {error && <span className="text-xs text-destructive">{error}</span>}
          <button
            type="button"
            onClick={() => {
              setS({ ...DEFAULTS });
              setStatus("Reset to defaults (unsaved)");
            }}
            className="flex items-center gap-1 rounded-md border border-border/60 px-2.5 py-1 text-xs transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <RotateCcw className="h-3 w-3" /> Defaults
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Save className="h-3.5 w-3.5" /> Save
          </button>
        </div>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-4 px-4 py-4">
          {/* Dreaming */}
          <Section
            icon={<Moon className="h-4 w-4" />}
            title="Dreaming"
            desc="Autonomous consolidation — the agent distills, merges, and links memories on a schedule."
          >
            <Toggle
              label="Enable dreaming"
              hint="When on, a background agent run consolidates memory at the configured interval."
              value={s.dreaming.enabled}
              onChange={(v) => setDreaming("enabled", v)}
            />
            <StringSelectRow
              label="Mode"
              hint="Full = distill + housekeep + read sources. Housekeeping only = merge/archive/rescore. Sources = pull connector reads."
              value={s.dreaming.mode}
              options={DREAMING_MODES}
              onChange={(v) => setDreaming("mode", v as DreamingSettings["mode"])}
            />
            <StringSelectRow
              label="Interval"
              hint="How often a scheduled dreaming run fires."
              value={String(s.dreaming.interval_secs)}
              options={INTERVAL_PRESETS.map((p) => ({ value: String(p.secs), label: p.label }))}
              onChange={(v) => setDreaming("interval_secs", Number(v))}
            />
            <NumberRow
              label="Max tool calls"
              hint="Hard cap on agent tool calls per run."
              value={s.dreaming.max_tool_calls}
              min={1}
              max={1000}
              step={1}
              onChange={(v) => setDreaming("max_tool_calls", v)}
            />
            <NumberRow
              label="Wall-clock budget (s)"
              hint="Maximum seconds a single run may take before it is stopped."
              value={s.dreaming.wall_clock_secs}
              min={10}
              max={3600}
              step={10}
              onChange={(v) => setDreaming("wall_clock_secs", v)}
            />
            <NumberRow
              label="Mutation cap"
              hint="Maximum number of memory writes/merges/archives a run may perform."
              value={s.dreaming.mutation_cap}
              min={1}
              max={10000}
              step={1}
              onChange={(v) => setDreaming("mutation_cap", v)}
            />
            <NumberRow
              label="Connector read budget"
              hint="Maximum connector reads per run (sources/full mode)."
              value={s.dreaming.connector_read_budget}
              min={0}
              max={1000}
              step={1}
              onChange={(v) => setDreaming("connector_read_budget", v)}
            />
          </Section>

          {/* Approval policy (HITL) */}
          <Section
            icon={<ShieldCheck className="h-4 w-4" />}
            title="Approval policy"
            desc="Human-in-the-loop review for automatic memory mutations. Off = everything applies directly (no review)."
          >
            <Toggle
              label="Require review"
              hint="Master switch. When on, the per-operation modes below take effect and low-confidence ops escalate one level."
              value={s.hitl.enabled}
              onChange={(v) => setHitl("enabled", v)}
            />
            {s.hitl.enabled && (
              <>
                {MEMORY_OPS.map((o) => (
                  <OpModeRow
                    key={o.op}
                    label={o.label}
                    hint={o.hint}
                    value={s.hitl.ops[o.op] ?? "auto"}
                    onChange={(m) => setOpMode(o.op, m)}
                  />
                ))}
                <SliderRow
                  label="Confidence threshold"
                  hint="Ops the model is less confident about than this are escalated one severity level (auto → review → gate)."
                  value={s.hitl.confidence_threshold}
                  min={0}
                  max={1}
                  step={0.05}
                  onChange={(v) => setHitl("confidence_threshold", v)}
                />
                <CsvRow
                  label="Realm scope"
                  hint="Comma-separated realms the policy applies to. Empty = all realms."
                  value={s.hitl.realm_scope}
                  onChange={(v) => setHitl("realm_scope", v)}
                />
                <CsvRow
                  label="Type scope"
                  hint="Comma-separated memory types (fact, decision, …). Empty = all types."
                  value={s.hitl.type_scope}
                  onChange={(v) => setHitl("type_scope", v)}
                />
              </>
            )}
          </Section>

          {/* Ingestion */}
          <Section icon={<Brain className="h-4 w-4" />} title="Ingestion" desc="How conversation/connector activity becomes memory.">
            <Toggle
              label="LLM extraction"
              hint="Extract atomic memories, entities, and relationships with conflict resolution. When off, the pipeline writes deterministic activity summaries."
              value={s.extraction_enabled}
              onChange={(v) => set("extraction_enabled", v)}
            />
            <NumberRow
              label="Min events to consolidate"
              hint="A project realm is only consolidated once it has at least this many new firehose events."
              value={s.min_events}
              min={1}
              max={1000}
              step={1}
              onChange={(v) => set("min_events", v)}
            />
          </Section>

          {/* Retrieval defaults */}
          <Section icon={<Search className="h-4 w-4" />} title="Retrieval defaults" desc="Defaults applied when a query doesn't override them.">
            <NumberRow label="Default result limit" hint="How many memories recall returns by default." value={s.default_limit} min={1} max={100} step={1} onChange={(v) => set("default_limit", v)} />
            <SelectRow label="Default graph hops" hint="How far to graph-expand from the top hits (connected memories + resources/traces)." value={s.default_expand_hops} options={[0, 1, 2]} onChange={(v) => set("default_expand_hops", v)} />
            <SliderRow label="ANN threshold" hint="Native columnar ANN cosine-distance cutoff for extent pruning. 0 = off (exact full scan). Higher prunes more aggressively." value={s.ann_threshold} min={0} max={2} step={0.05} onChange={(v) => set("ann_threshold", v)} suffix={s.ann_threshold === 0 ? "off" : undefined} />
          </Section>

          {/* Ranking weights */}
          <Section icon={<SlidersHorizontal className="h-4 w-4" />} title="Ranking weights" desc="The hybrid relevance blend. Higher weights favor that signal.">
            {WEIGHTS.map((w) => (
              <SliderRow
                key={w.key}
                label={w.label}
                hint={w.hint}
                value={s[w.key] as number}
                min={0}
                max={2}
                step={0.05}
                onChange={(v) => set(w.key, v as never)}
              />
            ))}
            <NumberRow label="Recency half-life (days)" hint="A memory's recency score halves every N days." value={s.half_life_days} min={1} max={365} step={1} onChange={(v) => set("half_life_days", v)} />
            <NumberRow label="RRF k" hint="Reciprocal-rank-fusion constant 1/(k + rank). Larger flattens the rank advantage." value={s.rrf_k} min={1} max={200} step={1} onChange={(v) => set("rrf_k", v)} />
          </Section>
        </div>
      </div>
    </div>
  );
}

function OpModeRow({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: OpMode;
  onChange: (m: OpMode) => void;
}) {
  const modes: { v: OpMode; l: string }[] = [
    { v: "auto", l: "Auto" },
    { v: "post_hoc", l: "Review" },
    { v: "gate", l: "Gate" },
  ];
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <div className="flex overflow-hidden rounded-md border border-border/60">
          {modes.map((m) => (
            <button
              key={m.v}
              type="button"
              onClick={() => onChange(m.v)}
              className={`px-2 py-1 text-2xs transition-colors ${
                value === m.v
                  ? "bg-primary text-primary-foreground"
                  : "bg-background text-muted-foreground hover:bg-accent"
              }`}
            >
              {m.l}
            </button>
          ))}
        </div>
      }
    />
  );
}

function CsvRow({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint: string;
  value: string[];
  onChange: (v: string[]) => void;
}) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <input
          type="text"
          value={value.join(", ")}
          onChange={(e) =>
            onChange(
              e.target.value
                .split(",")
                .map((x) => x.trim())
                .filter(Boolean),
            )
          }
          placeholder="all"
          className="w-44 rounded border bg-background px-2 py-1 text-sm"
        />
      }
    />
  );
}

function Section({ icon, title, desc, children }: { icon: React.ReactNode; title: string; desc: string; children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl border border-border/60 bg-card/40 shadow-elev-1">
      <div className="flex items-center gap-2 border-b border-border/50 px-4 py-2.5">
        <span className="text-primary/80">{icon}</span>
        <span className="text-sm font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">— {desc}</span>
      </div>
      <div className="divide-y divide-border/40">{children}</div>
    </div>
  );
}

function Row({ label, hint, control }: { label: string; hint: string; control: React.ReactNode }) {
  return (
    <div className="flex items-start gap-4 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="text-sm">{label}</div>
        <div className="text-xs text-muted-foreground">{hint}</div>
      </div>
      <div className="flex shrink-0 items-center gap-2">{control}</div>
    </div>
  );
}

function Toggle({ label, hint, value, onChange }: { label: string; hint: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <button
          type="button"
          onClick={() => onChange(!value)}
          className={`h-5 w-9 rounded-full transition-colors ${value ? "bg-primary" : "bg-muted"}`}
        >
          <span className={`block h-4 w-4 rounded-full bg-background transition-transform ${value ? "translate-x-4" : "translate-x-0.5"}`} />
        </button>
      }
    />
  );
}

function NumberRow({ label, hint, value, min, max, step, onChange }: { label: string; hint: string; value: number; min: number; max: number; step: number; onChange: (v: number) => void }) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <input
          type="number"
          value={value}
          min={min}
          max={max}
          step={step}
          onChange={(e) => onChange(Number(e.target.value))}
          className="w-20 rounded border bg-background px-2 py-1 text-right text-sm"
        />
      }
    />
  );
}

function SelectRow({ label, hint, value, options, onChange }: { label: string; hint: string; value: number; options: number[]; onChange: (v: number) => void }) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <select value={value} onChange={(e) => onChange(Number(e.target.value))} className="rounded border bg-background px-2 py-1 text-sm">
          {options.map((o) => (
            <option key={o} value={o}>
              {o}
            </option>
          ))}
        </select>
      }
    />
  );
}

function StringSelectRow({ label, hint, value, options, onChange }: { label: string; hint: string; value: string; options: { value: string; label: string }[]; onChange: (v: string) => void }) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <select value={value} onChange={(e) => onChange(e.target.value)} className="rounded border bg-background px-2 py-1 text-sm">
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      }
    />
  );
}

function SliderRow({ label, hint, value, min, max, step, onChange, suffix }: { label: string; hint: string; value: number; min: number; max: number; step: number; onChange: (v: number) => void; suffix?: string }) {
  return (
    <Row
      label={label}
      hint={hint}
      control={
        <>
          <input type="range" value={value} min={min} max={max} step={step} onChange={(e) => onChange(Number(e.target.value))} className="w-32 accent-primary" />
          <span className="w-12 text-right font-mono text-xs text-muted-foreground">{suffix ?? value.toFixed(2)}</span>
        </>
      }
    />
  );
}
