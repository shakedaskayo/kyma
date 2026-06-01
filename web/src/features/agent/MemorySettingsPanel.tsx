import { useCallback, useEffect, useState } from "react";
import { Brain, RotateCcw, Save, Search, SlidersHorizontal } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  getMemorySettings,
  putMemorySettings,
  type MemorySettings,
} from "@/sdk/memory";

/**
 * Memory → Settings: tune the ingestion pipeline and the agentic-memory recall
 * ranking at runtime (persisted via PUT /v1/agent/memory/settings; read by the
 * consolidator and the recall orchestrator). No redeploy needed.
 */

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
};

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
        setS(d);
        setError(null);
      })
      .catch((e) => setError((e as Error).message));
  }, [endpoint, token]);

  const set = useCallback(<K extends keyof MemorySettings>(k: K, v: MemorySettings[K]) => {
    setS((cur) => (cur ? { ...cur, [k]: v } : cur));
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
      <header className="flex items-center gap-2 border-b px-4 py-2 text-sm">
        <SlidersHorizontal className="h-4 w-4 text-primary" />
        <h1 className="font-semibold tracking-tight">Memory settings</h1>
        <div className="ml-auto flex items-center gap-2">
          {status && <span className="text-xs text-emerald-500">{status}</span>}
          {error && <span className="text-xs text-destructive">{error}</span>}
          <button
            type="button"
            onClick={() => {
              setS({ ...DEFAULTS });
              setStatus("Reset to defaults (unsaved)");
            }}
            className="flex items-center gap-1 rounded border px-2 py-1 text-xs hover:bg-accent"
          >
            <RotateCcw className="h-3 w-3" /> Defaults
          </button>
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground disabled:opacity-50"
          >
            <Save className="h-3.5 w-3.5" /> Save
          </button>
        </div>
      </header>

      <div className="flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-4 px-4 py-4">
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

function Section({ icon, title, desc, children }: { icon: React.ReactNode; title: string; desc: string; children: React.ReactNode }) {
  return (
    <div className="rounded-lg border">
      <div className="flex items-center gap-2 border-b px-3 py-2">
        <span className="text-primary">{icon}</span>
        <span className="text-sm font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">— {desc}</span>
      </div>
      <div className="divide-y">{children}</div>
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
