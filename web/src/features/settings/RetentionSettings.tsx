import { useCallback, useEffect, useState } from "react";
import { Clock, Database, FileArchive, Plug, Plus, Save, Trash2 } from "lucide-react";
import { useSession } from "@/sdk/session";
import {
  getRetentionSettings,
  putRetentionSettings,
  type RetentionSettings as Settings,
} from "@/sdk/retention";

/**
 * Settings → Retention: tune how long ingested data is kept, end-to-end.
 *
 * - Per artifact class (log / file …) and per source (github / fswatch …) drive
 *   object-store blob expiry (the retention worker stamps `expires_at`).
 * - Per table (`database.table`) drives columnar extent retention.
 * - A global default backs them all. Empty / unset = retain forever.
 *
 * Global / source defaults deliberately apply to the expendable artifact blobs
 * only — never to columnar table data, which needs an explicit per-table entry.
 */

type MapKey = "per_artifact_class_days" | "per_source_days" | "per_table_days";

export function RetentionSettings() {
  const { endpoint, token } = useSession();
  const [s, setS] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!endpoint || !token) return;
    getRetentionSettings({ endpoint, token })
      .then((d) => {
        setS(d);
        setError(null);
      })
      .catch((e) => setError((e as Error).message));
  }, [endpoint, token]);

  const setGlobal = useCallback((v: number | null) => {
    setS((cur) => (cur ? { ...cur, global_default_days: v } : cur));
    setStatus(null);
  }, []);

  const setMap = useCallback((field: MapKey, next: Record<string, number>) => {
    setS((cur) => (cur ? { ...cur, [field]: next } : cur));
    setStatus(null);
  }, []);

  const save = useCallback(async () => {
    if (!endpoint || !token || !s) return;
    // Drop empty keys / non-positive values before sending (0 is rejected
    // server-side; blanks are just incomplete rows).
    const clean = (m: Record<string, number>) =>
      Object.fromEntries(
        Object.entries(m).filter(([k, v]) => k.trim() !== "" && Number(v) >= 1),
      );
    const payload: Settings = {
      global_default_days:
        s.global_default_days && s.global_default_days >= 1 ? s.global_default_days : null,
      per_artifact_class_days: clean(s.per_artifact_class_days),
      per_source_days: clean(s.per_source_days),
      per_table_days: clean(s.per_table_days),
    };
    setSaving(true);
    try {
      await putRetentionSettings({ endpoint, token }, payload);
      setS(payload);
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
      <div className="flex h-32 items-center justify-center text-sm text-muted-foreground">
        {error ? <span className="text-destructive">{error}</span> : "Loading settings…"}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2 text-sm">
        <div className="ml-auto flex items-center gap-2">
          {status && <span className="text-xs text-emerald-500">{status}</span>}
          {error && <span className="text-xs text-destructive">{error}</span>}
          <button
            type="button"
            onClick={() => void save()}
            disabled={saving}
            className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-xs font-medium text-primary-foreground disabled:opacity-50"
          >
            <Save className="h-3.5 w-3.5" /> Save
          </button>
        </div>
      </div>

      <Section
        icon={<Clock className="h-4 w-4" />}
        title="Global default"
        desc="Applies to artifact blobs (logs, files) with no more specific rule. Blank = retain forever. Never auto-applies to table data."
      >
        <Row
          label="Default retention"
          hint="Delete artifacts older than this many days unless a source/class rule overrides it."
          control={
            <div className="flex items-center gap-1.5">
              <input
                type="number"
                min={1}
                placeholder="∞"
                value={s.global_default_days ?? ""}
                onChange={(e) =>
                  setGlobal(e.target.value === "" ? null : Number(e.target.value))
                }
                className="w-20 rounded border bg-background px-2 py-1 text-right text-sm"
              />
              <span className="text-xs text-muted-foreground">days</span>
            </div>
          }
        />
      </Section>

      <MapEditor
        icon={<FileArchive className="h-4 w-4" />}
        title="By artifact class"
        desc="Object-store blobs by class, e.g. log → 14, file → 90. Highest precedence."
        keyPlaceholder="log"
        map={s.per_artifact_class_days}
        onChange={(m) => setMap("per_artifact_class_days", m)}
      />

      <MapEditor
        icon={<Plug className="h-4 w-4" />}
        title="By source"
        desc="All artifacts from a data source, e.g. github → 30, fswatch → 7."
        keyPlaceholder="github"
        map={s.per_source_days}
        onChange={(m) => setMap("per_source_days", m)}
      />

      <MapEditor
        icon={<Database className="h-4 w-4" />}
        title="By table (columnar)"
        desc="Explicit per-table retention, keyed database.table, e.g. pensieve.github_job_logs → 30. Required to expire table data."
        keyPlaceholder="database.table"
        map={s.per_table_days}
        onChange={(m) => setMap("per_table_days", m)}
      />
    </div>
  );
}

function MapEditor({
  icon,
  title,
  desc,
  keyPlaceholder,
  map,
  onChange,
}: {
  icon: React.ReactNode;
  title: string;
  desc: string;
  keyPlaceholder: string;
  map: Record<string, number>;
  onChange: (next: Record<string, number>) => void;
}) {
  // Render as an ordered list of [key, days] pairs; editing a key rewrites the
  // map preserving order.
  const rows = Object.entries(map);

  const updateKey = (idx: number, key: string) => {
    const next = rows.map((r, i) => (i === idx ? ([key, r[1]] as [string, number]) : r));
    onChange(Object.fromEntries(next));
  };
  const updateVal = (idx: number, days: number) => {
    const next = rows.map((r, i) => (i === idx ? ([r[0], days] as [string, number]) : r));
    onChange(Object.fromEntries(next));
  };
  const remove = (idx: number) => onChange(Object.fromEntries(rows.filter((_, i) => i !== idx)));
  const add = () => onChange({ ...map, "": 30 });

  return (
    <Section icon={icon} title={title} desc={desc}>
      {rows.length === 0 && (
        <div className="px-3 py-2.5 text-xs text-muted-foreground">No rules — retain forever (unless a broader default applies).</div>
      )}
      {rows.map(([k, v], idx) => (
        <div key={idx} className="flex items-center gap-2 px-3 py-2">
          <input
            type="text"
            value={k}
            placeholder={keyPlaceholder}
            onChange={(e) => updateKey(idx, e.target.value)}
            className="min-w-0 flex-1 rounded border bg-background px-2 py-1 text-sm"
          />
          <input
            type="number"
            min={1}
            value={v}
            onChange={(e) => updateVal(idx, Number(e.target.value))}
            className="w-20 rounded border bg-background px-2 py-1 text-right text-sm"
          />
          <span className="text-xs text-muted-foreground">days</span>
          <button
            type="button"
            onClick={() => remove(idx)}
            className="rounded p-1 text-muted-foreground hover:bg-accent hover:text-destructive"
            aria-label="Remove rule"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <div className="px-3 py-2">
        <button
          type="button"
          onClick={add}
          className="flex items-center gap-1 rounded border px-2 py-1 text-xs hover:bg-accent"
        >
          <Plus className="h-3 w-3" /> Add rule
        </button>
      </div>
    </Section>
  );
}

function Section({
  icon,
  title,
  desc,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  desc: string;
  children: React.ReactNode;
}) {
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

function Row({
  label,
  hint,
  control,
}: {
  label: string;
  hint: string;
  control: React.ReactNode;
}) {
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
