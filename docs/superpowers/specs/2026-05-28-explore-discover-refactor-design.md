# Explore → Discover Refactor: Design Spec

**Date:** 2026-05-28
**Status:** Approved (brainstorming) — pending implementation plan
**Owner:** Shaked
**Goal:** Replace the editor-first Explore page with a Kibana-Discover-style surface that lets any user search, filter, and browse data across every database and table in the deployment without writing a query language. Production-grade end-to-end: new engine endpoint, real fanout, real limits, real errors, real tests.

## 1. Motivation

Today's `_app.explore.tsx` is a Monaco KQL editor with a free-text bar bolted on. Three pains compound:

1. **Syntax barrier.** Users land on a blank editor with a pipe-syntax language they don't know.
2. **Discovery gap.** Users don't know what databases / tables / fields / values exist before they can compose anything.
3. **Single-source scope.** Every query is scoped to one `x-database` and one leading table. There is no way to type a phrase and see hits across the whole deployment.

The refactor addresses all three at once by adopting the Kibana Discover model — a no-code surface where the user types into a search bar, results stream in grouped by source, and any field or value in the results becomes a click-to-filter pill.

## 2. Goals & non-goals

### Goals

- A user with zero query-language knowledge can find data by typing into one search bar.
- Default scope is the entire deployment (all DBs + all tables the caller can read).
- Power users keep a clean escape hatch to KQL via a separate Query Editor tab.
- Production-grade: server-side fanout, per-source and global budgets, RBAC-aware scope resolution, structured per-request observability, contract-tested NDJSON streaming, golden-path E2E tests.

### Non-goals (v1)

- Charts beyond the time histogram.
- Saved *searches* (saved scope+search+pills) — v1 only ships saved *views* (named scopes).
- Dashboards built from Discover panels.
- Real-time tail / live streaming.
- Cross-source joins.
- Alerting from a saved view.

## 3. UX overview

### 3.1 Page anatomy

Single new route `/_app.discover.tsx` becomes the default landing (`/` redirects to `/discover`). Top-to-bottom layout:

```
┌─ Top bar ─────────────────────────────────────────────────────────────┐
│  Scope: [ All sources ▾ ]   Search: type anything…       [▸ Run]     │
│  Filters: [service:payments ×] [-severity:INFO ×] [+ Add]   ⏱ 15m ▾  │
├─ Left rail (240px) ─────────┬─ Main ────────────────────────────────┤
│  ▼ Sources                  │  Time histogram (stacked by source)    │
│    ☑ prod.otel_logs  (1.2k) │  ▁▁▂▅█▇▆▄▃▂▁                           │
│    ☑ prod.http_reqs  (340)  ├───────────────────────────────────────┤
│    ☐ stg.otel_logs   (89)   │  ▼ prod.otel_logs   1,238 hits         │
│                             │     timestamp  severity  message  …    │
│  ▼ Fields (in selected)     │     14:32:01   ERROR     conn refused  │
│    □ severity_text  ⓘ       │     ▸ … (paged, virtualized)           │
│    □ service_name  ⓘ        │  ▶ prod.http_reqs   340 hits           │
└─────────────────────────────┴───────────────────────────────────────┘
```

- **Scope chip.** Popover with three modes: `All sources` (default), `Pick db/tables` (multi-select tree), `Use saved view`. Sticky per-tab.
- **Search bar.** The only required input. Empty search = "recent everything in scope". Submits on Enter or the Run button.
- **Filter pills.** Added/removed via the bar, the fields rail, or click-to-filter in result rows. Each pill is `field:value`, `-field:value`, `field:>n`, or a quoted free phrase. Pills apply globally across all source sections.
- **Time range.** Existing component governs every source that exposes a timestamp column; sources without timestamps are unaffected and labeled "no time filter applied" in the source header.
- **Histogram.** Stacked-by-source, returned by the engine endpoint as a dedicated frame; client only renders.
- **Sources rail.** Every db.table that matched the current search, with hit counts. Checkboxes toggle visibility of result sections (no re-query — the engine already returned them).
- **Fields rail.** Scoped to the *expanded* (or singly-ticked) source. Click a field → adds to the source's column list AND opens a popover with top-N value breakdown (port today's `FieldStats` behavior).
- **Results.** Grouped accordion sections, one per source. Each section keeps its own columns, sort, and virtualized table. Row click → detail drawer with full row JSON + "Filter by this value" actions.
- **No editor on this page.** Action menu: `Open in Query Editor` (compiles current state to KQL in a new Query tab), `Save as view…`, `Ask AI…`.

