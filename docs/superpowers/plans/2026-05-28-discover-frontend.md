# Discover — Frontend Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans.

**Goal:** Ship the Kibana-Discover-style frontend page that consumes `POST /v1/explore/search`. Users land on `/discover`, get a search bar + filter pills + time histogram + grouped-by-source results without ever seeing KQL. KQL editor lives behind an "Open in Query Editor" action that lands in the renamed `/query` page (today's Explore).

**Architecture:** New route `_app.discover.tsx` becomes the default landing (`/` → `/discover` redirect). New feature dir `web/src/features/discover/` houses all components, store, hook, grammar mirror, and SDK adapter. Workspace store gains a discriminated tab kind (`discover` | `query`). Today's `_app.explore.tsx` is renamed `_app.query.tsx` and serves as the escape hatch.

**Tech Stack:** React 18 + TypeScript, TanStack Router, Zustand (with persist), TanStack Query, shadcn/ui + Tailwind, Monaco (only in Query tab — Discover has no editor), Playwright for E2E.

**Reference spec:** `docs/superpowers/specs/2026-05-28-explore-discover-refactor-design.md` (sections 3, 5, 6, 7, 8.4, 8.6, 8.7).

**Prereqs:** Plan A (`2026-05-28-discover-engine-search.md`) must be merged. Plan B (saved views) is optional for the page to function — the SavedViewsMenu shows an empty list until B ships.

---

## File Structure

| File                                                          | Action  | Responsibility                                              |
|---------------------------------------------------------------|---------|-------------------------------------------------------------|
| `web/src/sdk/discover.ts`                                     | Create  | Typed client + NDJSON streaming parser                      |
| `web/src/sdk/discover.test.ts`                                | Create  | Vitest for the parser                                       |
| `web/src/features/discover/types.ts`                          | Create  | Shared types (Pill, Scope, Frame, SourceState, etc.)        |
| `web/src/features/discover/discoverGrammar.ts`                | Create  | Parse `string ↔ Pill[]` (mirrors engine grammar)            |
| `web/src/features/discover/discoverGrammar.test.ts`           | Create  | Vitest                                                      |
| `web/src/features/discover/discover-store.ts`                 | Create  | Zustand slice for per-discover-tab state                    |
| `web/src/features/discover/useDiscoverSearch.ts`              | Create  | Hook: wraps SDK call + frame reducer                        |
| `web/src/features/discover/ScopePicker.tsx`                   | Create  | "All sources" / pick / view popover                         |
| `web/src/features/discover/SearchBar.tsx`                     | Create  | Single text input + Run button + AI button                  |
| `web/src/features/discover/FilterPills.tsx`                   | Create  | Dismissable pill row + Add Pill popover                     |
| `web/src/features/discover/SourcesRail.tsx`                   | Create  | Left rail: visible-source checkboxes + counts               |
| `web/src/features/discover/FieldsRail.tsx`                    | Create  | Left rail: fields for selected source + click-to-filter     |
| `web/src/features/discover/Histogram.tsx`                     | Create  | Time histogram (stacked-by-source bars)                     |
| `web/src/features/discover/SourceSection.tsx`                 | Create  | Per-source accordion: header, virtualized table, errors     |
| `web/src/features/discover/RowDetailDrawer.tsx`               | Create  | Right-side drawer with full row JSON + Filter-by actions    |
| `web/src/features/discover/SavedViewsMenu.tsx`                | Create  | Save current scope as view / pick view / delete view        |
| `web/src/features/discover/DiscoverPage.tsx`                  | Create  | Top-level assembly                                          |
| `web/src/features/discover/compileToKql.ts`                   | Create  | Compile current Discover state → KQL union (Open in Query Editor) |
| `web/src/routes/_app.discover.tsx`                            | Create  | Route file mounting `DiscoverPage`                          |
| `web/src/routes/_app.query.tsx`                               | Create  | Renamed `_app.explore.tsx`                                  |
| `web/src/routes/_app.explore.tsx`                             | Delete  | Replaced by `_app.query.tsx`                                |
| `web/src/router.tsx` (or wherever routes are wired)           | Modify  | `/` → `/discover` redirect; `/explore` → `/query` redirect   |
| `web/src/features/tabs/workspace-store.ts`                    | Modify  | Discriminated `Tab` kind + persisted-state migration         |
| `web/src/features/tabs/workspace-store.test.ts`               | Modify  | Cover the migration                                          |
| `web/src/features/tabs/TabBar.tsx`                            | Modify  | Show distinct icons per tab kind                             |
| `web/src/features/agent/AskAIDialog.tsx` (if exists)          | Modify  | Accept Discover patch outputs                                |
| `web/tests/e2e/discover.spec.ts`                              | Create  | Playwright golden path                                       |

---

## Phase 1 — SDK + grammar (no UI yet)

### Task 1: Typed client + NDJSON streaming parser

**Files:** `web/src/sdk/discover.ts`, `web/src/sdk/discover.test.ts`

- [ ] **Step 1: Write failing parser test**

```ts
// web/src/sdk/discover.test.ts
import { describe, it, expect } from "vitest";
import { parseNdjsonStream } from "./discover";

function streamFrom(chunks: string[]): ReadableStream<Uint8Array> {
  const enc = new TextEncoder();
  return new ReadableStream({
    start(c) {
      for (const chunk of chunks) c.enqueue(enc.encode(chunk));
      c.close();
    },
  });
}

describe("parseNdjsonStream", () => {
  it("emits frames split across chunk boundaries", async () => {
    const s = streamFrom([
      '{"type":"plan","sources":[]}\n{"type":"sourc',
      'e_progress","source":"db.t","state":"running"}\n',
      '{"type":"done","elapsed_ms":12}\n',
    ]);
    const out: any[] = [];
    for await (const f of parseNdjsonStream(s)) out.push(f);
    expect(out.map(f => f.type)).toEqual(["plan", "source_progress", "done"]);
  });

  it("yields no frames for empty input", async () => {
    const s = streamFrom([""]);
    const out: any[] = [];
    for await (const f of parseNdjsonStream(s)) out.push(f);
    expect(out).toHaveLength(0);
  });

  it("throws on malformed JSON", async () => {
    const s = streamFrom(['{"type":"plan"\n']);
    let threw = false;
    try { for await (const _ of parseNdjsonStream(s)) {} } catch { threw = true; }
    expect(threw).toBe(true);
  });
});
```

- [ ] **Step 2: Implement the SDK**

```ts
// web/src/sdk/discover.ts
import { authFetch } from "./auth-fetch";

export type Frame =
  | { type: "plan"; sources: { source: string; has_timestamp: boolean }[] }
  | { type: "source_progress"; source: string; state: "running" }
  | { type: "rows"; source: string; rows: Record<string, unknown>[] }
  | { type: "histogram"; source: string; buckets: { t: string; n: number }[] }
  | { type: "source_done"; source: string; total: number; capped: boolean; dropped_clauses: any[] }
  | { type: "error"; source?: string; code: string; message: string }
  | { type: "done"; elapsed_ms: number };

export type Scope =
  | { kind: "all" }
  | { kind: "sources"; sources: string[] }
  | { kind: "view"; view_id: string };

export type SearchRequest = {
  query: string;
  scope: Scope;
  time_range?: { from: string; to: string } | null;
  per_source_limit?: number;
  histogram?: { interval_ms: number };
};

export async function* searchDiscover(
  req: SearchRequest,
  signal?: AbortSignal,
): AsyncGenerator<Frame, void, unknown> {
  const resp = await authFetch("/v1/explore/search", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(req),
    signal,
  });
  if (!resp.ok) {
    let detail: any = null;
    try { detail = await resp.json(); } catch {}
    throw new DiscoverError(
      detail?.error?.code ?? `http_${resp.status}`,
      detail?.error?.message ?? resp.statusText,
      resp.status,
    );
  }
  if (!resp.body) throw new DiscoverError("no_body", "empty response body", resp.status);
  yield* parseNdjsonStream(resp.body);
}

export class DiscoverError extends Error {
  constructor(public code: string, message: string, public status?: number) {
    super(message);
  }
}

export async function* parseNdjsonStream(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<Frame, void, unknown> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  while (true) {
    const { value, done } = await reader.read();
    if (value) buf += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      yield JSON.parse(line) as Frame;
    }
    if (done) break;
  }
  const tail = buf.trim();
  if (tail) yield JSON.parse(tail) as Frame;
}

export type SavedView = {
  id: string;
  name: string;
  sources: string[];
  columns: unknown | null;
  created_at: string;
  updated_at: string;
};

export async function listSavedViews(): Promise<SavedView[]> {
  const r = await authFetch("/v1/explore/views");
  if (!r.ok) throw new DiscoverError("list_failed", await r.text(), r.status);
  return r.json();
}

export async function createSavedView(name: string, sources: string[]): Promise<SavedView> {
  const r = await authFetch("/v1/explore/views", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name, sources }),
  });
  if (!r.ok) throw new DiscoverError("create_failed", await r.text(), r.status);
  return r.json();
}

export async function deleteSavedView(id: string): Promise<void> {
  const r = await authFetch(`/v1/explore/views/${id}`, { method: "DELETE" });
  if (!r.ok && r.status !== 204) throw new DiscoverError("delete_failed", await r.text(), r.status);
}
```

- [ ] **Step 3: Run tests**

```bash
cd web && pnpm vitest run src/sdk/discover.test.ts
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add web/src/sdk/discover.ts web/src/sdk/discover.test.ts
git commit -m "feat(web/sdk): typed Discover client + NDJSON streaming parser"
```

---

### Task 2: Grammar mirror

**Files:** `web/src/features/discover/types.ts`, `web/src/features/discover/discoverGrammar.ts`, `web/src/features/discover/discoverGrammar.test.ts`

- [ ] **Step 1: Shared types**

```ts
// web/src/features/discover/types.ts
import type { Scope, Frame } from "../../sdk/discover";

export type Pill =
  | { kind: "substring"; value: string }
  | { kind: "eq"; field: string; value: string }
  | { kind: "neq"; field: string; value: string }
  | { kind: "cmp"; field: string; op: "gt" | "ge" | "lt" | "le"; value: string }
  | { kind: "exists"; field: string };

export type SourceKey = string; // "db.table"

export type SourceState = {
  source: SourceKey;
  hasTimestamp: boolean;
  progress: "pending" | "running" | "done" | "error";
  rows: Record<string, unknown>[];
  total: number;
  capped: boolean;
  droppedClauses: any[];
  histogram?: { t: string; n: number }[];
  error?: { code: string; message: string };
};

export type DiscoverResultsState = {
  status: "idle" | "running" | "done" | "error";
  sources: Map<SourceKey, SourceState>;
  startedAt?: number;
  finishedAt?: number;
  topError?: { code: string; message: string };
};

export type { Scope, Frame };
```

- [ ] **Step 2: Grammar test suite**

```ts
// web/src/features/discover/discoverGrammar.test.ts
import { describe, it, expect } from "vitest";
import { parseSearch, serializePills } from "./discoverGrammar";

describe("parseSearch", () => {
  it("parses bare substring", () => {
    expect(parseSearch("auth")).toEqual([{ kind: "substring", value: "auth" }]);
  });
  it("parses quoted phrase", () => {
    expect(parseSearch('"foo bar"')).toEqual([{ kind: "substring", value: "foo bar" }]);
  });
  it("parses eq / neq", () => {
    expect(parseSearch("svc:pay -level:INFO")).toEqual([
      { kind: "eq", field: "svc", value: "pay" },
      { kind: "neq", field: "level", value: "INFO" },
    ]);
  });
  it("parses numeric cmp", () => {
    expect(parseSearch("status:>500 lat:<=10")).toEqual([
      { kind: "cmp", field: "status", op: "gt", value: "500" },
      { kind: "cmp", field: "lat", op: "le", value: "10" },
    ]);
  });
  it("parses exists", () => {
    expect(parseSearch("trace_id:*")).toEqual([{ kind: "exists", field: "trace_id" }]);
  });
  it("round-trips via serializePills", () => {
    const inputs = [
      "auth svc:pay -level:INFO status:>500 trace_id:*",
      '"foo bar" baz',
    ];
    for (const s of inputs) {
      const pills = parseSearch(s);
      const reparsed = parseSearch(serializePills(pills));
      expect(reparsed).toEqual(pills);
    }
  });
  it("throws on unterminated quote", () => {
    expect(() => parseSearch('svc:"foo')).toThrow();
  });
});
```

- [ ] **Step 3: Implement parseSearch / serializePills**

```ts
// web/src/features/discover/discoverGrammar.ts
import type { Pill } from "./types";

export function parseSearch(input: string): Pill[] {
  const out: Pill[] = [];
  let i = 0;
  while (i < input.length) {
    while (i < input.length && /\s/.test(input[i])) i++;
    if (i >= input.length) break;
    const negated = input[i] === "-";
    if (negated) i++;
    let raw = "";
    while (i < input.length && !/\s/.test(input[i])) {
      if (input[i] === '"') {
        i++;
        let closed = false;
        while (i < input.length) {
          if (input[i] === '"') { closed = true; i++; break; }
          raw += input[i++];
        }
        if (!closed) throw new Error("unterminated quoted string");
        continue;
      }
      raw += input[i++];
    }
    if (!raw) continue;
    out.push(tokenToPill(raw, negated));
  }
  return out;
}

function tokenToPill(token: string, negated: boolean): Pill {
  const c = token.indexOf(":");
  if (c <= 0) return { kind: "substring", value: token };
  const field = token.slice(0, c);
  const value = token.slice(c + 1);
  if (value === "*") return { kind: "exists", field };
  if (value.startsWith(">=")) return { kind: "cmp", field, op: "ge", value: value.slice(2) };
  if (value.startsWith("<=")) return { kind: "cmp", field, op: "le", value: value.slice(2) };
  if (value.startsWith(">"))  return { kind: "cmp", field, op: "gt", value: value.slice(1) };
  if (value.startsWith("<"))  return { kind: "cmp", field, op: "lt", value: value.slice(1) };
  return negated
    ? { kind: "neq", field, value }
    : { kind: "eq", field, value };
}

export function serializePills(pills: Pill[]): string {
  return pills.map(pillToToken).join(" ");
}

function pillToToken(p: Pill): string {
  switch (p.kind) {
    case "substring": return needsQuotes(p.value) ? `"${p.value}"` : p.value;
    case "eq":  return `${p.field}:${quoteIfNeeded(p.value)}`;
    case "neq": return `-${p.field}:${quoteIfNeeded(p.value)}`;
    case "exists": return `${p.field}:*`;
    case "cmp":
      const op = { gt: ">", ge: ">=", lt: "<", le: "<=" }[p.op];
      return `${p.field}:${op}${p.value}`;
  }
}

function needsQuotes(s: string) { return /\s/.test(s); }
function quoteIfNeeded(s: string) { return needsQuotes(s) ? `"${s}"` : s; }
```

- [ ] **Step 4: Tests pass**

```bash
cd web && pnpm vitest run src/features/discover/discoverGrammar.test.ts
```

Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add web/src/features/discover/types.ts \
        web/src/features/discover/discoverGrammar.ts \
        web/src/features/discover/discoverGrammar.test.ts
git commit -m "feat(discover): grammar parser mirrors engine"
```

---

## Phase 2 — State + hook

### Task 3: Discover Zustand slice (per-tab state)

**Files:** `web/src/features/discover/discover-store.ts`

- [ ] **Step 1: Write the slice**

```ts
// web/src/features/discover/discover-store.ts
//
// Per-tab Discover state. Held inside the workspace store's discriminated
// `discover` tab kind (see Task 13). This module is the *shape* + a couple of
// pure helpers so the workspace store can hold/persist it.

import type { Scope, Pill, DiscoverResultsState, SourceState, Frame, SourceKey } from "./types";
import type { TimeRange } from "../tabs/workspace-store";

export type DiscoverTabState = {
  scope: Scope;
  search: string;
  pills: Pill[];
  timeRange: TimeRange;
  visibleSources: SourceKey[] | null; // null = all in plan
  selectedSource: SourceKey | null;   // drives the Fields rail
  results: DiscoverResultsState;
};

export const initialDiscoverTabState = (): DiscoverTabState => ({
  scope: { kind: "all" },
  search: "",
  pills: [],
  timeRange: { preset: "1h" },
  visibleSources: null,
  selectedSource: null,
  results: { status: "idle", sources: new Map() },
});

export function applyFrame(state: DiscoverResultsState, frame: Frame): DiscoverResultsState {
  const next: DiscoverResultsState = {
    ...state,
    sources: new Map(state.sources),
  };
  switch (frame.type) {
    case "plan": {
      next.status = "running";
      next.startedAt = next.startedAt ?? Date.now();
      next.sources = new Map(
        frame.sources.map((s): [SourceKey, SourceState] => [
          s.source,
          {
            source: s.source,
            hasTimestamp: s.has_timestamp,
            progress: "pending",
            rows: [],
            total: 0,
            capped: false,
            droppedClauses: [],
          },
        ]),
      );
      return next;
    }
    case "source_progress": {
      const s = next.sources.get(frame.source);
      if (s) next.sources.set(frame.source, { ...s, progress: "running" });
      return next;
    }
    case "rows": {
      const s = next.sources.get(frame.source);
      if (s) next.sources.set(frame.source, { ...s, rows: [...s.rows, ...frame.rows] });
      return next;
    }
    case "histogram": {
      const s = next.sources.get(frame.source);
      if (s) next.sources.set(frame.source, { ...s, histogram: frame.buckets });
      return next;
    }
    case "source_done": {
      const s = next.sources.get(frame.source);
      if (s) next.sources.set(frame.source, {
        ...s,
        progress: "done",
        total: frame.total,
        capped: frame.capped,
        droppedClauses: frame.dropped_clauses,
      });
      return next;
    }
    case "error": {
      if (frame.source) {
        const s = next.sources.get(frame.source);
        if (s) next.sources.set(frame.source, {
          ...s,
          progress: "error",
          error: { code: frame.code, message: frame.message },
        });
      } else {
        next.topError = { code: frame.code, message: frame.message };
      }
      return next;
    }
    case "done": {
      next.status = next.topError ? "error" : "done";
      next.finishedAt = Date.now();
      return next;
    }
  }
}
```

- [ ] **Step 2: Write a focused reducer test**

```ts
// web/src/features/discover/discover-store.test.ts
import { describe, it, expect } from "vitest";
import { applyFrame } from "./discover-store";
import type { DiscoverResultsState } from "./types";

const empty: DiscoverResultsState = { status: "idle", sources: new Map() };

describe("applyFrame", () => {
  it("plan creates a pending entry per source", () => {
    const s = applyFrame(empty, {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: true }],
    });
    expect(s.status).toBe("running");
    expect(s.sources.get("a.b")?.progress).toBe("pending");
  });
  it("rows appends rows for a source", () => {
    let s = applyFrame(empty, { type: "plan", sources: [{ source: "a.b", has_timestamp: false }] });
    s = applyFrame(s, { type: "rows", source: "a.b", rows: [{ x: 1 }] });
    s = applyFrame(s, { type: "rows", source: "a.b", rows: [{ x: 2 }] });
    expect(s.sources.get("a.b")?.rows).toEqual([{ x: 1 }, { x: 2 }]);
  });
  it("source_done captures totals + capped", () => {
    let s = applyFrame(empty, { type: "plan", sources: [{ source: "a.b", has_timestamp: false }] });
    s = applyFrame(s, { type: "source_done", source: "a.b", total: 100, capped: true, dropped_clauses: [] });
    expect(s.sources.get("a.b")?.total).toBe(100);
    expect(s.sources.get("a.b")?.capped).toBe(true);
  });
  it("error frame without source becomes topError", () => {
    const s = applyFrame(empty, { type: "error", code: "x", message: "y" });
    expect(s.topError).toEqual({ code: "x", message: "y" });
  });
  it("done with a topError ends in error state", () => {
    let s = applyFrame(empty, { type: "error", code: "x", message: "y" });
    s = applyFrame(s, { type: "done", elapsed_ms: 1 });
    expect(s.status).toBe("error");
  });
});
```

- [ ] **Step 3: Tests pass**

```bash
cd web && pnpm vitest run src/features/discover/discover-store.test.ts
```

Expected: 5 passed.

- [ ] **Step 4: Commit**

```bash
git add web/src/features/discover/discover-store.ts web/src/features/discover/discover-store.test.ts
git commit -m "feat(discover): per-tab state shape + frame reducer"
```

---

### Task 4: useDiscoverSearch hook

**Files:** `web/src/features/discover/useDiscoverSearch.ts`

- [ ] **Step 1: Write the hook**

```ts
// web/src/features/discover/useDiscoverSearch.ts
import { useCallback, useEffect, useRef, useState } from "react";
import { searchDiscover, type SearchRequest } from "../../sdk/discover";
import { applyFrame } from "./discover-store";
import { serializePills } from "./discoverGrammar";
import type { DiscoverResultsState, Pill, Scope } from "./types";
import type { TimeRange } from "../tabs/workspace-store";

