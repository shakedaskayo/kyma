# Discover v2 Phase 1 — Unified Stream UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Discover's per-source stacked tables with a single time-sorted event stream, a persistent query bar (pills removed), an honest summary line, and a labeled/brushable histogram — on the existing one-shot `/v1/explore/search` protocol.

**Architecture:** Pure merge/summary logic lives in small testable modules (`stream.ts`, `rowSummary.ts`); React components compose them. The discover tab state drops `pills` for a plain `search` string (the backend already parses the same grammar server-side); persisted workspaces migrate v2→v3. One additive backend change: the `plan` frame gains `timestamp_column` per source so the client can sort rows without guessing.

**Tech Stack:** React 18 + TanStack Router, Zustand persist, vitest, Tailwind; axum/serde for the one backend frame change.

**Branch/commit policy for this plan:** the working tree carries unrelated in-flight work from a parallel session. Every commit must `git add` ONLY the paths named in the task. Working on `main` (matching how this session's bug fixes were left); the user can branch/rearrange later.

**Spec:** `docs/superpowers/specs/2026-06-05-discover-live-stream-design.md`

---

### Task 1: Backend — `timestamp_column` in plan frames

The client needs to know each source's timestamp column to time-sort merged rows. `compile.rs` already computes it (`find_timestamp_column`); expose it through `PlanSource`.

**Files:**
- Modify: `crates/pensieve-server/src/discover/frames.rs` (PlanSource struct + its test)
- Modify: `crates/pensieve-server/src/discover/compile.rs` (expose ts col on CompiledSource if not already public)
- Modify: `crates/pensieve-server/src/discover/fanout.rs` (populate the new field where the Plan frame is built)
- Modify: `web/src/sdk/discover.ts` (Frame type), `web/src/features/discover/types.ts` (SourceState), `web/src/features/discover/discover-store.ts` (applyFrame)

- [ ] **Step 1: Write the failing Rust test** — extend the existing round-trip test in `frames.rs`:

```rust
#[test]
fn plan_frame_round_trips() {
    let f = Frame::Plan {
        sources: vec![
            PlanSource {
                source: "prod.otel_logs".into(),
                has_timestamp: true,
                timestamp_column: Some("timestamp".into()),
            },
            PlanSource {
                source: "prod.metrics".into(),
                has_timestamp: false,
                timestamp_column: None,
            },
        ],
    };
    let line = frame_to_line(&f);
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["sources"][0]["timestamp_column"], "timestamp");
    assert!(v["sources"][1]["timestamp_column"].is_null());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p pensieve-server --lib discover::frames`
Expected: FAIL — `PlanSource` has no field `timestamp_column`.

- [ ] **Step 3: Add the field**

In `frames.rs`:
```rust
#[derive(Debug, Clone, Serialize)]
pub struct PlanSource {
    pub source: String, // "db.table"
    pub has_timestamp: bool,
    /// Name of the timestamp-typed column used for time filtering/sorting,
    /// when one exists. Additive — older clients ignore it.
    pub timestamp_column: Option<String>,
}
```

In `fanout.rs`, find where `PlanSource` values are constructed (grep `PlanSource {`). The compiled source (from `compile.rs`) computes `ts_col` — if `CompiledSource` doesn't already carry it publicly, add `pub timestamp_column: Option<String>` to it in `compile.rs` (set from the existing local `ts_col`) and populate:
```rust
PlanSource {
    source: src.source_key.clone(),
    has_timestamp: compiled.has_timestamp,
    timestamp_column: compiled.timestamp_column.clone(),
}
```

- [ ] **Step 4: Run backend tests**

Run: `cargo test -p pensieve-server --lib discover`
Expected: PASS (all discover unit tests).

- [ ] **Step 5: Mirror in the frontend types**

`web/src/sdk/discover.ts` — in the `Frame` union's plan variant, the source entries gain `timestamp_column?: string | null`.
`web/src/features/discover/types.ts`:
```ts
export type SourceState = {
  source: SourceKey;
  hasTimestamp: boolean;
  timestampColumn: string | null;
  // ...existing fields unchanged
};
```
`web/src/features/discover/discover-store.ts` `applyFrame` plan case:
```ts
{
  source: s.source,
  hasTimestamp: s.has_timestamp,
  timestampColumn: s.timestamp_column ?? null,
  progress: "pending",
  rows: [],
  total: 0,
  capped: false,
  droppedClauses: [],
}
```

- [ ] **Step 6: Typecheck + web tests**

Run: `cd web && npx tsc --noEmit -p tsconfig.json && npx vitest run src/features/discover`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-server/src/discover/frames.rs crates/pensieve-server/src/discover/compile.rs crates/pensieve-server/src/discover/fanout.rs web/src/sdk/discover.ts web/src/features/discover/types.ts web/src/features/discover/discover-store.ts
git commit -m "feat(discover): expose timestamp_column in plan frames"
```

---

### Task 2: `stream.ts` — pure merge logic

**Files:**
- Create: `web/src/features/discover/stream.ts`
- Test: `web/src/features/discover/stream.test.ts`

- [ ] **Step 1: Write the failing tests**

```ts
import { expect, test } from "vitest";
import { mergeSources, type StreamRow } from "./stream";
import type { SourceState } from "./types";

function src(over: Partial<SourceState>): SourceState {
  return {
    source: "db.t",
    hasTimestamp: true,
    timestampColumn: "timestamp",
    progress: "done",
    rows: [],
    total: 0,
    capped: false,
    droppedClauses: [],
    ...over,
  };
}

test("mergeSources merges rows across sources sorted by time desc", () => {
  const a = src({
    source: "db.a",
    rows: [
      { timestamp: "2026-06-05T10:00:00Z", m: "a-old" },
      { timestamp: "2026-06-05T12:00:00Z", m: "a-new" },
    ],
  });
  const b = src({
    source: "db.b",
    rows: [{ timestamp: "2026-06-05T11:00:00Z", m: "b-mid" }],
  });
  const out = mergeSources([a, b]);
  expect(out.map((r) => r.row.m)).toEqual(["a-new", "b-mid", "a-old"]);
  expect(out[0].source).toBe("db.a");
  expect(out[0].ts).toBe(Date.parse("2026-06-05T12:00:00Z"));
});

test("mergeSources skips sources without a timestamp column", () => {
  const noTs = src({ source: "db.ref", hasTimestamp: false, timestampColumn: null, rows: [{ id: 1 }] });
  const withTs = src({ rows: [{ timestamp: "2026-06-05T10:00:00Z" }] });
  expect(mergeSources([noTs, withTs])).toHaveLength(1);
});

test("mergeSources tolerates rows with missing/garbage timestamps by sinking them", () => {
  const a = src({
    rows: [
      { timestamp: "2026-06-05T10:00:00Z", m: "good" },
      { timestamp: "not a date", m: "bad" },
      { m: "missing" },
    ],
  });
  const out = mergeSources([a]);
  expect(out[0].row.m).toBe("good");
  expect(out.slice(1).map((r) => r.ts)).toEqual([null, null]);
});

test("mergeSources respects the visible filter", () => {
  const a = src({ source: "db.a", rows: [{ timestamp: "2026-06-05T10:00:00Z" }] });
  const b = src({ source: "db.b", rows: [{ timestamp: "2026-06-05T11:00:00Z" }] });
  const out = mergeSources([a, b], ["db.a"]);
  expect(out).toHaveLength(1);
  expect(out[0].source).toBe("db.a");
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/features/discover/stream.test.ts`
Expected: FAIL — module `./stream` not found.

- [ ] **Step 3: Implement**

```ts
// Pure stream-merge logic for the unified Discover timeline.
//
// Sources stream in independently; the page shows one time-sorted (desc)
// stream. Sorting is a full re-sort per update — a few thousand rows is
// milliseconds, and it keeps this trivially correct. Phase 2 (live) can add
// incremental insertion behind the same StreamRow shape.

import type { SourceKey, SourceState } from "./types";

export type StreamRow = {
  /** Epoch millis, or null when the row's timestamp failed to parse. */
  ts: number | null;
  source: SourceKey;
  row: Record<string, unknown>;
};

export function mergeSources(
  sources: SourceState[],
  visible?: SourceKey[] | null,
): StreamRow[] {
  const out: StreamRow[] = [];
  for (const s of sources) {
    if (!s.timestampColumn) continue; // not in timeline
    if (visible != null && !visible.includes(s.source)) continue;
    for (const row of s.rows) {
      const raw = row[s.timestampColumn];
      const parsed = typeof raw === "string" || typeof raw === "number" ? Date.parse(String(raw)) : NaN;
      out.push({ ts: Number.isNaN(parsed) ? null : parsed, source: s.source, row });
    }
  }
  // Desc by time; unparseable timestamps sink to the bottom.
  out.sort((a, b) => (b.ts ?? -Infinity) - (a.ts ?? -Infinity));
  return out;
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd web && npx vitest run src/features/discover/stream.test.ts`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/features/discover/stream.ts web/src/features/discover/stream.test.ts
git commit -m "feat(discover): pure time-sorted stream merge"
```

---

### Task 3: `rowSummary.ts` — message-field picker + k=v summary

**Files:**
- Create: `web/src/features/discover/rowSummary.ts`
- Test: `web/src/features/discover/rowSummary.test.ts`

- [ ] **Step 1: Write the failing tests**

```ts
import { expect, test } from "vitest";
import { pickMessageField, summarizeRow } from "./rowSummary";

test("pickMessageField prefers well-known message names", () => {
  const rows = [{ message: "hi", id: "1", note: "a much longer string than message" }];
  expect(pickMessageField(rows)).toBe("message");
});

test("pickMessageField falls back to the longest avg string field", () => {
  const rows = [
    { id: "1", detail: "a fairly long description of the event" },
    { id: "2", detail: "another long description here" },
  ];
  expect(pickMessageField(rows)).toBe("detail");
});

test("pickMessageField returns null for rows with no string fields", () => {
  expect(pickMessageField([{ n: 1, b: true }])).toBe(null);
});

test("summarizeRow returns primary text and remaining k=v pairs", () => {
  const row = { timestamp: "t", message: "boom", service: "api", code: 500 };
  const s = summarizeRow(row, "message", "timestamp", ["code"]);
  expect(s.primary).toBe("boom");
  expect(s.rest).toEqual([["service", "api"]]); // ts column and excluded cols dropped
});

test("summarizeRow without a message field puts everything in rest", () => {
  const s = summarizeRow({ a: 1, b: "x" }, null, null, []);
  expect(s.primary).toBe(null);
  expect(s.rest).toEqual([["a", "1"], ["b", "x"]]);
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/features/discover/rowSummary.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

```ts
// Smart row summaries for the unified stream: pick a "message-ish" field to
// lead with, render the rest as dimmed k=v pairs. Vector columns are excluded
// by the caller (see columns.ts partitionColumns).

import { formatCell } from "./columns";

const MESSAGE_NAMES = ["message", "msg", "body", "content", "log", "text"];
const SAMPLE = 20;

export function pickMessageField(rows: Record<string, unknown>[]): string | null {
  const sample = rows.slice(0, SAMPLE);
  if (sample.length === 0) return null;
  const fields = new Set<string>();
  for (const r of sample) for (const k of Object.keys(r)) fields.add(k);

  for (const name of MESSAGE_NAMES) if (fields.has(name)) return name;

  // Fallback: the string field with the longest average value.
  let best: string | null = null;
  let bestAvg = 0;
  for (const f of fields) {
    const vals = sample.map((r) => r[f]).filter((v): v is string => typeof v === "string");
    if (vals.length === 0) continue;
    const avg = vals.reduce((s, v) => s + v.length, 0) / vals.length;
    if (avg > bestAvg) {
      bestAvg = avg;
      best = f;
    }
  }
  return best;
}

export function summarizeRow(
  row: Record<string, unknown>,
  messageField: string | null,
  timestampColumn: string | null,
  excludeColumns: string[],
): { primary: string | null; rest: [string, string][] } {
  const primary = messageField != null && row[messageField] != null ? formatCell(row[messageField]) : null;
  const rest: [string, string][] = [];
  for (const [k, v] of Object.entries(row)) {
    if (k === messageField || k === timestampColumn || excludeColumns.includes(k)) continue;
    if (v == null) continue;
    rest.push([k, formatCell(v)]);
  }
  rest.sort(([a], [b]) => a.localeCompare(b));
  return { primary, rest };
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd web && npx vitest run src/features/discover/rowSummary.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/features/discover/rowSummary.ts web/src/features/discover/rowSummary.test.ts
git commit -m "feat(discover): row summary picker for the unified stream"
```

---

### Task 4: Tab state v3 — pills → search text, columns, viewMode

**Files:**
- Modify: `web/src/features/discover/discover-store.ts` (DiscoverTabState)
- Modify: `web/src/features/tabs/workspace-store.ts` (migration v2→v3, version bump, tabIdentity)
- Test: `web/src/features/tabs/workspace-store.test.ts`, `web/src/features/discover/discover-store.test.ts`

- [ ] **Step 1: Write the failing migration test** (in `workspace-store.test.ts`)

```ts
test("migrateWorkspace v2→v3 folds discover pills into the search text", () => {
  const v2 = {
    state: {
      tabs: [
        {
          id: "d",
          kind: "discover",
          state: {
            scope: { kind: "all" },
            search: "draft text",
            pills: [
              { kind: "substring", value: "auth" },
              { kind: "eq", field: "service", value: "payments" },
            ],
            timeRange: { preset: "1h" },
            visibleSources: null,
            selectedSource: null,
            results: { status: "idle", sources: new Map() },
          },
        },
      ],
      activeId: "d",
    },
  };
  const m = migrateWorkspace(v2, 2) as {
    state: { tabs: Array<{ state: { search: string; pills?: unknown; columns: string[]; viewMode: string } }> };
  };
  const st = m.state.tabs[0].state;
  // Pills serialize back to grammar text and join the draft.
  expect(st.search).toBe("auth service:payments draft text");
  expect(st.pills).toBeUndefined();
  expect(st.columns).toEqual([]);
  expect(st.viewMode).toBe("stream");
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && npx vitest run src/features/tabs/workspace-store.test.ts`
Expected: FAIL — search keeps "draft text" only / pills still present.

- [ ] **Step 3: Update `DiscoverTabState`** in `discover-store.ts`:

```ts
export type DiscoverTabState = {
  scope: Scope;
  /** The persistent query bar text — the single source of truth. Sent
   * verbatim as `query`; the backend parses the grammar. */
  search: string;
  timeRange: TimeRange;
  visibleSources: SourceKey[] | null;
  selectedSource: SourceKey | null;
  /** Explicit columns toggled on from the fields rail. */
  columns: string[];
  /** stream = unified timeline; table:<source> = plain table of one source. */
  viewMode: "stream" | { table: SourceKey };
  results: DiscoverResultsState;
};

export const initialDiscoverTabState = (): DiscoverTabState => ({
  scope: { kind: "all" },
  search: "",
  timeRange: { preset: "1h" },
  visibleSources: null,
  selectedSource: null,
  columns: [],
  viewMode: "stream",
  results: { status: "idle", sources: new Map() },
});
```

(`Pill` type stays in `types.ts` — `discoverGrammar.ts` still uses it as the parse artifact for client-side validation and `compileToKql`.)

- [ ] **Step 4: Add the v3 migration** in `workspace-store.ts` — bump `version: 3` and extend `migrateWorkspace`:

```ts
import { serializePills } from "../discover/discoverGrammar";

// inside migrateWorkspace, after the v1→v2 block:
if (fromVersion < 3) {
  p.state.tabs = (p.state.tabs as Array<Record<string, unknown>>).map((t) => {
    if (t.kind !== "discover") return t;
    const st = t.state as {
      pills?: unknown[];
      search?: string;
      columns?: string[];
      viewMode?: unknown;
    } & Record<string, unknown>;
    const pillText = Array.isArray(st.pills) && st.pills.length > 0
      ? serializePills(st.pills as never)
      : "";
    const search = [pillText, st.search ?? ""].filter(Boolean).join(" ").trim();
    const { pills: _pills, ...rest } = st;
    return {
      ...t,
      state: { ...rest, search, columns: st.columns ?? [], viewMode: "stream" },
    };
  });
}
```

Also update `tabIdentity` (discover arm) — pills are gone:
```ts
const { scope, search } = t.state;
return `d|${search}|${JSON.stringify(scope)}`;
```

- [ ] **Step 5: Fix compile errors across the feature** — `DiscoverPage.tsx`, `useDiscoverSearch.ts`, and `discover-store.test.ts` reference `pills`. Patch them minimally so the suite compiles (full rewrites come in Tasks 5–9): `useDiscoverSearch` takes `search: string` and sends it verbatim (`query: args.search`); `DiscoverPage` passes `st.search` and drops pill handlers (the page is rewritten in Task 9 anyway). Update `discover-store.test.ts` fixtures to the new state shape.

- [ ] **Step 6: Run the whole web suite + typecheck**

Run: `cd web && npx tsc --noEmit -p tsconfig.json && npx vitest run src/features`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add web/src/features/discover/discover-store.ts web/src/features/discover/discover-store.test.ts web/src/features/tabs/workspace-store.ts web/src/features/tabs/workspace-store.test.ts web/src/features/discover/useDiscoverSearch.ts web/src/features/discover/DiscoverPage.tsx
git commit -m "feat(discover): search text becomes the query source of truth (v3 migration)"
```

---

### Task 5: `QueryBar.tsx` — persistent bar with inline grammar errors

**Files:**
- Create: `web/src/features/discover/QueryBar.tsx`
- Delete (in Task 9 cleanup): `SearchBar.tsx`, `FilterPills.tsx`

- [ ] **Step 1: Implement** (presentational; logic already covered by grammar tests)

```tsx
import { useMemo } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Loader2, Play, X } from "lucide-react";
import { parseSearch } from "./discoverGrammar";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  onCancel: () => void;
  running: boolean;
};

/** Persistent query bar — the text IS the query. Validates the grammar
 * client-side for inline feedback; the backend re-parses authoritatively. */
export function QueryBar({ value, onChange, onRun, onCancel, running }: Props) {
  const grammarError = useMemo(() => {
    try {
      parseSearch(value);
      return null;
    } catch (e) {
      return (e as Error).message;
    }
  }, [value]);

  return (
    <div className="flex flex-col gap-1 flex-1 min-w-0">
      <div className="flex gap-2 items-center">
        <Input
          placeholder='Search… e.g. auth service:payments -severity:INFO  (empty = everything)'
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !grammarError) onRun();
          }}
          aria-invalid={grammarError != null}
          className="font-mono text-sm flex-1 min-w-0"
        />
        {running ? (
          <Button variant="destructive" size="sm" onClick={onCancel}>
            <X className="size-4 mr-1" /> Cancel
          </Button>
        ) : (
          <Button size="sm" onClick={onRun} disabled={grammarError != null}>
            <Play className="size-4 mr-1" /> Run
          </Button>
        )}
        {running && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
      </div>
      {grammarError && (
        <div className="text-xs text-destructive px-1" role="alert">
          {grammarError}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Typecheck**

Run: `cd web && npx tsc --noEmit -p tsconfig.json`
Expected: PASS (component not yet wired; Task 9 wires it).

- [ ] **Step 3: Commit**

```bash
git add web/src/features/discover/QueryBar.tsx
git commit -m "feat(discover): persistent query bar with inline grammar errors"
```

---

### Task 6: Histogram — labeled time axis + brush-to-zoom

**Files:**
- Modify: `web/src/features/discover/Histogram.tsx`

- [ ] **Step 1: Implement.** Keep the stacked-bar logic; add (a) first/middle/last bucket time labels under the bars, (b) drag-brush that calls `onZoom(fromIso, toIso)`.

```tsx
import { useMemo, useRef, useState } from "react";
import type { DiscoverResultsState } from "./types";

const PALETTE = [
  "#3b82f6", "#10b981", "#f59e0b", "#ef4444",
  "#8b5cf6", "#ec4899", "#14b8a6", "#f97316",
];

type Props = {
  results: DiscoverResultsState;
  onZoom?: (fromIso: string, toIso: string) => void;
};

export function Histogram({ results, onZoom }: Props) {
  const { bars, sourceColors } = useMemo(() => stack(results), [results]);
  const [drag, setDrag] = useState<{ start: number; end: number } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  if (bars.length === 0) return null;

  const max = bars.reduce((m, b) => Math.max(m, b.total), 0) || 1;
  const fmt = (iso: string) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  };
  const idxAt = (clientX: number) => {
    const el = wrap.current;
    if (!el) return 0;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return Math.min(bars.length - 1, Math.floor(frac * bars.length));
  };
  const finishDrag = () => {
    if (!drag || !onZoom) return setDrag(null);
    const [lo, hi] = [Math.min(drag.start, drag.end), Math.max(drag.start, drag.end)];
    if (hi > lo) {
      // Zoom to [start of lo bucket, start of bucket after hi] (or last label).
      const from = bars[lo].label;
      const to = bars[hi + 1]?.label ?? new Date().toISOString();
      onZoom(from, to);
    }
    setDrag(null);
  };
  const inDrag = (i: number) =>
    drag != null && i >= Math.min(drag.start, drag.end) && i <= Math.max(drag.start, drag.end);

  return (
    <div className="border-b select-none">
      <div
        ref={wrap}
        className="flex items-end h-24 px-2 gap-px cursor-crosshair"
        onMouseDown={(e) => setDrag({ start: idxAt(e.clientX), end: idxAt(e.clientX) })}
        onMouseMove={(e) => drag && setDrag({ ...drag, end: idxAt(e.clientX) })}
        onMouseUp={finishDrag}
        onMouseLeave={() => drag && finishDrag()}
      >
        {bars.map((b, i) => (
          <div
            key={i}
            className={`flex-1 flex flex-col-reverse ${inDrag(i) ? "bg-accent" : ""}`}
            title={`${fmt(b.label)} — ${b.total} events`}
          >
            {b.segments.map((seg) => (
              <div
                key={seg.source}
                style={{
                  height: `${(seg.n / max) * 100}%`,
                  backgroundColor: PALETTE[seg.colorIdx % PALETTE.length],
                }}
              />
            ))}
          </div>
        ))}
      </div>
      {/* Time axis: first / middle / last bucket labels. */}
      <div className="flex justify-between px-2 text-[10px] text-muted-foreground tabular-nums">
        <span>{fmt(bars[0].label)}</span>
        {bars.length > 2 && <span>{fmt(bars[Math.floor(bars.length / 2)].label)}</span>}
        <span>{fmt(bars[bars.length - 1].label)}</span>
      </div>
      <div className="flex gap-3 px-2 py-1 text-[10px] text-muted-foreground flex-wrap">
        {Array.from(sourceColors.entries()).map(([src, idx]) => (
          <span key={src} className="inline-flex items-center gap-1">
            <span
              className="inline-block size-2 rounded-sm"
              style={{ backgroundColor: PALETTE[idx % PALETTE.length] }}
            />
            <span className="font-mono">{src}</span>
          </span>
        ))}
      </div>
    </div>
  );
}
```

(`stack()` stays exactly as it is today.)

- [ ] **Step 2: Typecheck + commit**

Run: `cd web && npx tsc --noEmit -p tsconfig.json`
```bash
git add web/src/features/discover/Histogram.tsx
git commit -m "feat(discover): histogram time-axis labels + brush-to-zoom"
```

---

### Task 7: SourcesRail — timeline/not-in-timeline groups

**Files:**
- Modify: `web/src/features/discover/SourcesRail.tsx`

- [ ] **Step 1: Implement.** Split sources by `timestampColumn != null`; keep checkbox/progress/count rows; clicking a not-in-timeline source selects it AND switches view mode (callback).

```tsx
import { Loader2, AlertCircle, CheckCircle2, Table2 } from "lucide-react";
import type { DiscoverResultsState, SourceKey, SourceState } from "./types";

type Props = {
  results: DiscoverResultsState;
  visible: SourceKey[] | null; // null = all visible
  onToggleVisible: (s: SourceKey) => void;
  selected: SourceKey | null;
  onSelect: (s: SourceKey) => void;
  onOpenTable: (s: SourceKey) => void;
};

function Row({
  s, isVisible, isSelected, onToggleVisible, onClick, showCheckbox,
}: {
  s: SourceState;
  isVisible: boolean;
  isSelected: boolean;
  onToggleVisible: () => void;
  onClick: () => void;
  showCheckbox: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-2 px-2 py-1 rounded text-sm cursor-pointer hover:bg-accent ${isSelected ? "bg-accent" : ""}`}
      onClick={onClick}
    >
      {showCheckbox ? (
        <input
          type="checkbox"
          checked={isVisible}
          onChange={(e) => { e.stopPropagation(); onToggleVisible(); }}
          onClick={(e) => e.stopPropagation()}
          className="size-3.5 accent-foreground cursor-pointer"
          aria-label={`toggle visibility of ${s.source}`}
        />
      ) : (
        <Table2 className="size-3.5 text-muted-foreground" />
      )}
      <span className="truncate flex-1 font-mono text-xs" title={s.source}>
        {s.source}
      </span>
      {s.progress === "running" && <Loader2 className="size-3 animate-spin text-muted-foreground" />}
      {s.progress === "done" && <CheckCircle2 className="size-3 text-muted-foreground" />}
      {s.progress === "error" && <AlertCircle className="size-3 text-destructive" />}
      <span className="text-xs text-muted-foreground tabular-nums">{s.total.toLocaleString()}</span>
    </div>
  );
}

export function SourcesRail({
  results, visible, onToggleVisible, selected, onSelect, onOpenTable,
}: Props) {
  const all = Array.from(results.sources.values());
  if (all.length === 0 && results.status === "idle") {
    return <div className="text-xs text-muted-foreground p-2">Run a search.</div>;
  }
  const inTimeline = all.filter((s) => s.timestampColumn != null);
  const noTimeline = all.filter((s) => s.timestampColumn == null);

  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1 uppercase tracking-wide">
        Sources
      </div>
      {inTimeline.map((s) => (
        <Row
          key={s.source}
          s={s}
          showCheckbox
          isVisible={visible == null || visible.includes(s.source)}
          isSelected={selected === s.source}
          onToggleVisible={() => onToggleVisible(s.source)}
          onClick={() => onSelect(s.source)}
        />
      ))}
      {noTimeline.length > 0 && (
        <>
          <div
            className="text-xs font-medium text-muted-foreground px-2 pt-2 pb-1 uppercase tracking-wide"
            title="These sources have no timestamp-typed column, so they can't join the timeline. Click one to view it as a table."
          >
            Not in timeline
          </div>
          {noTimeline.map((s) => (
            <Row
              key={s.source}
              s={s}
              showCheckbox={false}
              isVisible
              isSelected={selected === s.source}
              onToggleVisible={() => {}}
              onClick={() => { onSelect(s.source); onOpenTable(s.source); }}
            />
          ))}
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Typecheck + commit**

Run: `cd web && npx tsc --noEmit -p tsconfig.json` (DiscoverPage still passes old props — patch the call site minimally or defer errors to Task 9 if the compiler complains; do NOT leave the tree red: pass `onOpenTable={() => {}}` temporarily.)

```bash
git add web/src/features/discover/SourcesRail.tsx web/src/features/discover/DiscoverPage.tsx
git commit -m "feat(discover): sources rail with not-in-timeline group"
```

---

### Task 8: FieldsRail — column toggles + filter insertion

**Files:**
- Modify: `web/src/features/discover/FieldsRail.tsx`

- [ ] **Step 1: Implement.** Replace the `+exists` pill button with: click field name → toggle as explicit column; small `filter` affordance → insert `field:*` into the query bar text.

```tsx
import { useMemo } from "react";
import { Columns3, Filter } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { SourceState } from "./types";

type Props = {
  source: SourceState | null;
  columns: string[];
  onToggleColumn: (field: string) => void;
  onInsertFilter: (text: string) => void;
};

export function FieldsRail({ source, columns, onToggleColumn, onInsertFilter }: Props) {
  const fields = useMemo(() => extractFields(source), [source]);
  if (!source) {
    return (
      <div className="text-xs text-muted-foreground p-2">
        Select a source to see its fields.
      </div>
    );
  }
  if (fields.length === 0) {
    return (
      <div className="text-xs text-muted-foreground p-2">
        {source.progress === "done" ? "No rows in this source." : "Loading…"}
      </div>
    );
  }
  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1 uppercase tracking-wide">
        Fields in {source.source}
      </div>
      {fields.map((f) => {
        const isCol = columns.includes(f);
        return (
          <div
            key={f}
            className="flex items-center gap-1 px-2 py-1 rounded text-xs hover:bg-accent group"
          >
            <button
              type="button"
              className={`truncate flex-1 text-left font-mono ${isCol ? "text-primary" : ""}`}
              title={isCol ? `remove ${f} column from the stream` : `show ${f} as a column in the stream`}
              onClick={() => onToggleColumn(f)}
            >
              {f}
            </button>
            {isCol && <Columns3 className="size-3 text-primary" />}
            <Button
              variant="ghost"
              size="sm"
              className="opacity-0 group-hover:opacity-100 h-5 px-1"
              onClick={() => onInsertFilter(`${f}:*`)}
              aria-label={`filter to rows where ${f} exists`}
              title={`add ${f}:* to the query`}
            >
              <Filter className="size-3" />
            </Button>
          </div>
        );
      })}
    </div>
  );
}

function extractFields(s: SourceState | null): string[] {
  if (!s) return [];
  const seen = new Set<string>();
  for (const row of s.rows.slice(0, 50)) {
    for (const k of Object.keys(row)) seen.add(k);
  }
  return Array.from(seen).sort();
}
```

- [ ] **Step 2: Typecheck + commit** (same temporary-call-site rule as Task 7)

```bash
git add web/src/features/discover/FieldsRail.tsx web/src/features/discover/DiscoverPage.tsx
git commit -m "feat(discover): fields rail toggles columns and inserts filters"
```

---

### Task 9: StreamView, SourceTableView, SummaryLine + DiscoverPage rewrite

**Files:**
- Create: `web/src/features/discover/StreamView.tsx`
- Create: `web/src/features/discover/SourceTableView.tsx`
- Create: `web/src/features/discover/SummaryLine.tsx`
- Test: `web/src/features/discover/SummaryLine.test.ts` (formatter only)
- Modify: `web/src/features/discover/DiscoverPage.tsx` (rewrite)
- Delete: `web/src/features/discover/SearchBar.tsx`, `web/src/features/discover/FilterPills.tsx`, `web/src/features/discover/SourceSection.tsx`

- [ ] **Step 1: Failing test for the summary formatter**

```ts
import { expect, test } from "vitest";
import { formatSummary } from "./SummaryLine";

test("formatSummary states sources, window, count and as-of", () => {
  const s = formatSummary({
    sourcesSearched: 4,
    windowLabel: "last 1h",
    eventCount: 1284,
    finishedAt: Date.parse("2026-06-05T12:04:31Z"),
    status: "done",
  });
  expect(s).toMatch(/4 sources/);
  expect(s).toMatch(/last 1h/);
  expect(s).toMatch(/1,284 events/);
  expect(s).toMatch(/as of/);
});

test("formatSummary while running says searching", () => {
  const s = formatSummary({ sourcesSearched: 2, windowLabel: "last 1h", eventCount: 10, finishedAt: null, status: "running" });
  expect(s).toMatch(/searching/i);
});
```

Run: `cd web && npx vitest run src/features/discover/SummaryLine.test.ts` — Expected: FAIL (module not found).

- [ ] **Step 2: `SummaryLine.tsx`**

```tsx
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
```

Run the test again — Expected: PASS.

- [ ] **Step 3: `StreamView.tsx`** — merged rows with ts, source chip, summary; explicit columns when toggled; cap initial render.

```tsx
import { useMemo, useState } from "react";
import { partitionColumns, formatCell } from "./columns";
import { pickMessageField, summarizeRow } from "./rowSummary";
import type { StreamRow } from "./stream";
import type { SourceState, SourceKey } from "./types";

const PAGE = 200;
const CHIP_COLORS = [
  "border-blue-500/50", "border-emerald-500/50", "border-amber-500/50", "border-red-500/50",
  "border-violet-500/50", "border-pink-500/50", "border-teal-500/50", "border-orange-500/50",
];

type Props = {
  rows: StreamRow[];
  sources: Map<SourceKey, SourceState>;
  columns: string[];
  onOpenRow: (source: SourceKey, row: Record<string, unknown>) => void;
};

export function StreamView({ rows, sources, columns, onOpenRow }: Props) {
  const [limit, setLimit] = useState(PAGE);

  // Per-source presentation hints, computed once per result set.
  const hints = useMemo(() => {
    const m = new Map<SourceKey, { message: string | null; hiddenVectors: string[]; tsCol: string | null }>();
    for (const [key, s] of sources) {
      const { hiddenVectors } = partitionColumns(s.rows);
      m.set(key, { message: pickMessageField(s.rows), hiddenVectors, tsCol: s.timestampColumn });
    }
    return m;
  }, [sources]);

  const chipColor = useMemo(() => {
    const m = new Map<SourceKey, string>();
    Array.from(sources.keys()).forEach((k, i) => m.set(k, CHIP_COLORS[i % CHIP_COLORS.length]));
    return m;
  }, [sources]);

  const fmtTs = (ts: number | null) =>
    ts == null ? "—" : new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

  return (
    <div className="text-xs font-mono">
      {rows.slice(0, limit).map((r, i) => {
        const h = hints.get(r.source);
        const sum = summarizeRow(r.row, h?.message ?? null, h?.tsCol ?? null, [
          ...(h?.hiddenVectors ?? []),
          ...columns,
        ]);
        return (
          <div
            key={i}
            className="flex items-start gap-2 px-3 py-1.5 border-b border-border/50 hover:bg-accent cursor-pointer"
            onClick={() => onOpenRow(r.source, r.row)}
          >
            <span className="text-muted-foreground tabular-nums shrink-0 w-[8ch]">{fmtTs(r.ts)}</span>
            <span
              className={`shrink-0 rounded border px-1 text-[10px] text-muted-foreground ${chipColor.get(r.source) ?? ""}`}
              title={r.source}
            >
              {r.source.split(".")[1] ?? r.source}
            </span>
            {columns.map((c) => (
              <span key={c} className="shrink-0 max-w-[24ch] truncate" title={`${c}=${formatCell(r.row[c])}`}>
                {formatCell(r.row[c])}
              </span>
            ))}
            <span className="min-w-0 truncate">
              {sum.primary && <span>{sum.primary}</span>}
              {sum.rest.length > 0 && (
                <span className="text-muted-foreground">
                  {sum.primary ? "  " : ""}
                  {sum.rest.map(([k, v]) => `${k}=${v}`).join(" ")}
                </span>
              )}
            </span>
          </div>
        );
      })}
      {rows.length > limit && (
        <button
          type="button"
          className="w-full p-2 text-center text-muted-foreground hover:bg-accent"
          onClick={() => setLimit((l) => l + PAGE)}
        >
          show {Math.min(PAGE, rows.length - limit)} more of {rows.length.toLocaleString()} loaded
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 4: `SourceTableView.tsx`** — plain table for one source (the not-in-timeline click target). Reuses the phase-0 column curation.

```tsx
import { useMemo } from "react";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { partitionColumns, formatCell } from "./columns";
import type { SourceState } from "./types";

type Props = {
  src: SourceState;
  onBack: () => void;
  onOpenRow: (row: Record<string, unknown>) => void;
};

export function SourceTableView({ src, onBack, onOpenRow }: Props) {
  const { shown: cols, hiddenVectors } = useMemo(() => partitionColumns(src.rows), [src]);
  return (
    <div>
      <div className="flex items-center gap-2 px-3 py-2 border-b">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="size-4 mr-1" /> Back to stream
        </Button>
        <span className="font-mono text-sm">{src.source}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {src.total.toLocaleString()} rows · not in timeline (no timestamp column)
        </span>
      </div>
      {src.rows.length === 0 ? (
        <div className="p-3 text-sm text-muted-foreground">
          {src.progress === "done" ? "no rows" : "loading…"}
        </div>
      ) : (
        <table className="w-full text-xs font-mono">
          <thead className="sticky top-0 bg-background z-10">
            <tr>
              {cols.map((c) => (
                <th key={c} className="text-left px-2 py-1 border-b font-medium">{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {src.rows.slice(0, 500).map((r, i) => (
              <tr key={i} className="hover:bg-accent cursor-pointer" onClick={() => onOpenRow(r)}>
                {cols.map((c) => (
                  <td key={c} className="px-2 py-1 truncate max-w-xs align-top" title={formatCell(r[c])}>
                    {formatCell(r[c])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {hiddenVectors.length > 0 && (
        <div className="p-2 text-xs text-muted-foreground">
          {hiddenVectors.length} vector column{hiddenVectors.length > 1 ? "s" : ""} hidden — open a row to see {hiddenVectors.length > 1 ? "them" : "it"}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Rewrite `DiscoverPage.tsx`** — compose everything; RowDetailDrawer's `onAddPill` becomes text insertion.

```tsx
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { useWorkspace } from "../tabs/workspace-store";
import type { TimeRange } from "../tabs/workspace-store";
import { TimeRangePicker } from "@/features/time-range/TimeRangePicker";
import { ScopePicker } from "./ScopePicker";
import { QueryBar } from "./QueryBar";
import { SourcesRail } from "./SourcesRail";
import { FieldsRail } from "./FieldsRail";
import { Histogram } from "./Histogram";
import { StreamView } from "./StreamView";
import { SourceTableView } from "./SourceTableView";
import { SummaryLine } from "./SummaryLine";
import { RowDetailDrawer } from "./RowDetailDrawer";
import { SavedViewsMenu } from "./SavedViewsMenu";
import { useDiscoverSearch } from "./useDiscoverSearch";
import { parseSearch, serializePills } from "./discoverGrammar";
import { compileToKql } from "./compileToKql";
import { mergeSources } from "./stream";
import type { Pill } from "./types";

type Props = { tabId: string };

const PRESET_MINUTES: Record<string, number> = {
  "5m": 5, "15m": 15, "1h": 60, "6h": 360,
  "24h": 1440, "7d": 10080, "30d": 43200,
};
const PRESET_LABEL: Record<string, string> = {
  "5m": "last 5m", "15m": "last 15m", "1h": "last 1h", "6h": "last 6h",
  "24h": "last 24h", "7d": "last 7d", "30d": "last 30d", custom: "custom range",
};

function resolveTimeRange(t: TimeRange): { from: string; to: string } | null {
  if (t.preset === "custom" && t.from && t.to) return { from: t.from, to: t.to };
  const minutes = PRESET_MINUTES[t.preset];
  if (minutes == null) return null;
  const now = Date.now();
  return {
    from: new Date(now - minutes * 60_000).toISOString(),
    to: new Date(now).toISOString(),
  };
}

export function DiscoverPage({ tabId }: Props) {
  const tab = useWorkspace((s) => s.tabs.find((t) => t.id === tabId));
  const patchDiscover = useWorkspace((s) => s.patchDiscover);
  const newTab = useWorkspace((s) => s.newTab);

  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);
  // The search submitted to the backend — typing edits `search` freely; Run
  // (or auto-run on scope/time change) snapshots it here.
  const isDiscover = tab?.kind === "discover";
  const st = isDiscover ? tab.state : null;
  const [submitted, setSubmitted] = useState(() => st?.search ?? "");

  const { results, cancel } = useDiscoverSearch({
    search: submitted,
    scope: st?.scope ?? { kind: "all" },
    timeRange: st?.timeRange ?? { preset: "1h" },
    enabled: Boolean(isDiscover),
  });

  if (!tab || tab.kind !== "discover" || !st) return null;

  const selected = st.selectedSource ? results.sources.get(st.selectedSource) ?? null : null;
  const streamRows = mergeSources(Array.from(results.sources.values()), st.visibleSources);
  const tableSrc =
    typeof st.viewMode === "object" ? results.sources.get(st.viewMode.table) ?? null : null;

  const insertFilter = (text: string) => {
    const next = st.search.trim() ? `${st.search.trim()} ${text}` : text;
    patchDiscover(tabId, { search: next });
  };

  const pillToText = (p: Pill) => serializePills([p]);

  const openInQueryEditor = () => {
    const sources = Array.from(results.sources.keys());
    const tr = resolveTimeRange(st.timeRange);
    let pills: Pill[] = [];
    try {
      pills = parseSearch(st.search);
    } catch {
      // Grammar errors are surfaced in the QueryBar; export without filters.
    }
    const kql = compileToKql(sources, pills, tr);
    newTab({
      kind: "query",
      state: {
        title: "from discover",
        query: kql,
        timeRange: st.timeRange,
        results: { kind: "idle" },
        chart: {},
        submittedQuery: null,
      },
    });
  };

  return (
    <div className="flex flex-col h-full">
      {/* Top bar */}
      <div className="flex items-center gap-2 p-3 border-b">
        <ScopePicker value={st.scope} onChange={(scope) => patchDiscover(tabId, { scope })} />
        <QueryBar
          value={st.search}
          onChange={(v) => patchDiscover(tabId, { search: v })}
          onRun={() => setSubmitted(st.search)}
          onCancel={cancel}
          running={results.status === "running"}
        />
        <TimeRangePicker value={st.timeRange} onChange={(tr) => patchDiscover(tabId, { timeRange: tr })} />
        <SavedViewsMenu currentScope={st.scope} />
        <Button variant="ghost" size="sm" onClick={openInQueryEditor}>
          Open in Query Editor
        </Button>
      </div>

      <SummaryLine
        sourcesSearched={results.sources.size}
        windowLabel={PRESET_LABEL[st.timeRange.preset] ?? "all time"}
        eventCount={streamRows.length}
        finishedAt={results.finishedAt ?? null}
        status={results.status}
      />

      <div className="flex flex-1 min-h-0">
        <aside className="w-60 border-r overflow-auto">
          <SourcesRail
            results={results}
            visible={st.visibleSources}
            onToggleVisible={(src) => {
              const cur = st.visibleSources ?? Array.from(results.sources.keys());
              const next = cur.includes(src) ? cur.filter((s) => s !== src) : [...cur, src];
              patchDiscover(tabId, { visibleSources: next });
            }}
            selected={st.selectedSource}
            onSelect={(src) => patchDiscover(tabId, { selectedSource: src })}
            onOpenTable={(src) => patchDiscover(tabId, { viewMode: { table: src } })}
          />
          <FieldsRail
            source={selected}
            columns={st.columns}
            onToggleColumn={(f) =>
              patchDiscover(tabId, {
                columns: st.columns.includes(f)
                  ? st.columns.filter((c) => c !== f)
                  : [...st.columns, f],
              })
            }
            onInsertFilter={insertFilter}
          />
        </aside>

        <main className="flex-1 overflow-auto">
          {tableSrc ? (
            <SourceTableView
              src={tableSrc}
              onBack={() => patchDiscover(tabId, { viewMode: "stream" })}
              onOpenRow={(row) => setOpenRow({ source: tableSrc.source, row })}
            />
          ) : (
            <>
              <Histogram
                results={results}
                onZoom={(from, to) =>
                  patchDiscover(tabId, { timeRange: { preset: "custom", from, to } })
                }
              />
              {results.topError && (
                <div className="p-3 m-3 border border-destructive rounded text-sm text-destructive">
                  <span className="font-semibold">{results.topError.code}</span>: {results.topError.message}
                </div>
              )}
              <StreamView
                rows={streamRows}
                sources={results.sources}
                columns={st.columns}
                onOpenRow={(source, row) => setOpenRow({ source, row })}
              />
              {results.status === "done" && results.sources.size === 0 && (
                <div className="p-6 text-center text-sm text-muted-foreground space-y-2">
                  <div>No data sources match this scope.</div>
                  {st.scope.kind === "all" ? (
                    <div>
                      Internal sources (agent memory) are hidden by default.{" "}
                      <button
                        type="button"
                        className="underline underline-offset-2 hover:text-foreground"
                        onClick={() =>
                          patchDiscover(tabId, { scope: { kind: "sources", sources: ["memory.*"] } })
                        }
                      >
                        Search internal sources
                      </button>
                    </div>
                  ) : (
                    <div>Try widening with the Scope picker.</div>
                  )}
                </div>
              )}
            </>
          )}
        </main>
      </div>

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={(p) => { insertFilter(pillToText(p)); setOpenRow(null); }}
      />
    </div>
  );
}
```

Note `useDiscoverSearch` arg change (already partly done in Task 4): its `Args` becomes `{ search: string; scope; timeRange; perSourceLimit?; enabled }`, request body uses `query: args.search`, and the `argsKey` JSON uses `search` instead of `pills`.

- [ ] **Step 6: Delete dead files**

```bash
git rm web/src/features/discover/SearchBar.tsx web/src/features/discover/FilterPills.tsx web/src/features/discover/SourceSection.tsx
```
(SourceSection's per-source tables are replaced by StreamView + SourceTableView; its time-filter badge semantics move to the rail's "Not in timeline" group, which is strictly clearer.)

- [ ] **Step 7: Full check**

Run: `cd web && npx tsc --noEmit -p tsconfig.json && npx vitest run src/features`
Expected: PASS, no references to deleted files.

- [ ] **Step 8: Commit**

```bash
git add -A web/src/features/discover web/src/features/tabs
git commit -m "feat(discover): unified time-sorted stream with summary line and table view"
```

---

### Task 10: Manual verification + spec/plan commit

- [ ] **Step 1:** With `pensieve serve` on :8080 and vite on :5173 running, load `http://localhost:5173/discover`. Verify: summary line states sources/window/count; stream is empty-state (this machine only has memory tables) with the "Search internal sources" link; clicking it shows `memory.*` under **Not in timeline** in the rail; clicking a memory source opens the plain table; back returns to stream. Run a `memory.*` scope search, click a row → drawer opens; drawer `=` button inserts `field:value` text into the query bar (visible, editable).
- [ ] **Step 2:** Ingest a few timestamped test rows (`curl -X POST http://127.0.0.1:8080/v1/ingest -H "Authorization: Bearer <token>" -H "X-Database: demo" -H "X-Table: events" -d '{"timestamp":"<now>","message":"hello","service":"api"}'` ×5 with varied values) and verify the stream renders them merged + histogram labels + brush zoom sets a custom range.
- [ ] **Step 3:** Commit the spec + plan docs:

```bash
git add docs/superpowers/specs/2026-06-05-discover-live-stream-design.md docs/superpowers/plans/2026-06-05-discover-unified-stream-phase1.md
git commit -m "docs: discover unified live stream spec + phase-1 plan"
```

---

## Self-review notes

- **Spec coverage:** summary line (T9), persistent bar (T5/T9), unified stream (T2/T9), no-timestamp handling (T7/T9 table view), labeled histogram + brush (T6), fields-rail columns + filter insertion (T8), pills removal + migration (T4), vector hygiene reused from phase 0 (T3/T9). Live (§2–3 of spec) is Phase 2 — separate plan after engine investigation.
- **Type consistency:** `timestampColumn: string | null` introduced in T1 is what T2 (`mergeSources`), T7 (rail grouping), and T9 consume. `viewMode: "stream" | { table: SourceKey }` defined in T4, consumed in T9.
- **Known judgment calls:** stream re-sorts per update instead of incremental insert (fine at ≤ a few k rows; Phase 2 revisits); `submitted` query snapshot lives in component state, not the tab (re-running on tab switch is acceptable; revisit if annoying).