### 3.2 Tab model

Workspace store gains a discriminated tab kind:

```ts
type Tab =
  | { id: string; kind: "discover"; state: DiscoverTabState }
  | { id: string; kind: "query";    state: QueryTabState }   // today's editor
```

Tab bar renders distinct icons. Saved-from-Discover URLs and saved-from-Query URLs are different artifacts; neither morphs into the other implicitly.

## 4. Backend

### 4.1 New endpoint: `POST /v1/explore/search`

**Request body:**

```json
{
  "query": "auth -severity:INFO service:payments",
  "scope": {
    "kind": "all" | "sources" | "view",
    "sources": ["prod.otel_logs", "prod.http_*"],
    "viewId": "uuid-or-null"
  },
  "time_range": {
    "from": "2026-05-28T13:00:00Z",
    "to":   "2026-05-28T14:00:00Z"
  },
  "per_source_limit": 500,
  "histogram": { "interval_ms": 60000 }
}
```

Exactly one of `scope.sources` or `scope.viewId` is honored depending on `scope.kind`. `scope.kind: "all"` ignores both. Entries in `scope.sources` are `db.table` patterns; `*` is a wildcard within either segment (`prod.*` = all tables in `prod`; `*.otel_logs` = `otel_logs` in any DB). When `scope.kind: "view"`, the view supplies the scope only — `time_range`, `query`, and `per_source_limit` always come from the request body, never the saved view.

Pagination beyond `per_source_limit` is **out of scope for v1**. When a source returns more rows than the cap, the `source_done` frame sets `capped: true` and the UI shows "showing first 500 of 1,238 — open in Query Editor for more". A `cursor` field is reserved for v2 drilldown and not accepted in v1.

**Headers:** existing `x-kyma-max-wall-clock-ms`, `x-kyma-max-memory-bytes` honored as a *global* budget (not per-source). Existing auth middleware applies.

**Response:** `Content-Type: application/x-ndjson`. One JSON object per line. Discriminated frame types:

```json
{"type":"plan","sources":[{"db":"prod","table":"otel_logs","has_timestamp":true}, ...]}
{"type":"source_progress","source":"prod.otel_logs","state":"running"}
{"type":"rows","source":"prod.otel_logs","rows":[ ... up to per_source_limit ... ]}
{"type":"histogram","source":"prod.otel_logs","buckets":[{"t":"2026-05-28T13:00:00Z","n":17}, ...]}
{"type":"source_done","source":"prod.otel_logs","total":1238,"capped":true,"dropped_clauses":[]}
{"type":"error","source":"prod.http_reqs","code":"timeout","message":"..."}
{"type":"done","elapsed_ms":842}
```

`plan` is always the first frame. `done` is always the last. Per-source frames may interleave arbitrarily.

**Engine behavior:**

1. Resolve `scope` against the catalog → set of `(db, table)` pairs the caller can read (RBAC filter).
2. Reject early with `scope_too_large` if the resolved set exceeds `max_sources_per_request` (default 200).
3. For each source, compile the search grammar to KQL (Section 5) — fields the source doesn't have are silently dropped from clauses (not an error).
4. Execute per-source queries in parallel under one shared wall-clock and memory budget.
5. Emit `rows` and `histogram` frames as each source completes (or partial frames if streaming row chunks).
6. Per-source errors emit `{"type":"error",...}` and do **not** abort siblings. Global budget exhaustion emits a final `error` and `done`.

### 4.2 Saved-views endpoints

New table `engine.saved_views`:

| column              | type      | notes                                        |
|---------------------|-----------|----------------------------------------------|
| id                  | uuid pk   |                                              |
| name                | text      | unique per owner                             |
| owner               | text      | user id                                      |
| scope_json          | json      | `{ "sources": [...] }`                       |
| default_columns_json| json      | optional per-source default columns          |
| created_at          | timestamp |                                              |
| updated_at          | timestamp |                                              |

Endpoints:

- `GET /v1/explore/views` — list views owned by caller
- `POST /v1/explore/views` — create
- `PATCH /v1/explore/views/:id` — rename / update scope
- `DELETE /v1/explore/views/:id`

A view is a *scope only*, not a saved query. Saving search+pills+time is explicitly v2.

### 4.3 Migration

One additive migration: `engine.saved_views` table. No destructive changes. The existing `POST /v1/query` endpoint, `x-database` header, and KQL semantics are untouched — the Query Editor tab still uses them.

## 5. Search grammar

Extension of today's `web/src/features/search/parseSearch.ts`, mirrored on the engine side so the grammar lives in one canonical place (engine compiles authoritatively; frontend parses only to render pills and validate input).

Tokens:

| Token form                  | Semantics                                 |
|-----------------------------|-------------------------------------------|
| `auth`                      | substring across all string columns       |
| `"foo bar"`                 | phrase substring match                    |
| `field:value`               | `field == "value"`                        |
| `-field:value`              | `field != "value"`                        |
| `field:>N` `field:<N` `>=` `<=` | numeric comparison                    |
| `field:*`                   | `isnotnull(field)`                        |

Multiple tokens combine with implicit AND. The pills bar is a structured rendering of the same grammar — every pill round-trips to a token in the search bar.

**Per-source compilation tolerance.** When compiling for a given source:

- Clauses referencing a field the source does not have are silently dropped.
- Clauses with a type-incompatible value (e.g. `status:>500` against a string column) are silently dropped *for that source only* — no error, no source-level failure.
- A source whose compiled query has zero remaining clauses still runs (returns recent rows in the time range, capped at `per_source_limit`).
- The engine includes a `dropped_clauses` field on `source_done` listing which user clauses were dropped per source, so the UI can show a small "?" tooltip explaining why this source returned everything.

Per source, compile to:

```
<db.table>
| where <conjunction-of-clauses>
| where timestamp between(from, to)   # only if has_timestamp
| take <per_source_limit>
```

Discover UI never shows raw KQL. The grammar leaves the page boundary only via the `Open in Query Editor` action, which emits a `union` of per-source pipes (or a single pipe for a single-source scope).

## 6. Frontend architecture

### 6.1 New files

```
web/src/routes/
  _app.discover.tsx                 # new default landing
  _app.query.tsx                    # renamed today's _app.explore.tsx

web/src/features/discover/
  DiscoverPage.tsx
  ScopePicker.tsx
  SearchBar.tsx
  FilterPills.tsx
  Histogram.tsx
  SourcesRail.tsx
  FieldsRail.tsx
  SourceSection.tsx
  RowDetailDrawer.tsx
  SavedViewsMenu.tsx
  useDiscoverSearch.ts              # TanStack Query wrapper around streaming endpoint
  discoverGrammar.ts                # parse/serialize the search grammar (mirrors engine)
  discover-store.ts                 # Zustand slice for current discover tab

web/src/sdk/
  discover.ts                       # typed client for /v1/explore/search and /v1/explore/views
```

### 6.2 Modified files

- `web/src/features/tabs/workspace-store.ts` — add discriminated `kind: "discover" | "query"` to Tab; migrate existing persisted tabs as `kind: "query"`.
- `web/src/features/tabs/TabBar.tsx` — distinct icons per kind.
- `web/src/features/search/parseSearch.ts` — extend with numeric comparison and `field:*` tokens; keep backwards compatible.
- `web/src/routes/_app.explore.tsx` — deleted; replaced by `_app.query.tsx`.
- Router root — `/` → `/discover` redirect; `/explore` → `/query` redirect (one release).

### 6.3 State persistence

Discover tabs persist in localStorage via the existing workspace store pattern:

```ts
type DiscoverTabState = {
  scope: { kind: "all" | "sources" | "view"; sources?: string[]; viewId?: string };
  search: string;
  pills: Pill[];
  timeRange: TimeRange;
  expandedSources: string[];        // db.table keys
  selectedSourceForFields: string;  // for fields rail
};
```

### 6.4 Streaming client

`useDiscoverSearch` returns `{ plan, sources: Map<string, SourceState>, isLoading, error, cancel }`. Each NDJSON frame mutates the relevant `SourceState` (rows append, histogram set, error set, capped flag set). Rendering is by-source so partial results paint as they arrive.

## 7. Escape hatch & AI

### 7.1 Open in Query Editor

Compiles current scope + search + pills + time range to KQL and opens a new `query` tab pre-filled. Multi-source compiles to a `union` of per-source pipes; single-source compiles to a single pipe. **One-way** — no sync back into Discover.

### 7.2 Ask AI

The existing `AskAIDialog` is repurposed. Inputs to the agent: current scope, search text, pills, time range, and catalog snapshot. Output contract:

```ts
type AskAIResult =
  | { kind: "patch"; search?: string; addPills?: Pill[]; removePills?: string[] }
  | { kind: "kql"; db: string; query: string };
```

`patch` applies inside Discover. `kql` opens Query Editor. The agent chooses based on whether the request fits the grammar.

## 8. Production-grade hardening

### 8.1 Limits & budgets

| Limit                                  | Default     | Configurable |
|----------------------------------------|-------------|--------------|
| `per_source_limit` (rows per source)   | 500         | request body, capped server-side at 5,000 |
| `max_sources_per_request`              | 200         | engine config |
| Global wall-clock                      | 10s         | `x-kyma-max-wall-clock-ms` header |
| Global memory                          | existing    | `x-kyma-max-memory-bytes` header |

Scopes resolving to more than `max_sources_per_request` are rejected pre-execution with `scope_too_large`; the UI links the user to the Scope picker.

### 8.2 Auth & RBAC

- Endpoint reuses existing auth middleware.
- Scope resolution filters out tables the caller cannot read (silent drop — no leak of existence via error messages).
- `saved_views` rows are owner-scoped; reads and writes both check ownership. (Org-shared views are v2.)

### 8.3 Error surfacing

| Failure mode             | Frame / response                              | UI                                                  |
|--------------------------|-----------------------------------------------|-----------------------------------------------------|
| Per-source timeout       | `error` frame with `code:"timeout"`           | Red dot + tooltip in that source's section header   |
| Per-source query error   | `error` frame with `code:"query_error"`       | Same; source section shows the message              |
| Scope too large          | HTTP 400 with `{code:"scope_too_large"}`      | Banner with link to Scope picker                    |
| Global budget exhausted  | Final `error` + `done`                        | Banner with copyable trace id                       |
| RBAC drop                | Silent (source omitted from `plan`)           | No UI; user only sees sources they can read         |

### 8.4 Empty / loading / zero-result states

- Streaming → skeleton rows per source as `source_progress` arrives.
- Resolved-but-empty scope (`plan.sources` is empty) → "no sources match this scope" with Scope picker link.
- Scope matched but zero rows → "no matches in selected time range" with a "widen time range" suggestion.
- Browser tab backgrounded mid-stream → request continues; on refocus the partial results are still visible.

### 8.5 Observability

Per-request structured engine log:

```json
{
  "endpoint": "/v1/explore/search",
  "user": "...",
  "scope_kind": "all",
  "scope_resolved_sources": 38,
  "search_len": 27,
  "pill_count": 2,
  "time_range_ms": 900000,
  "total_rows_returned": 4127,
  "total_rows_capped": 3,
  "elapsed_ms": 842,
  "per_source_errors": 1,
  "trace_id": "..."
}
```

New Prometheus counters under `explore_search_*`: requests, errors_by_code, sources_resolved_histogram, per-source latency histogram, cap-hits.

### 8.6 Testing

**Engine:**

- Unit tests for grammar → KQL compiler (every token + drop-unknown-field behavior).
- Integration tests for fanout: success across N sources, partial per-source failure, scope-too-large, per-source timeout, global budget exhausted, RBAC drop, empty plan, histogram correctness.
- Contract test pinning the NDJSON frame shapes.