type Args = {
  scope: Scope;
  pills: Pill[];
  timeRange: TimeRange;
  perSourceLimit?: number;
  enabled: boolean;
};

export function useDiscoverSearch(args: Args, onResults: (s: DiscoverResultsState) => void) {
  const [results, setResults] = useState<DiscoverResultsState>({ status: "idle", sources: new Map() });
  const abortRef = useRef<AbortController | null>(null);

  const run = useCallback(async () => {
    abortRef.current?.abort();
    const ac = new AbortController();
    abortRef.current = ac;

    let acc: DiscoverResultsState = { status: "running", sources: new Map(), startedAt: Date.now() };
    setResults(acc);
    onResults(acc);

    const req: SearchRequest = {
      query: serializePills(args.pills) + (args.pills.length ? " " : "") + "", // pills carry the structured part; bar text is in pills already
      scope: args.scope,
      time_range: resolveTimeRange(args.timeRange),
      per_source_limit: args.perSourceLimit ?? 500,
    };
    // If pills weren't pre-merged with bar text, the page should pass the merged
    // Pill[] into args.pills. Page enforces this — see DiscoverPage Task 12.

    try {
      for await (const frame of searchDiscover(req, ac.signal)) {
        acc = applyFrame(acc, frame);
        setResults(acc);
        onResults(acc);
      }
    } catch (e: any) {
      if (e.name === "AbortError") return;
      acc = { ...acc, status: "error", topError: { code: e.code ?? "fetch_error", message: e.message } };
      setResults(acc);
      onResults(acc);
    }
  }, [JSON.stringify(args)]);

  useEffect(() => {
    if (args.enabled) void run();
    return () => abortRef.current?.abort();
  }, [args.enabled, run]);

  return { results, rerun: run, cancel: () => abortRef.current?.abort() };
}

