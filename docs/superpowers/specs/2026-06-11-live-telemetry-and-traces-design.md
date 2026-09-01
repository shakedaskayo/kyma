# Live telemetry + Traces page — design

**Date:** 2026-06-11
**Status:** approved (design), pending implementation plan

## Problem

Three user-visible failures on the Memory surface, one root cause discovered during investigation:

1. **Live pulse shows "1d ago" for events that appear to arrive live.** Investigation showed the data is genuinely stale: the freshest `claude_code_events` row was 4 days old. Capture had been silently dead since June 7 — `pensieve service install` mints and pins a token in the launchd plist (`PENSIEVE_AUTH_TOKENS`) but never updates `~/.pensieve/config.json`, so every hook ingest and CLI call got `401 unknown token` and dropped. The UI compounded this by animating old backfill rows in one-per-second as if they were arriving now. Timestamp serialization itself is correct (ISO-8601 with `Z`; `relTime` in `web/src/lib/time.ts` is sound).
2. **Memory retrievals / fetches / ingestions never appear in the live stream.** They cannot: no handler in `pensieve-server` (memory query/recall, import, export, agent query, ingest) emits any observable event. The stream only tails the Claude Code hook firehose (`default.claude_code_events`).
3. **No Traces page.** There is no way to see what an entity (coding-agent identity, workstation) is doing against Pensieve end-to-end. `pensieve-ingest-otlp` currently receives **logs only** (`otel.otel_logs`); OTLP trace ingest is an unimplemented "phase B". The server's own `tracing` spans go to stdout; nothing is exported or stored.

## Decisions (made with user)

- Memory-op events are **derived from Pensieve's own traces** — one telemetry system, no separate audit table.
- The Traces page shows **self-traces and external traces**: implement the generic OTLP traces receiver and have `pensieve-server` feed its own spans through the same path (dogfooding).
- Capture-pipeline robustness (token drift + silent-failure surfacing) is **in scope**, not just a local token fix.

## Design

### Part 1 — Capture pipeline hardening (CLI + server)

- `pensieve service install` (and any path that mints `PENSIEVE_AUTH_TOKENS`) writes the same token into `~/.pensieve/config.json` atomically. One source of truth; the plist and the CLI config can no longer drift.
- `pensieve status` adds an authenticated probe: one tokened request after `/health`, reporting `auth: ok` or `auth: TOKEN REJECTED — re-run pensieve service install or pensieve connect`.
- The capture hook records non-2xx ingest outcomes to `~/.pensieve/capture-health.json` (`{ts, status, endpoint, detail}`) instead of dropping silently. `pensieve status` and the pensieve-status skill read and surface it.

*Ops note:* the live local instance was repaired on 2026-06-11 by syncing `~/.pensieve/config.json` to the plist token (backup at `~/.pensieve/config.json.bak-20260611`).

### Part 2 — OTLP traces receiver + self-instrumentation

**Receiver (`pensieve-ingest-otlp` phase B):** implement `ExportTraceServiceRequest` on the existing OTLP gRPC listener, alongside logs. Spans land in `otel.otel_traces`:

| column | type | notes |
|---|---|---|
| `start_time` | Timestamp(ns, UTC) | span start; primary time column |
| `end_time` | Timestamp(ns, UTC) | |
| `duration_ns` | Int64 | precomputed for cheap sorting/filtering |
| `trace_id`, `span_id`, `parent_span_id` | Utf8 | hex-encoded; `parent_span_id` empty for roots |
| `name`, `kind` | Utf8 | span name; SpanKind as text |
| `status_code`, `status_message` | Utf8 | |
| `service_name` | Utf8 | from resource attributes |
| `subject` | Utf8 | promoted from `pensieve.subject` attribute (entity identity) |
| `tenant` | Utf8 | promoted from `pensieve.tenant` |
| `attributes_json`, `resource_json` | Utf8 | remaining attributes, merged JSON |

`subject`/`tenant` are real columns (not buried in JSON) because the Traces page filters on them constantly.

**Self-instrumentation (`pensieve-server` / `pensieve-bin`):**

- Add a `tracing-opentelemetry` layer with a custom **in-process span exporter** that writes batches through the same internal ingest path the OTLP receiver uses — no loopback gRPC.
- **Recursion guard:** spans originating from the span-writer / otlp-ingest targets are filtered out of the exporter pipeline. Health checks and live-tail frame pumps are excluded as noise.
- **Span shape:** root span per HTTP/WS request created in auth middleware with `pensieve.subject`, `pensieve.tenant`, `http.route`, response status. Child spans on memory ops — `memory.recall`, `memory.save`, `memory.import`, `memory.export`, `agent.query`, `ingest.batch` — carrying op attributes (truncated query text, row/memory counts, store hit counts).

### Part 3 — Live pulse fixes (web)

- **Memory-op events in the stream:** `useMemoryStream` opens a second `LiveSession` tailing `otel_traces` filtered to memory/ingest op spans, merged client-side into the same pulse. New `KIND_STYLE` entries: recall, save, ingest, query (distinct accents). The polling fallback covers the second source identically.
- **Honest backfill:** rows older than 5 minutes *at arrival time* are history — rendered immediately under a subtle "earlier" divider, no staged one-per-second release, no slide-in animation. Only genuinely fresh events animate. (This kills the "old event pretending to arrive live" illusion behind the original report.)
- **Capture-stalled indicator:** when stream status is live/polling but the newest event is older than 10 minutes, the rail header shows an amber "capture stalled — last event Xd ago" state with a pointer to `pensieve status`, instead of a healthy pulse.

### Part 4 — Traces page (web)

- New sidebar item **Traces** in the Explore group (`web/src/app/Sidebar.tsx`), route `/traces`.
- **List view:** one row per trace, grouped from root spans via KQL on `otel_traces`. Columns: start time, root operation name, entity (`subject`), service, duration, span count, status. Filters: time-range presets, service, subject, status, free-text over span names. A "live" toggle keeps the list fresh by polling every 5s (the self-trace batch exporter flushes on a similar cadence, so a socket buys nothing here).
- **Detail view** (`/traces/$traceId` or drawer): waterfall span tree — indented parent/child layout, proportional duration bars, per-span attributes panel (JSON attributes, status, timings). Data: `otel_traces | where trace_id == '…'`.
- **Entity-centric flow:** clicking a subject filters the list to that identity — "what is this workstation/agent doing end-to-end".

## Error handling

- Exporter back-pressure: span batches that fail to write are dropped with a counter metric, never block request handling.
- Receiver rejects malformed OTLP payloads per-resource-span, not per-request, so one bad span doesn't kill a batch.
- Web live sessions: existing socket→polling fallback applies to both sources; a failed second source degrades to firehose-only with the stalled indicator logic still active.

## Testing

- Rust: unit tests for OTLP trace decode → row mapping (ids hex-encoded, times, subject/tenant promotion); recursion-guard test (span-writer activity produces no new spans); e2e: hit a memory endpoint, assert a span row lands in `otel_traces` with the caller's subject.
- Web: Vitest for backfill-vs-live classification and stalled-indicator thresholds; Playwright e2e for the Traces list + waterfall against seeded spans, and for memory-op events appearing in the pulse.

## Out of scope

- OTLP metrics ingest; OTLP/HTTP (protobuf-over-HTTP) transport if the current listener is gRPC-only — gRPC parity with logs is the bar.
- Tracing the embedded `pensieve mcp` local-catalog path (no server involved).
- Sampling/retention policies for `otel_traces` beyond defaults (follow-up with the compaction story).
