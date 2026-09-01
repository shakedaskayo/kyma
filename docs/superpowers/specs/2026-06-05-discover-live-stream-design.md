# Discover v2 — Unified Live Stream

**Date**: 2026-06-05
**Status**: Approved (brainstormed with Shaked; decisions locked in session)

## Problem

The Discover page is hard to read: it auto-loads an unfiltered dump of every
source as stacked per-source tables, never states what was searched over which
time range, silently skips the time filter on sources without a timestamp
column, renders embedding vectors as walls of floats, and eats typed searches
into pills. Phase-0 bug fixes (tab dedupe, vector-column hiding, time-filter
badge, internal-db exclusion) landed separately; this spec is the structural
redesign.

## Decisions (locked)

1. **Core model: unified event stream.** One merged, time-sorted (desc) stream
   of rows across all in-scope sources, with a source chip per row and a single
   histogram. Per-source sections are removed.
2. **No-timestamp sources are out of the stream.** They appear in the sources
   rail under "Not in timeline"; clicking one opens it as a plain table view in
   the main panel. They never silently mix into the timeline.
3. **Persistent query bar.** The typed query text stays in the bar, editable,
   and is the single source of truth. UI interactions (clicking a value in a
   row, fields rail) insert text into the bar — the pills model is removed.
   The bar's text is sent verbatim as `query` to the backend (which already
   parses the same grammar server-side).
4. **Actually live, over WebSocket + ingest notify.** Transport alone is not
   instant; the ingest path must notify live sessions. Design couples a WS
   endpoint with an in-process "rows appended" event bus.

## Architecture

### Page layout

```
┌──────────────────────────────────────────────────────────────────┐
│ [Scope ▾] [ query bar — persistent ] [🔴 Live ▾] [Last 1h ▾]      │
│ Searched 4 sources · last 1h · 1,284 events · as of 12:04:31      │
├──────────┬────────────────────────────────────────────────────────┤
│ SOURCES  │ ▂▃▇▅▂▁▃   histogram, labeled time axis, brush = zoom   │
│ ☑ a 890  ├────────────────────────────────────────────────────────┤
│ ☑ b 394  │ 12:04:31  src-chip  smart summary (message-first,      │
│ NOT IN   │ 12:04:30  src-chip  dimmed k=v pairs, never vectors)   │
│ TIMELINE │ …          row click → detail drawer                   │
│ FIELDS   │            value click → inserts `field:value` in bar  │
└──────────┴────────────────────────────────────────────────────────┘
```

- **Summary line** (required): sources searched, time window, event count,
  "as of" stamp. Direct fix for "can't tell what's going on".
- **Stream row**: timestamp · source chip · summary cell. Summary cell picks a
  message-ish field (`message`, `body`, `content`, `msg`, longest string
  fallback) and renders remaining fields as dimmed `k=v` pairs. Vector columns
  (per phase-0 `partitionColumns`) never render in the stream.
- **Fields rail**: per-selected-source fields; clicking a field toggles it as
  an explicit column; value-level affordances insert filter text into the bar.
- **Sources rail**: per-source counts + visibility checkboxes (client-side
  stream filter), plus the "Not in timeline" group for no-timestamp sources.

### Live: `/v1/explore/live` (WebSocket)

- `GET /v1/explore/live` upgrades to WS. Registered in pensieve-server and mounted
  in pensieve-local, **outside** Bearer-auth middleware (browsers cannot set WS
  headers). See `docs` note on local-mode capabilities: new /v1 surfaces need
  both mounts.
- **Auth**: first client message `{"type":"auth","token"}` within 5s, validated
  against the same AuthBackend; close on failure/timeout.
- **Client → server**: `auth`, `subscribe {query, scope, time_range,
  per_source_limit}`, `update {…}` (re-subscribe in place), `pause`, `resume`.
- **Server → client**: backfill pass reuses the existing fanout — emits the
  same frames as `/v1/explore/search` (`plan`, `rows`, `histogram`,
  `source_done`) — then `{"type":"live"}`, then incremental `rows` frames,
  `heartbeat` every 15s, `error`.
- **Ingest notify**: bounded `tokio::broadcast` of `{db, table}` published at
  the engine's commit choke-point (shared by REST/OTLP/Kafka/filedrop ingest).
  Live sessions filter events to their resolved sources, debounce ~150ms, then
  run an incremental scan (`ts > per-source high-water cursor`) and push rows.
  30s timer fallback for paths that bypass notify. Notify-then-scan keeps all
  filtering/grammar in one place; no rows travel through the bus.
- **Reconnect**: client backoff + re-subscribe with `time_range.from = last
  seen ts`. Same-millisecond duplicates are accepted in v1.

### Live UX

- 🔴 Live toggle in the toolbar; new rows prepend with a brief highlight;
  histogram buckets increment client-side.
- Scrolling down auto-pauses ("⏸ paused — N new events ↑ jump to latest");
  jumping back to top resumes.
- Status dot: live / connecting / paused / error, wired to WS state.
- Editing the query while live sends `update` — no reconnect.

## Phases

- **Phase 1 — unified stream UI** on the existing one-shot `/v1/explore/search`
  protocol: merge logic, summary line, persistent query bar (pills removed,
  with persisted-state migration), labeled histogram + brush, rails, row
  drawer, not-in-timeline table view. No backend change.
- **Phase 2 — live**: WS endpoint + ingest notify bus + live UX.

## Error handling

- WS auth failure/timeout → close with coded reason; client surfaces "sign-in
  expired" and falls back to one-shot search.
- Broadcast lag/overflow → session falls back to scanning all its cursors.
- Per-source scan errors in live mode → same `error` frame semantics as
  search; source marked errored in the rail, stream continues.
- Query grammar errors → inline under the bar (no toast, no console-only).

## Testing

- **Backend**: unit tests for cursor/incremental-scan and debounce; WS
  integration test (auth timeout, subscribe → backfill order → `live` marker →
  incremental rows); pensieve-local test asserting `/v1/explore/live` is mounted
  and upgrades (prevents the dashboards-405 class of dual-mount bug).
- **Frontend**: pure-function tests for time-ordered merge/insert + dedupe,
  summary-line formatter, query-bar text insertion, message-field picker;
  existing vitest setup.