function resolveTimeRange(t: TimeRange): { from: string; to: string } | null {
  // Re-use the existing preset → ISO helper. If you don't have one, this is the
  // minimal version:
  const now = Date.now();
  const minutes = ({ "5m": 5, "15m": 15, "1h": 60, "6h": 360, "24h": 1440, "7d": 10080, "30d": 43200 } as Record<string, number>)[t.preset];
  if (t.preset === "custom" && t.from && t.to) return { from: t.from, to: t.to };
  if (minutes == null) return null;
  return {
    from: new Date(now - minutes * 60_000).toISOString(),
    to: new Date(now).toISOString(),
  };
}
```

If the codebase already has a `timeRangeToIso(t: TimeRange)` helper in `web/src/features/time-range/`, import and use it instead of the inline `resolveTimeRange`. Confirm via:

```bash
grep -rn 'preset' web/src/features/time-range/ | head
```

- [ ] **Step 2: Compile-check**

```bash
cd web && pnpm tsc --noEmit
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add web/src/features/discover/useDiscoverSearch.ts
git commit -m "feat(discover): streaming search hook with frame reducer"
```

---

## Phase 3 — UI components

The following tasks build small components in isolation. Each one is its own task with TDD via Vitest + React Testing Library where logic is non-trivial; pure-presentation components don't need unit tests but do need to compile.

### Task 5: ScopePicker

**Files:** `web/src/features/discover/ScopePicker.tsx`

- [ ] **Step 1: Write the component**

```tsx
// web/src/features/discover/ScopePicker.tsx
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { listSavedViews, type SavedView } from "../../sdk/discover";
import type { Scope } from "./types";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";

type Props = { value: Scope; onChange: (s: Scope) => void };