**Frontend:**

- Vitest for `discoverGrammar.ts` (parse round-trip; pill serialization).
- Vitest for `useDiscoverSearch` against a mocked NDJSON stream (partial frames, error frames, cancellation).
- Playwright golden-path E2E: load Discover → empty search returns recent rows from multiple sources → add a field:value pill → expand a source → open row drawer → "Filter by this value" adds a pill → "Open in Query Editor" produces a runnable KQL tab.

### 8.7 Feature gate & cutover

- Engine endpoint and frontend route ship behind a feature gate `discover_v1`.
- Internal dogfood for one cycle (`/` still defaults to `/explore` while the gate is off).
- After dogfood acceptance: flip the gate on, `/` → `/discover` becomes the default redirect, `/explore` → `/query` redirect lands.
- One release later: delete the old Explore page code, delete the gate, delete the `/explore` redirect.

## 9. Risks & open questions

| Risk                                            | Mitigation                                                       |
|-------------------------------------------------|------------------------------------------------------------------|
| Engine fanout cost dominates request time       | Per-source parallelism cap + global budget + cap-hit observability |
| Grammar drift between frontend parser and engine compiler | Single source-of-truth grammar doc + contract tests on token set |
| Workspace-store migration breaks persisted tabs | Migration treats every legacy tab as `kind: "query"`; covered by Vitest |
| RBAC drop hides debugging signal from operators | Add an operator-only flag to log dropped sources in trace_id     |
| Histogram interval choice                       | Default = time_range / 50 buckets clamped to [1s, 1h]; revisit after dogfood |

Open:

- Should the `Open in Query Editor` action also seed the Query tab's saved-query catalog with a "from Discover" entry? (v2 decision.)
- Should saved views support sharing across users / orgs? (v2.)

## 10. Out of scope (explicit)

- Saved searches (scope + search + pills as a named artifact). Saved *views* (scope only) ship in v1.
- Charts beyond the histogram.
- Dashboards.
- Real-time tail.
- Cross-source joins.
- Alerting.

---

## Appendix A — file inventory

| File                                                    | Action     |
|---------------------------------------------------------|------------|
| `web/src/routes/_app.discover.tsx`                      | new        |
| `web/src/routes/_app.query.tsx`                         | new (renamed from `_app.explore.tsx`) |
| `web/src/routes/_app.explore.tsx`                       | delete (post-cutover) |
| `web/src/features/discover/*` (11 new files)            | new        |
| `web/src/sdk/discover.ts`                               | new        |
| `web/src/sdk/discover.test.ts`                          | new        |
| `web/src/features/search/parseSearch.ts`                | extend     |
| `web/src/features/search/parseSearch.test.ts`           | extend     |
| `web/src/features/tabs/workspace-store.ts`              | extend (discriminated tab kind + migration) |
| `web/src/features/tabs/TabBar.tsx`                      | extend     |
| `web/src/router.tsx` (or equivalent)                    | extend (redirects) |
| engine: `POST /v1/explore/search` handler               | new        |
| engine: `GET/POST/PATCH/DELETE /v1/explore/views`       | new        |
| engine: migration `00NN_saved_views.sql`                | new        |
| engine: grammar→KQL compiler module                     | new        |
| engine: fanout executor (parallel + budget)             | new        |

## Appendix B — example end-to-end flow

1. User opens `/` → redirected to `/discover`.
2. Default tab loads with scope=`all`, empty search, time=last 15m.
3. Page issues `POST /v1/explore/search` with empty query.
4. Engine resolves scope → 38 readable sources → fans out → streams NDJSON.
5. Frontend renders sources rail (38 entries with hit counts) and grouped result sections as frames arrive.
6. User types `auth` and hits Enter → request re-issued; results re-render.
7. User clicks `service_name: payments` in a row → pill added; request re-issued with `service:payments`.
8. User clicks `Open in Query Editor` → new Query tab opens with `union prod.otel_logs | where ... | take 500, prod.http_reqs | where ... | take 500`.