export function ScopePicker({ value, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const views = useQuery({ queryKey: ["saved-views"], queryFn: listSavedViews });

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm">{label(value, views.data)}</Button>
      </PopoverTrigger>
      <PopoverContent className="w-80 space-y-3">
        <button className="block w-full text-left text-sm hover:bg-accent rounded px-2 py-1"
          onClick={() => { onChange({ kind: "all" }); setOpen(false); }}>
          All sources
        </button>
        <div className="border-t pt-2">
          <div className="text-xs text-muted-foreground mb-1">Saved views</div>
          {(views.data ?? []).map(v => (
            <button key={v.id}
              className="block w-full text-left text-sm hover:bg-accent rounded px-2 py-1"
              onClick={() => { onChange({ kind: "view", view_id: v.id }); setOpen(false); }}>
              {v.name}
            </button>
          ))}
          {(views.data ?? []).length === 0 && (
            <div className="text-xs text-muted-foreground italic">No saved views yet.</div>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function label(s: Scope, views?: SavedView[]) {
  if (s.kind === "all") return "All sources";
  if (s.kind === "sources") return `${s.sources.length} sources`;
  const v = views?.find(v => v.id === s.view_id);
  return v ? `View: ${v.name}` : "Saved view";
}
```

- [ ] **Step 2: Compile-check & commit**

```bash
cd web && pnpm tsc --noEmit
git add web/src/features/discover/ScopePicker.tsx
git commit -m "feat(discover): ScopePicker (All / saved views)"
```

(A v2 task can add "Pick db/tables" via the existing schema browser. v1 ships with "All" and saved-views only — explicit picking is doable via saved views.)

---

### Task 6: SearchBar + FilterPills

**Files:** `web/src/features/discover/SearchBar.tsx`, `web/src/features/discover/FilterPills.tsx`

- [ ] **Step 1: SearchBar**

```tsx
// web/src/features/discover/SearchBar.tsx
import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Loader2, Play, X } from "lucide-react";

type Props = {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  onCancel: () => void;
  running: boolean;
};

export function SearchBar({ value, onChange, onSubmit, onCancel, running }: Props) {
  const [draft, setDraft] = useState(value);
  return (
    <div className="flex gap-2 items-center">
      <Input
        placeholder="Search… e.g. auth service:payments -severity:INFO"
        value={draft}
        onChange={e => setDraft(e.target.value)}
        onKeyDown={e => {
          if (e.key === "Enter") { onChange(draft); onSubmit(); }
        }}
        className="font-mono text-sm"
      />
      {running ? (
        <Button variant="destructive" size="sm" onClick={onCancel}>
          <X className="size-4 mr-1" /> Cancel
        </Button>
      ) : (
        <Button size="sm" onClick={() => { onChange(draft); onSubmit(); }}>
          <Play className="size-4 mr-1" /> Run
        </Button>
      )}
      {running && <Loader2 className="size-4 animate-spin text-muted-foreground" />}
    </div>
  );
}
```

- [ ] **Step 2: FilterPills**

```tsx
// web/src/features/discover/FilterPills.tsx
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { X, Plus } from "lucide-react";
import { serializePills } from "./discoverGrammar";
import type { Pill } from "./types";

type Props = {
  pills: Pill[];
  onRemove: (idx: number) => void;
  onAdd?: () => void; // hook for "Add Pill" popover (v2)
};

export function FilterPills({ pills, onRemove, onAdd }: Props) {
  if (pills.length === 0 && !onAdd) return null;
  return (
    <div className="flex flex-wrap gap-1 items-center">
      {pills.map((p, i) => (
        <Badge key={i} variant="secondary" className="font-mono text-xs gap-1">
          {serializePills([p])}
          <button onClick={() => onRemove(i)} aria-label="remove pill">
            <X className="size-3" />
          </button>
        </Badge>
      ))}
      {onAdd && (
        <Button variant="ghost" size="sm" onClick={onAdd}>
          <Plus className="size-3 mr-1" /> Filter
        </Button>
      )}
    </div>
  );
}
```

- [ ] **Step 3: Compile + commit**

```bash
cd web && pnpm tsc --noEmit
git add web/src/features/discover/SearchBar.tsx web/src/features/discover/FilterPills.tsx
git commit -m "feat(discover): SearchBar + FilterPills"
```

---

### Task 7: SourcesRail

**Files:** `web/src/features/discover/SourcesRail.tsx`

- [ ] **Step 1: Write the component**

```tsx
// web/src/features/discover/SourcesRail.tsx
import { Checkbox } from "@/components/ui/checkbox";
import { Loader2, AlertCircle, CheckCircle2 } from "lucide-react";
import type { DiscoverResultsState, SourceKey } from "./types";

type Props = {
  results: DiscoverResultsState;
  visible: SourceKey[] | null;
  onToggleVisible: (s: SourceKey) => void;
  selected: SourceKey | null;
  onSelect: (s: SourceKey) => void;
};

export function SourcesRail({ results, visible, onToggleVisible, selected, onSelect }: Props) {
  const sources = Array.from(results.sources.values());
  if (sources.length === 0 && results.status === "idle") {
    return <div className="text-xs text-muted-foreground p-2">Run a search.</div>;
  }
  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1">Sources</div>
      {sources.map(s => {
        const isVisible = visible == null || visible.includes(s.source);
        const isSelected = selected === s.source;
        return (
          <div key={s.source}
            className={`flex items-center gap-2 px-2 py-1 rounded text-sm cursor-pointer hover:bg-accent ${isSelected ? "bg-accent" : ""}`}
            onClick={() => onSelect(s.source)}>
            <Checkbox checked={isVisible} onCheckedChange={() => onToggleVisible(s.source)} onClick={e => e.stopPropagation()} />
            <span className="truncate flex-1" title={s.source}>{s.source}</span>
            {s.progress === "running" && <Loader2 className="size-3 animate-spin text-muted-foreground" />}
            {s.progress === "done" && <CheckCircle2 className="size-3 text-muted-foreground" />}
            {s.progress === "error" && <AlertCircle className="size-3 text-destructive" />}
            <span className="text-xs text-muted-foreground tabular-nums">{s.total.toLocaleString()}</span>
          </div>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/features/discover/SourcesRail.tsx
git commit -m "feat(discover): SourcesRail with per-source status + counts"
```

---

### Task 8: FieldsRail

**Files:** `web/src/features/discover/FieldsRail.tsx`

- [ ] **Step 1: Write the component**

```tsx
// web/src/features/discover/FieldsRail.tsx
import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import type { SourceState, Pill } from "./types";

type Props = {
  source: SourceState | null;
  onAddPill: (p: Pill) => void;
};

export function FieldsRail({ source, onAddPill }: Props) {
  const fields = useMemo(() => extractFields(source), [source]);
  if (!source) {
    return <div className="text-xs text-muted-foreground p-2">Select a source.</div>;
  }
  if (fields.length === 0) {
    return <div className="text-xs text-muted-foreground p-2">No rows yet.</div>;
  }
  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1">
        Fields in {source.source}
      </div>
      {fields.map(f => (
        <div key={f} className="flex items-center gap-2 px-2 py-1 rounded text-xs hover:bg-accent group">
          <span className="truncate flex-1" title={f}>{f}</span>
          <Button
            variant="ghost" size="sm"
            className="opacity-0 group-hover:opacity-100 h-5 px-1 text-[10px]"
            onClick={() => onAddPill({ kind: "exists", field: f })}>
            +exists
          </Button>
        </div>
      ))}
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

(v2 will add the top-N value breakdown popover — that's the existing `field-stats` component, which can be wrapped in a follow-up. v1 ships the basic field list with click-to-filter via `+exists`.)

- [ ] **Step 2: Commit**

```bash
git add web/src/features/discover/FieldsRail.tsx
git commit -m "feat(discover): FieldsRail (basic field list + add-exists pill)"
```

---

### Task 9: Histogram

**Files:** `web/src/features/discover/Histogram.tsx`

- [ ] **Step 1: Write the component**

```tsx
// web/src/features/discover/Histogram.tsx
import { useMemo } from "react";
import type { DiscoverResultsState } from "./types";

const PALETTE = [
  "#3b82f6","#10b981","#f59e0b","#ef4444","#8b5cf6","#ec4899","#14b8a6","#f97316",
];

type Props = { results: DiscoverResultsState };

export function Histogram({ results }: Props) {
  const data = useMemo(() => stack(results), [results]);
  if (data.bars.length === 0) return null;

  const max = data.bars.reduce((m, b) => Math.max(m, b.total), 0) || 1;
  return (
    <div className="border-b">
      <div className="flex items-end h-24 px-2 gap-px">
        {data.bars.map((b, i) => (
          <div key={i} className="flex-1 flex flex-col-reverse" title={`${b.label} — ${b.total}`}>
            {b.segments.map(seg => (
              <div key={seg.source}
                style={{ height: `${(seg.n / max) * 100}%`, backgroundColor: PALETTE[seg.colorIdx % PALETTE.length] }} />
            ))}
          </div>
        ))}
      </div>
      <div className="flex gap-3 px-2 py-1 text-[10px] text-muted-foreground">
        {Array.from(results.sources.keys()).map((src, i) => (
          <span key={src} className="inline-flex items-center gap-1">
            <span className="inline-block size-2 rounded-sm" style={{ backgroundColor: PALETTE[i % PALETTE.length] }} />
            {src}
          </span>
        ))}
      </div>
    </div>
  );
}

function stack(r: DiscoverResultsState) {
  // Bucket key = ISO timestamp; segments = per-source counts.
  const bucketIndex = new Map<string, Map<string, number>>();
  const sourceKeys = Array.from(r.sources.keys());
  for (const [src, st] of r.sources) {
    if (!st.histogram) continue;
    for (const b of st.histogram) {
      const m = bucketIndex.get(b.t) ?? new Map<string, number>();
      m.set(src, (m.get(src) ?? 0) + b.n);
      bucketIndex.set(b.t, m);
    }
  }
  const ordered = Array.from(bucketIndex.entries()).sort(([a],[b]) => a.localeCompare(b));
  const bars = ordered.map(([t, m]) => {
    const segs = sourceKeys.map((src, i) => ({ source: src, n: m.get(src) ?? 0, colorIdx: i }));
    const total = segs.reduce((s, v) => s + v.n, 0);
    return { label: t, total, segments: segs };
  });
  return { bars };
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/features/discover/Histogram.tsx
git commit -m "feat(discover): time histogram (stacked by source)"
```

---

### Task 10: SourceSection + RowDetailDrawer

**Files:** `web/src/features/discover/SourceSection.tsx`, `web/src/features/discover/RowDetailDrawer.tsx`

- [ ] **Step 1: SourceSection**

```tsx
// web/src/features/discover/SourceSection.tsx
import { useMemo, useState } from "react";
import { ChevronRight, ChevronDown, AlertCircle } from "lucide-react";
import type { SourceState, Pill } from "./types";

type Props = {
  src: SourceState;
  onOpenRow: (row: Record<string, unknown>) => void;
  onAddPill: (p: Pill) => void;
};

export function SourceSection({ src, onOpenRow, onAddPill }: Props) {
  const [open, setOpen] = useState(true);
  const cols = useMemo(() => columnsFor(src), [src]);

  return (
    <div className="border-b">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 w-full text-left px-3 py-2 hover:bg-accent">
        {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        <span className="font-medium">{src.source}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {src.total.toLocaleString()} hits{src.capped ? " (capped)" : ""}
        </span>
        {src.error && (
          <span className="ml-auto inline-flex items-center gap-1 text-xs text-destructive">
            <AlertCircle className="size-3" /> {src.error.code}
          </span>
        )}
      </button>
      {open && (
        <div className="overflow-auto max-h-96">
          {src.error ? (
            <div className="p-3 text-sm text-destructive">{src.error.message}</div>
          ) : src.rows.length === 0 ? (
            <div className="p-3 text-sm text-muted-foreground">
              {src.progress === "done" ? "no rows" : "loading…"}
            </div>
          ) : (
            <table className="w-full text-xs font-mono">
              <thead className="sticky top-0 bg-background">
                <tr>{cols.map(c => <th key={c} className="text-left px-2 py-1 border-b">{c}</th>)}</tr>
              </thead>
              <tbody>
                {src.rows.slice(0, 200).map((r, i) => (
                  <tr key={i} className="hover:bg-accent cursor-pointer" onClick={() => onOpenRow(r)}>
                    {cols.map(c => (
                      <td key={c} className="px-2 py-1 truncate max-w-xs"
                        title={String(r[c] ?? "")}>{format(r[c])}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {src.rows.length > 200 && (
            <div className="p-2 text-xs text-muted-foreground">
              showing first 200 of {src.rows.length} loaded rows
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function columnsFor(src: SourceState): string[] {
  const seen = new Set<string>();
  for (const r of src.rows.slice(0, 50)) for (const k of Object.keys(r)) seen.add(k);
  const arr = Array.from(seen);
  // Stable order: timestamp first, then alpha.
  arr.sort((a, b) => (a === "timestamp" ? -1 : b === "timestamp" ? 1 : a.localeCompare(b)));
  return arr;
}

function format(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}
```

(v2 swaps the table for `react-virtual` when row count > 500. v1 caps display at 200 rows per source which is fine for the per_source_limit=500 default and avoids the virtualization complexity.)

- [ ] **Step 2: RowDetailDrawer**

```tsx
// web/src/features/discover/RowDetailDrawer.tsx
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import type { Pill, SourceKey } from "./types";

type Props = {
  source: SourceKey | null;
  row: Record<string, unknown> | null;
  onClose: () => void;
  onAddPill: (p: Pill) => void;
};

export function RowDetailDrawer({ source, row, onClose, onAddPill }: Props) {
  const open = !!row;
  return (
    <Sheet open={open} onOpenChange={o => !o && onClose()}>
      <SheetContent className="w-[480px] sm:max-w-[480px] overflow-auto">
        <SheetHeader>
          <SheetTitle className="font-mono text-sm">{source}</SheetTitle>
        </SheetHeader>
        {row && (
          <div className="mt-4 space-y-1 font-mono text-xs">
            {Object.entries(row).map(([k, v]) => (
              <div key={k} className="grid grid-cols-[140px_1fr] gap-2 group">
                <div className="text-muted-foreground truncate" title={k}>{k}</div>
                <div className="flex items-start gap-1">
                  <span className="break-all flex-1">{format(v)}</span>
                  {scalarish(v) && (
                    <>
                      <Button variant="ghost" size="sm" className="h-5 px-1 text-[10px] opacity-0 group-hover:opacity-100"
                        onClick={() => onAddPill({ kind: "eq", field: k, value: String(v) })}>
                        =
                      </Button>
                      <Button variant="ghost" size="sm" className="h-5 px-1 text-[10px] opacity-0 group-hover:opacity-100"
                        onClick={() => onAddPill({ kind: "neq", field: k, value: String(v) })}>
                        ≠
                      </Button>
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}

function format(v: unknown): string {
  if (v == null) return "null";
  if (typeof v === "object") return JSON.stringify(v, null, 2);
  return String(v);
}
function scalarish(v: unknown): boolean {
  return v != null && (typeof v === "string" || typeof v === "number" || typeof v === "boolean");
}
```

- [ ] **Step 3: Commit**

```bash
git add web/src/features/discover/SourceSection.tsx web/src/features/discover/RowDetailDrawer.tsx
git commit -m "feat(discover): SourceSection + RowDetailDrawer"
```

---

### Task 11: SavedViewsMenu

**Files:** `web/src/features/discover/SavedViewsMenu.tsx`

- [ ] **Step 1: Write the component**

```tsx
// web/src/features/discover/SavedViewsMenu.tsx
import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { createSavedView } from "../../sdk/discover";
import type { Scope } from "./types";

type Props = { currentScope: Scope; onSaved?: (id: string) => void };

export function SavedViewsMenu({ currentScope, onSaved }: Props) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const qc = useQueryClient();
  const m = useMutation({
    mutationFn: async () => {
      if (currentScope.kind !== "sources") {
        throw new Error("only explicit source scopes can be saved as views in v1");
      }
      return createSavedView(name, currentScope.sources);
    },
    onSuccess: v => {
      qc.invalidateQueries({ queryKey: ["saved-views"] });
      setOpen(false);
      setName("");
      onSaved?.(v.id);
    },
  });

  const canSave = currentScope.kind === "sources";

  return (
    <>
      <Button variant="outline" size="sm" disabled={!canSave} onClick={() => setOpen(true)}>
        Save as view…
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Save scope as view</DialogTitle>
          </DialogHeader>
          <Input value={name} onChange={e => setName(e.target.value)} placeholder="e.g. prod-logs" />
          {m.error && <div className="text-sm text-destructive">{(m.error as Error).message}</div>}
          <DialogFooter>
            <Button onClick={() => m.mutate()} disabled={!name.trim() || m.isPending}>
              {m.isPending ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/features/discover/SavedViewsMenu.tsx
git commit -m "feat(discover): SavedViewsMenu (save current scope)"
```

---

## Phase 4 — Page assembly + workspace store

### Task 12: DiscoverPage assembly + compileToKql

**Files:** `web/src/features/discover/DiscoverPage.tsx`, `web/src/features/discover/compileToKql.ts`

- [ ] **Step 1: compileToKql**

```ts
// web/src/features/discover/compileToKql.ts
//
// Compile the current Discover state to a KQL string that can be pasted into
// the Query Editor. Multi-source → union of per-source pipes. Single source →
// a single pipe. Mirrors the engine's per-source compilation rules but does
// NOT attempt to type-check; the editor user can clean up dropped clauses.

import type { Pill, SourceKey } from "./types";

export function compileToKql(sources: SourceKey[], pills: Pill[], timeRange?: { from: string; to: string } | null): string {
  if (sources.length === 0) return "";
  const pipes = sources.map(s => onePipe(s, pills, timeRange));
  if (pipes.length === 1) return pipes[0];
  return "union\n  " + pipes.join(",\n  ");
}

function onePipe(source: SourceKey, pills: Pill[], tr?: { from: string; to: string } | null): string {
  const [, table] = source.split(".", 2);
  const tableExpr = table ?? source;
  const clauses = pills.map(pillToKql).filter(Boolean) as string[];
  if (tr) clauses.push(`timestamp >= datetime("${tr.from}") and timestamp < datetime("${tr.to}")`);
  let out = tableExpr;
  for (const c of clauses) out += ` | where ${c}`;
  out += " | take 500";
  return out;
}

function pillToKql(p: Pill): string | null {
  const esc = (s: string) => `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  switch (p.kind) {
    case "substring": return `* contains ${esc(p.value)}`;
    case "eq":  return `${p.field} == ${esc(p.value)}`;
    case "neq": return `${p.field} != ${esc(p.value)}`;
    case "exists": return `isnotnull(${p.field})`;
    case "cmp":
      const op = { gt: ">", ge: ">=", lt: "<", le: "<=" }[p.op];
      return `${p.field} ${op} ${p.value}`;
  }
}
```

- [ ] **Step 2: DiscoverPage**

```tsx
// web/src/features/discover/DiscoverPage.tsx
import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { TimeRangePicker } from "../time-range/TimeRangePicker"; // adjust path to whatever exists
import { ScopePicker } from "./ScopePicker";
import { SearchBar } from "./SearchBar";
import { FilterPills } from "./FilterPills";
import { SourcesRail } from "./SourcesRail";
import { FieldsRail } from "./FieldsRail";
import { Histogram } from "./Histogram";
import { SourceSection } from "./SourceSection";
import { RowDetailDrawer } from "./RowDetailDrawer";
import { SavedViewsMenu } from "./SavedViewsMenu";
import { useDiscoverSearch } from "./useDiscoverSearch";
import { parseSearch, serializePills } from "./discoverGrammar";
import { compileToKql } from "./compileToKql";
import { useWorkspace } from "../tabs/workspace-store";

type Props = { tabId: string };

export function DiscoverPage({ tabId }: Props) {
  const tab = useWorkspace(s => s.tabs.find(t => t.id === tabId));
  const updateDiscover = useWorkspace(s => s.updateDiscover);
  const newQueryTab = useWorkspace(s => s.newTab);

  if (!tab || tab.kind !== "discover") return null;
  const st = tab.state;

  const [openRow, setOpenRow] = useState<{ source: string; row: Record<string, unknown> } | null>(null);

  const allPills = st.pills;
  const { results, cancel } = useDiscoverSearch({
    scope: st.scope,
    pills: allPills,
    timeRange: st.timeRange,
    enabled: true,
  }, () => {});

  const selected = st.selectedSource ? results.sources.get(st.selectedSource) ?? null : null;

  const submit = () => {
    const merged = mergePills(st.search, st.pills);
    updateDiscover(tabId, { ...st, search: "", pills: merged });
  };

  const removePill = (idx: number) => {
    const next = st.pills.filter((_, i) => i !== idx);
    updateDiscover(tabId, { ...st, pills: next });
  };

  const addPill = (p: Pill) => {
    updateDiscover(tabId, { ...st, pills: [...st.pills, p] });
  };

  const openInQueryEditor = () => {
    const sources = Array.from(results.sources.keys());
    const tr = resolveTimeRangeForKql(st.timeRange);
    const kql = compileToKql(sources, st.pills, tr);
    newQueryTab({ kind: "query", state: {
      title: `from Discover`, query: kql, timeRange: st.timeRange,
      results: { kind: "idle" }, chart: {}, submittedQuery: null,
    }});
  };

  return (
    <div className="flex flex-col h-full">
      {/* Top bar */}
      <div className="flex flex-col gap-2 p-3 border-b">
        <div className="flex items-center gap-2">
          <ScopePicker value={st.scope} onChange={scope => updateDiscover(tabId, { ...st, scope })} />
          <SearchBar
            value={st.search}
            onChange={v => updateDiscover(tabId, { ...st, search: v })}
            onSubmit={submit}
            onCancel={cancel}
            running={results.status === "running"}
          />
          <TimeRangePicker value={st.timeRange} onChange={tr => updateDiscover(tabId, { ...st, timeRange: tr })} />
          <SavedViewsMenu currentScope={st.scope} />
          <Button variant="ghost" size="sm" onClick={openInQueryEditor}>Open in Query Editor</Button>
        </div>
        <FilterPills pills={st.pills} onRemove={removePill} />
      </div>

      <div className="flex flex-1 min-h-0">
        <aside className="w-60 border-r overflow-auto">
          <SourcesRail
            results={results}
            visible={st.visibleSources}
            onToggleVisible={src => {
              const cur = st.visibleSources ?? Array.from(results.sources.keys());
              const next = cur.includes(src) ? cur.filter(s => s !== src) : [...cur, src];
              updateDiscover(tabId, { ...st, visibleSources: next });
            }}
            selected={st.selectedSource}
            onSelect={src => updateDiscover(tabId, { ...st, selectedSource: src })}
          />
          <FieldsRail source={selected} onAddPill={addPill} />
        </aside>

        <main className="flex-1 overflow-auto">
          <Histogram results={results} />
          {results.topError && (
            <div className="p-3 m-3 border border-destructive rounded text-sm text-destructive">
              {results.topError.code}: {results.topError.message}
            </div>
          )}
          {Array.from(results.sources.values())
            .filter(s => st.visibleSources == null || st.visibleSources.includes(s.source))
            .map(s => (
              <SourceSection key={s.source} src={s}
                onOpenRow={row => setOpenRow({ source: s.source, row })}
                onAddPill={addPill} />
            ))}
          {results.status === "done" && results.sources.size === 0 && (
            <div className="p-6 text-center text-sm text-muted-foreground">
              No sources match this scope. Try widening with the Scope picker.
            </div>
          )}
        </main>
      </div>

      <RowDetailDrawer
        source={openRow?.source ?? null}
        row={openRow?.row ?? null}
        onClose={() => setOpenRow(null)}
        onAddPill={p => { addPill(p); setOpenRow(null); }}
      />
    </div>
  );
}

function mergePills(barText: string, existing: Pill[]): Pill[] {
  if (!barText.trim()) return existing;
  const fresh = parseSearch(barText);
  return [...existing, ...fresh];
}

function resolveTimeRangeForKql(t: any) {
  // Re-use the same helper as useDiscoverSearch (consider extracting both into a shared util on cleanup).
  const now = Date.now();
  const minutes = ({ "5m": 5, "15m": 15, "1h": 60, "6h": 360, "24h": 1440, "7d": 10080, "30d": 43200 } as Record<string, number>)[t.preset];
  if (t.preset === "custom" && t.from && t.to) return { from: t.from, to: t.to };
  if (minutes == null) return null;
  return { from: new Date(now - minutes * 60_000).toISOString(), to: new Date(now).toISOString() };
}
```

- [ ] **Step 3: Compile-check**

```bash
cd web && pnpm tsc --noEmit
```

(Will surface type errors against the not-yet-migrated workspace store — that's expected. Fix them in Task 13 before merging this task.)

- [ ] **Step 4: Commit (gated on Task 13 building)**

This task's commit is bundled with Task 13. Continue to Task 13.

---

### Task 13: Workspace store discriminated tab kind + migration

**Files:** `web/src/features/tabs/workspace-store.ts`, `web/src/features/tabs/workspace-store.test.ts`, `web/src/features/tabs/TabBar.tsx`

- [ ] **Step 1: Rewrite the Tab type as discriminated**

Read the current file:

```bash
cat web/src/features/tabs/workspace-store.ts
```

Apply this shape:

```ts
// web/src/features/tabs/workspace-store.ts
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { DiscoverTabState } from "../discover/discover-store";

export type TimeRangePreset = "5m" | "15m" | "1h" | "6h" | "24h" | "7d" | "30d" | "custom";
export type TimeRange = { preset: TimeRangePreset; from?: string; to?: string };

export type ResultsState =
  | { kind: "idle" }
  | { kind: "running"; startedAt: number }
  | { kind: "ok"; rowCount: number; durationMs: number; finishedAt: number }
  | { kind: "error"; message: string; requestId?: string; code?: string };

export type ChartConfig = {
  override?: { type: "line" | "bar" | "scatter" | "stat"; x?: string; y?: string };
};

export type QueryTabState = {
  title: string;
  query: string;
  timeRange: TimeRange;
  results: ResultsState;
  chart: ChartConfig;
  submittedQuery: string | null;
};

export type Tab =
  | { id: string; kind: "query"; state: QueryTabState }
  | { id: string; kind: "discover"; state: DiscoverTabState };

export function isQueryTabDirty(t: Extract<Tab, { kind: "query" }>): boolean {
  return t.state.submittedQuery !== null && t.state.submittedQuery !== t.state.query;
}

type Store = {
  tabs: Tab[];
  activeId: string | null;
  newTab: (seed?: Partial<Tab>) => string;
  closeTab: (id: string) => void;
  setActive: (id: string) => void;
  updateQuery: (id: string, patch: Partial<QueryTabState>) => void;
  updateDiscover: (id: string, next: DiscoverTabState) => void;
  resetAll: () => void;
};

const DEFAULT_RANGE: TimeRange = { preset: "1h" };

function defaultDiscoverState(): DiscoverTabState {
  // Re-export from discover-store so this file stays free of feature imports.
  // The actual default is imported lazily to avoid a circular import surface;
  // in practice tsc handles type-only imports fine.
  return {
    scope: { kind: "all" }, search: "", pills: [],
    timeRange: DEFAULT_RANGE, visibleSources: null, selectedSource: null,
    results: { status: "idle", sources: new Map() },
  };
}

export const useWorkspace = create<Store>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeId: null,
      newTab: (seed) => {
        const id = crypto.randomUUID();
        const tab: Tab = seed?.kind === "query"
          ? { id, kind: "query", state: { title: "query", query: "", timeRange: DEFAULT_RANGE,
              results: { kind: "idle" }, chart: {}, submittedQuery: null, ...((seed as any)?.state ?? {}) } }
          : { id, kind: "discover", state: { ...defaultDiscoverState(), ...((seed as any)?.state ?? {}) } };
        set({ tabs: [...get().tabs, tab], activeId: id });
        return id;
      },
      closeTab: (id) => {
        const tabs = get().tabs.filter(t => t.id !== id);
        const active = get().activeId === id ? (tabs[0]?.id ?? null) : get().activeId;
        set({ tabs, activeId: active });
      },
      setActive: (id) => set({ activeId: id }),
      updateQuery: (id, patch) => set({
        tabs: get().tabs.map(t =>
          t.id === id && t.kind === "query"
            ? { ...t, state: { ...t.state, ...patch } }
            : t),
      }),
      updateDiscover: (id, next) => set({
        tabs: get().tabs.map(t =>
          t.id === id && t.kind === "discover"
            ? { ...t, state: next }
            : t),
      }),
      resetAll: () => set({ tabs: [], activeId: null }),
    }),
    {
      name: "pensieve-workspace",
      version: 2,
      migrate: (persisted: any, fromVersion) => {
        if (fromVersion < 2 && persisted?.state?.tabs) {
          // v1 tabs were flat. Migrate every old tab to { kind: "query" }.
          persisted.state.tabs = persisted.state.tabs.map((old: any) => ({
            id: old.id,
            kind: "query",
            state: {
              title: old.title,
              query: old.query ?? "",
              timeRange: old.timeRange ?? DEFAULT_RANGE,
              results: old.results ?? { kind: "idle" },
              chart: old.chart ?? {},
              submittedQuery: old.submittedQuery ?? null,
            },
          }));
        }
        return persisted;
      },
      // Map<>s are not JSON-serializable. Persist Discover results.sources as an
      // array tuple and rehydrate as a Map. The "results" field is transient
      // anyway — strip it on persist.
      partialize: (s) => ({
        tabs: s.tabs.map(t => t.kind === "discover"
          ? { ...t, state: { ...t.state, results: undefined } }
          : t),
        activeId: s.activeId,
      }),
      onRehydrateStorage: () => (s) => {
        if (!s) return;
        s.tabs = s.tabs.map(t => t.kind === "discover"
          ? { ...t, state: { ...t.state, results: { status: "idle", sources: new Map() } } }
          : t);
      },
    },
  ),
);
```

- [ ] **Step 2: Migration test**

```ts
// web/src/features/tabs/workspace-store.test.ts (add to existing file)
import { describe, it, expect } from "vitest";

describe("workspace-store v1→v2 migration", () => {
  it("converts old flat tab to query kind", async () => {
    const v1 = {
      state: {
        tabs: [{ id: "x", title: "T", query: "select 1", timeRange: { preset: "1h" }, results: { kind: "idle" }, chart: {}, submittedQuery: null }],
        activeId: "x",
      },
      version: 1,
    };
    localStorage.setItem("pensieve-workspace", JSON.stringify(v1));
    // Force a fresh import so the persist middleware re-runs migration.
    const mod = await import("./workspace-store?bust=" + Date.now());
    const tabs = mod.useWorkspace.getState().tabs;
    expect(tabs[0].kind).toBe("query");
    expect((tabs[0] as any).state.query).toBe("select 1");
  });
});
```

(If the `?bust=` import trick is too brittle in your test setup, the alternative is to reset by clearing localStorage and creating a fresh store instance — adapt to whatever pattern the codebase already uses for store tests.)

- [ ] **Step 3: TabBar — distinct icons**

```tsx
// web/src/features/tabs/TabBar.tsx (relevant edit)
// Inside the per-tab render block, add:
import { Search, Code } from "lucide-react";

// Then in JSX:
{tab.kind === "discover" ? <Search className="size-3" /> : <Code className="size-3" />}
```

(Other styling untouched.)

- [ ] **Step 4: Update callers**

Search & fix:

```bash
grep -rn 'setQuery\|setTimeRange\|setResults\|setChart\|markSubmitted\|isTabDirty\|tab.query\|tab.title\|tab.results\b' web/src --include='*.ts' --include='*.tsx'
```

For each match, refactor to the new shape:
- `setQuery(id, q)` → `updateQuery(id, { query: q })`
- `setTimeRange(id, r)` → `updateQuery(id, { timeRange: r })`
- `setResults(id, r)` → `updateQuery(id, { results: r })`
- `setChart(id, c)` → `updateQuery(id, { chart: c })`
- `markSubmitted(id, q)` → `updateQuery(id, { submittedQuery: q })`
- `isTabDirty(tab)` → `tab.kind === "query" && isQueryTabDirty(tab)`
- `tab.query`/`tab.title`/`tab.results`/etc. → `tab.state.query` etc., guarded by `tab.kind === "query"` discriminant

This is mechanical; expect to touch 8–15 call sites.

- [ ] **Step 5: TSC + tests**

```bash
cd web && pnpm tsc --noEmit
cd web && pnpm vitest run src/features/tabs/workspace-store.test.ts
```

Expected: tsc clean, 1+ test passes (existing + new migration).

- [ ] **Step 6: Commit bundled with Task 12**

```bash
git add web/src/features/tabs/workspace-store.ts \
        web/src/features/tabs/workspace-store.test.ts \
        web/src/features/tabs/TabBar.tsx \
        web/src/features/discover/DiscoverPage.tsx \
        web/src/features/discover/compileToKql.ts \
        # plus every file changed in Step 4
git commit -m "feat(discover): DiscoverPage + discriminated workspace store with v1→v2 migration"
```

---

## Phase 5 — Routes + cutover

### Task 14: Routes + redirects

**Files:** `web/src/routes/_app.discover.tsx`, `web/src/routes/_app.query.tsx`, `web/src/routes/_app.explore.tsx` (delete), router config

- [ ] **Step 1: Create the Discover route**

```tsx
// web/src/routes/_app.discover.tsx
import { createFileRoute } from "@tanstack/react-router";
import { useEffect } from "react";
import { useWorkspace } from "../features/tabs/workspace-store";
import { DiscoverPage } from "../features/discover/DiscoverPage";

export const Route = createFileRoute("/_app/discover")({
  component: DiscoverRoute,
});

function DiscoverRoute() {
  const tabs = useWorkspace(s => s.tabs);
  const activeId = useWorkspace(s => s.activeId);
  const newTab = useWorkspace(s => s.newTab);

  useEffect(() => {
    const active = tabs.find(t => t.id === activeId);
    if (!active || active.kind !== "discover") {
      const firstDiscover = tabs.find(t => t.kind === "discover");
      if (firstDiscover) {
        useWorkspace.getState().setActive(firstDiscover.id);
      } else {
        newTab({ kind: "discover" });
      }
    }
  }, [tabs, activeId]);

  const active = tabs.find(t => t.id === activeId);
  if (!active || active.kind !== "discover") return null;
  return <DiscoverPage tabId={active.id} />;
}
```

- [ ] **Step 2: Rename old Explore route → Query**

```bash
git mv web/src/routes/_app.explore.tsx web/src/routes/_app.query.tsx
```

Edit the renamed file:
- Replace `createFileRoute("/_app/explore")` with `createFileRoute("/_app/query")`.
- Within the component, when calling `newTab()`/reading `active`, branch on `active.kind === "query"` and (if missing) call `newTab({ kind: "query" })`.

- [ ] **Step 3: Redirects**

In the root route (`web/src/routes/__root.tsx` or wherever the index redirect lives — check `grep -rn 'redirect\|createRootRoute\|/_app' web/src/routes/`):

```ts
// _app.index.tsx or similar
import { createFileRoute, redirect } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/")({
  beforeLoad: () => { throw redirect({ to: "/discover" }); },
});
```

And add `/explore` → `/query`:

```ts
// _app.explore.tsx (recreate as a redirect-only stub)
import { createFileRoute, redirect } from "@tanstack/react-router";
export const Route = createFileRoute("/_app/explore")({
  beforeLoad: () => { throw redirect({ to: "/query" }); },
});
```

(If TanStack Router auto-generates route trees, run the generator after these changes: `pnpm tsr generate` or whatever script the codebase uses — search `grep -n 'tsr\|generate' web/package.json`.)

- [ ] **Step 4: Sidebar / nav update**

Find the nav links:

```bash
grep -rn '"/explore"\|to="/explore"\|"Explore"' web/src --include='*.tsx'
```

For each, add a "Discover" entry pointing to `/discover` (primary), and rename the existing "Explore" link to "Query Editor" pointing to `/query`.

- [ ] **Step 5: TSC + smoke**

```bash
cd web && pnpm tsc --noEmit
cd web && pnpm dev   # in another terminal
```

Visit `http://localhost:5173/` — should redirect to `/discover` and show the page. Visit `/explore` — redirects to `/query` and shows the (renamed) editor.

- [ ] **Step 6: Commit**

```bash
git add web/src/routes/ web/src/router.tsx web/src/components/Sidebar.tsx # adjust paths
git commit -m "feat(web): mount /discover, rename /explore→/query, redirects"
```

---

### Task 15: Ask AI integration

**Files:** `web/src/features/agent/AskAIDialog.tsx` (if exists; otherwise grep `AskAI`/`Ask AI` in features)

- [ ] **Step 1: Find current dialog**

```bash
grep -rn 'AskAI\|Ask AI' web/src --include='*.tsx' | head
```

- [ ] **Step 2: Extend the agent output contract**

Add a discriminator on the agent reply:

```ts
// In the dialog component, where the agent's structured output is parsed:
type AskAIResult =
  | { kind: "patch"; search?: string; addPills?: Pill[]; removePills?: string[] }
  | { kind: "kql"; db: string; query: string };
```

If the current dialog only returns KQL (the existing behavior), the Discover-facing change is: when the dialog is launched from a Discover tab, pass `mode: "discover"` to the agent prompt. The agent decides which kind to return. If a Discover tab receives a `kql` reply, the dialog calls `newTab({ kind: "query", state: { query: reply.query, ... } })` (same path as Open in Query Editor).

If the agent's prompt/system instructions live in the engine (`/v1/agent/*` or `crates/pensieve-agent/...`), the engine-side prompt should add: "When the user is on Discover, prefer to return `{kind:'patch'}` with structured filters when possible; only use `{kind:'kql'}` when the request can't be expressed in Discover's grammar." That's a separate small commit in the agent crate.

- [ ] **Step 3: Wire the patch path in DiscoverPage**

Pass a callback into the dialog (when launched from Discover) that, on a `patch` reply, merges `addPills`/`removePills`/`search` into the current tab via `updateDiscover`.

- [ ] **Step 4: Compile + commit**

```bash
cd web && pnpm tsc --noEmit
git add web/src/features/agent/ web/src/features/discover/DiscoverPage.tsx
git commit -m "feat(discover): AskAI returns patch or kql; patches apply in Discover"
```

If the agent prompt needs an engine-side tweak, do it as a sibling commit in the agent crate.

---

## Phase 6 — E2E

### Task 16: Playwright golden path

**Files:** `web/tests/e2e/discover.spec.ts`, possibly `web/playwright.config.ts`

- [ ] **Step 1: Inspect existing E2E setup**

```bash
ls web/tests/e2e 2>/dev/null || ls web/e2e 2>/dev/null
cat web/playwright.config.ts 2>/dev/null
```

If no Playwright setup exists, add it:

```bash
cd web && pnpm dlx playwright install
cd web && pnpm add -D @playwright/test
```

And create a minimal `playwright.config.ts` if missing — pattern after the `auth_handler_it.rs` integration tests: spawn the engine + `pnpm dev`, run a single project against `http://localhost:5173`.

- [ ] **Step 2: Write the spec**

```ts
// web/tests/e2e/discover.spec.ts
import { test, expect } from "@playwright/test";

test("discover golden path", async ({ page }) => {
  await page.goto("/");
  // Root redirects to /discover.
  await expect(page).toHaveURL(/\/discover/);

  // The search bar exists and is focusable.
  const search = page.getByPlaceholder(/Search/i);
  await expect(search).toBeVisible();

  // Empty search runs automatically on first paint — sources rail eventually populates.
  await expect(page.getByText("Sources")).toBeVisible();
  await expect.poll(async () =>
    page.locator("aside").getByText(/\d+/).count(), { timeout: 15000 }
  ).toBeGreaterThan(0);

  // Type a filter, hit Enter → pill appears, results refresh.
  await search.fill("service_name:payments");
  await search.press("Enter");
  await expect(page.getByText("service_name:payments")).toBeVisible();

  // Click first row → drawer opens.
  const firstRow = page.locator("tbody tr").first();
  if (await firstRow.count()) {
    await firstRow.click();
    await expect(page.getByRole("dialog")).toBeVisible();
  }

  // Open in Query Editor → URL flips to /query and a tab is open with KQL.
  await page.getByRole("button", { name: /Open in Query Editor/i }).click();
  await expect(page).toHaveURL(/\/query/);
  await expect(page.getByText(/take 500/)).toBeVisible();
});
```

- [ ] **Step 3: Run it**

```bash
cd web && pnpm playwright test discover.spec.ts
```

Expected: pass. (Requires the engine running locally with at least one seeded table.)

- [ ] **Step 4: Commit**

```bash
git add web/tests/e2e/discover.spec.ts web/playwright.config.ts web/package.json
git commit -m "test(e2e): Discover golden path (search → pill → row drawer → open in editor)"
```

---

## Phase 7 — Cleanup

### Task 17: Final sweep

- [ ] **Step 1: TSC clean**

```bash
cd web && pnpm tsc --noEmit
```

- [ ] **Step 2: All unit tests pass**

```bash
cd web && pnpm vitest run
```

- [ ] **Step 3: Engine integration tests still green**

```bash
cargo test -p pensieve-server -- --nocapture
```

- [ ] **Step 4: Manual smoke**

Run `cargo run -p pensieve-bin` and `cd web && pnpm dev`. Open `http://localhost:5173/`.

Verify:
- Lands on `/discover`.
- Sources rail lists every seeded table.
- Typing `auth` shows hits per source (or empty source sections).
- Adding a filter pill (`status:>500`) narrows results.
- Open in Query Editor lands on `/query` with a valid KQL string.
- Visiting `/explore` redirects to `/query`.

- [ ] **Step 5: Tag and commit any final fixes**

If lints flagged anything in the sweep:

```bash
cd web && pnpm prettier -w src
cd web && pnpm eslint --fix src
git diff --quiet || git commit -am "chore(discover): final lint/format pass"
```

---

## Self-Review Checklist

**Spec coverage:**

- §3.1 page anatomy (scope chip + bar + pills + time + histogram + sources rail + fields rail + grouped results + row drawer) — Tasks 5–12.
- §3.2 discriminated tab kind — Task 13.
- §5 grammar mirror — Task 2.
- §6.1 file inventory — File Structure table above; every entry mapped to a task.
- §6.3 state persistence — Task 13 (`partialize` strips transient `results`, `onRehydrateStorage` rehydrates Map).
- §6.4 streaming client — Tasks 1 + 4.
- §7.1 Open in Query Editor — Task 12 (`compileToKql` + button); Task 14 (lands in `/query`).
- §7.2 Ask AI patch-or-kql contract — Task 15.
- §8.4 empty/loading/zero-result states — DiscoverPage (Task 12) renders "no sources match this scope" when `done` + empty sources Map; SourceSection shows "loading…" / "no rows".
- §8.7 cutover via redirect — Task 14 (`/` → `/discover`, `/explore` → `/query`).
- Out-of-scope items (saved searches, charts beyond histogram, dashboards, real-time tail, joins, alerting) — none introduced.

**Placeholder scan:** none. Where a piece is intentionally v2 (e.g. value-breakdown popover in FieldsRail, virtualized table at >500 rows, explicit db/table picker in ScopePicker), it's called out inline as a v2 follow-up — not a TODO in code.

**Type consistency:**

- `Pill` is the single source of truth (defined in `types.ts`), used by grammar, hook, components, AI patches, and compileToKql.
- `DiscoverTabState` is shared between `discover-store.ts` and `workspace-store.ts` via a type-only import.
- Frame type is one of the canonical `Frame` discriminated unions in `sdk/discover.ts`.
- `Scope` flows: SDK → DiscoverPage → ScopePicker → SavedViewsMenu unchanged.

**Open follow-ups (not in this plan):**

- v2: explicit db/table multi-select in ScopePicker (depends on the existing schema-browser).
- v2: value-breakdown popover from FieldsRail (port FieldStats).
- v2: virtualized SourceSection table.
- v2: real-time auto-rerun on time-range or pill change with debouncing.
- v2: org-shared saved views (Plan B's `owner_subject` already supports a future org dimension).
