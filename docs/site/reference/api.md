---
title: HTTP API
description: Every HTTP route the kyma engine serves — method, path, auth role, content-type, request and response shapes. Arrow Flight (gRPC, separate port) included.
---

# HTTP API

Every HTTP route the engine mounts when `kyma` is running. Auth is
optional — when `KYMA_AUTH_TOKENS` is empty the role column is
informational and every route accepts any caller. When auth is on,
the listed minimum role is required, presented as
`Authorization: Bearer <token>`.

The HTTP server listens on `KYMA_HTTP_ADDR` (default `0.0.0.0:8080`).
Arrow Flight runs on its own port (default `0.0.0.0:9090`,
configurable via `KYMA_GRPC_ADDR`); OTLP gRPC, when enabled, runs on
`KYMA_OTLP_ADDR` (off by default; conventional port `4317`).

Every JSON error response uses the same shape:

```json
{ "error": { "code": "...", "message": "...", "request_id": "..." } }
```

`X-Request-ID` is set on the request if the client didn't send one,
and propagated back on the response. Logs and error bodies share that
id.

## Health and metrics

Always unauthenticated.

| Method | Path       | Response                                     | Notes                                  |
| ------ | ---------- | -------------------------------------------- | -------------------------------------- |
| GET    | `/health`  | `200 application/json` — `{status, version}` | Liveness probe.                        |
| GET    | `/metrics` | `200 text/plain` — Prometheus exposition     | Histogram buckets tuned for `_duration_seconds` metrics. |

## Query

The query surface accepts SQL, KQL, or a query in any frontend the
content-type registers. Results stream back as NDJSON — one JSON
object per row.

| Method | Path                  | Min role | Content-Type                                       | Effect                                                                  |
| ------ | --------------------- | -------- | -------------------------------------------------- | ----------------------------------------------------------------------- |
| POST   | `/v1/query`           | Read     | `application/sql` (default), `application/x-kql`   | Run a query against the database in `X-Database` (default `default`).  |
| GET    | `/v1/catalog/schema`  | Read     | —                                                  | Tree of databases → tables → columns. Cached server-side (default 5 s). |

Query request headers:

| Header                            | Default     | Effect                                                  |
| --------------------------------- | ----------- | ------------------------------------------------------- |
| `X-Database`                      | `default`   | Target database.                                        |
| `Content-Type`                    | `application/sql` | `application/x-kql` switches to KQL parsing.       |
| `X-Kyma-Max-Wall-Clock-Ms`        | server-side | Per-query wall-clock budget; min 10 ms.                 |
| `X-Kyma-Max-Memory-Bytes`         | server-side | Memory pool ceiling for the DataFusion run; min 1 MiB.  |
| `X-Kyma-Max-Object-Store-Bytes`   | server-side | Bytes the scan may read from object store.              |

Query response headers:

| Header           | Meaning                                              |
| ---------------- | ---------------------------------------------------- |
| `Content-Type`   | `application/x-ndjson; charset=utf-8`.               |
| `X-Kyma-Rows`    | Total row count returned.                            |
| `X-Request-ID`   | Echo of the request id.                              |

Status codes: `200` (rows in body), `400` (parse error / empty body),
`404` (database empty or unknown), `413` (body > 16 MiB),
`429` (budget exceeded — `X-Kyma-Budget-Limit` header included),
`500` (execution error).

## Ingest

| Method | Path                                                | Min role | Content-Type            | Effect                                                |
| ------ | --------------------------------------------------- | -------- | ----------------------- | ----------------------------------------------------- |
| POST   | `/v1/ingest`                                        | Write    | `application/x-ndjson`  | Append rows to a table. NDJSON body, max 64 MiB.      |
| POST   | `/v1/admin/databases/{database}/tables/{table}`     | Write    | —                       | Idempotent table provisioning.                        |
| GET    | `/v1/admin/databases/{database}/tables/{table}`     | Write    | —                       | Return columns + row count for the table.             |

Ingest request headers:

| Header               | Default   | Effect                                                                                                       |
| -------------------- | --------- | ------------------------------------------------------------------------------------------------------------ |
| `X-Database`         | `default` | Target database.                                                                                             |
| `X-Table`            | required  | Target table.                                                                                                |
| `X-Idempotency-Key`  | —         | Replay-safe key. A repeated request with the same key returns the original ack with `replayed: true`.        |
| `X-Auto-Create`      | `true`    | Create the database + table on first write. Set `false` to require pre-provisioning.                        |
| `X-Schema-Evolve`    | `true`    | Add new columns mid-batch. Set `false` to drop unknown fields silently.                                      |

Ingest response (`200 application/json`):

```json
{
  "snapshot_id": "...",
  "extent_count": 1,
  "rows_ingested": 123,
  "bytes_written": 4567,
  "replayed": false
}
```

Status codes: `200` (committed), `400` (missing `X-Table`, malformed
NDJSON), `404` (table not found with `X-Auto-Create: false`),
`413` (body > 64 MiB), `500` (commit failed).

## Agent

| Method | Path                       | Min role | Content-Type             | Effect                                                       |
| ------ | -------------------------- | -------- | ------------------------ | ------------------------------------------------------------ |
| POST   | `/v1/agent/ask`            | Read     | `application/json`       | Run one agent turn. Streams Server-Sent Events.              |
| GET    | `/v1/agent/runs/{run_id}`  | Read     | —                        | Look up a persisted run by uuid.                             |

`POST /v1/agent/ask` body:

```json
{
  "question": "...",
  "database": "default",
  "include_thinking": false,
  "session_id": "<uuid>",
  "source": "kyma"
}
```

Pass `session_id` (a uuid returned in a prior turn's `data-session` SSE part) to continue a conversation. `source` is a free-form label stored on the session row (`"claude_code"`, `"kyma"`, etc.).

The SSE response carries seven event names: `run_started`,
`thinking_delta` (only when `include_thinking: true`),
`answer_delta`, `tool_call`, `tool_result`, `answer_final`,
`run_error`, `run_finished`. `run_finished` is always the last
frame. See [Agent endpoint](/query/agent-endpoint) for the full
event payload reference.

Run wall-clock cap: 60 s. Per-run tool-call cap: 12. Both surface as
`run_error` codes (`timeout` and `tool_loop`) before the stream ends.

## Agent sessions

Multi-turn conversation history (A5). Each `POST /v1/agent/ask` call that
includes a `session_id` appends to an existing session; omitting it mints
a new session uuid, returned in the SSE event `session` as `{"sessionId": "<uuid>"}` in the JSON payload.

| Method | Path                                    | Min role | Effect                                                            |
| ------ | --------------------------------------- | -------- | ----------------------------------------------------------------- |
| GET    | `/v1/agent/sessions`                    | Read     | List sessions, newest first (max 200). Returns `{sessions:[…]}`. |
| GET    | `/v1/agent/sessions/{session_id}`       | Read     | Fetch session metadata (title, rolling_summary, source, turn count). |
| GET    | `/v1/agent/sessions/{session_id}/turns` | Read     | Full turn log ordered by `turn_index`. Returns `{turns:[…]}`.    |
| DELETE | `/v1/agent/sessions/{session_id}`       | Read     | Delete session + cascade turns; detaches linked runs.             |

Session list item shape: `{session_id, title, created_at, last_active, source, turn_count}`.

Turn item shape: `{turn_index, role, content, run_id, created_at}`.

`{session_id}` must be a uuid; non-uuid values return `400`. Missing sessions return `404`.

## Engine config

One global engine preference row drives all agent runs. See
[Agent engines](/agent/engines) for provider setup and model selection.

| Method | Path                     | Min role | Content-Type       | Effect                                                                  |
| ------ | ------------------------ | -------- | ------------------ | ----------------------------------------------------------------------- |
| GET    | `/v1/agent/engines`      | Read     | —                  | Available providers + their model menus, plus the active config.        |
| GET    | `/v1/agent/engine`       | Read     | —                  | The persisted `EngineConfig` (one row per installation).                |
| PUT    | `/v1/agent/engine`       | Read     | `application/json` | Persist a new `EngineConfig`. Echoes the saved value.                   |
| POST   | `/v1/agent/engine/test`  | Read     | `application/json` | Probe a candidate config without persisting. Returns `{ok, kind, model}` or `502/504`. |

`EngineConfig` JSON shape:

```json
{
  "kind": "anthropic",
  "model": "claude-opus-4-7",
  "credential_id": "<uuid or null>",
  "host": "<url or null>",
  "extras": {}
}
```

`kind` ∈ `anthropic`, `openai`, `ollama`, `claude_cli`.
`host` is only relevant for `ollama` (the Ollama base URL).
`credential_id` references an encrypted credential from `/v1/credentials`.
The test endpoint fires a single-token probe (30 s timeout); the real
engine+credentials are used for Anthropic/OpenAI/Ollama; the `claude` CLI
binary is probed directly for the `claude_cli` kind.

## Agent skills

| Method | Path                         | Min role | Effect                                                                     |
| ------ | ---------------------------- | -------- | -------------------------------------------------------------------------- |
| GET    | `/v1/agent/skills`           | Read     | List all discovered skills on the host with metadata and enabled state.    |
| GET    | `/v1/agent/skills/enabled`   | Read     | The toggled set — just the names.                                          |
| PUT    | `/v1/agent/skills/enabled`   | Read     | Replace the toggled set. Body: `{skills: ["name", …]}`.                   |

## Memory

The memory surface stores and retrieves durable agent memories. See
[Agent memory](/agent/memory) for knob meanings and the memory model.

| Method | Path                              | Min role | Effect                                                                        |
| ------ | --------------------------------- | -------- | ----------------------------------------------------------------------------- |
| POST   | `/v1/agent/memory/query`          | Read     | Hybrid recall — semantic + keyword + graph expansion.                         |
| GET    | `/v1/agent/memory/overview`       | Read     | Aggregate stats: memory count, firehose rows, recent pipeline runs.           |
| GET    | `/v1/agent/memory/settings`       | Read     | Current tunable settings (consolidation knobs, dreaming schedule).            |
| PUT    | `/v1/agent/memory/settings`       | Read     | Persist tunable settings.                                                     |
| GET    | `/v1/agent/memory/export`         | Read     | Full snapshot (`{memory_nodes, memory_edges}`) for backup or portability.    |
| POST   | `/v1/agent/memory/import`         | Write    | Bulk-apply node/edge rows from another instance's export/changes response.    |
| GET    | `/v1/agent/memory/changes`        | Read     | Incremental sync: nodes/edges updated strictly after `?since=<RFC3339>`.     |

`POST /v1/agent/memory/query` body:

```json
{
  "query": "payments latency investigation",
  "realms": [],
  "memory_type": null,
  "tags": [],
  "importance_min": null,
  "as_of": null,
  "include_invalidated": false,
  "limit": null,
  "expand_hops": null,
  "mode": "fast"
}
```

`mode` is `"fast"` (default, LLM-free graph-aware hybrid retrieval) or
`"agentic"` (adds a short synthesized `brief` from the retrieved context).
All fields except `query` are optional.

`GET /v1/agent/memory/changes` query params: `since=<RFC3339>` (omit for full
epoch), `realm=<name>`. The response includes a `until` watermark to advance
on the next poll.

`POST /v1/agent/memory/import` requires `Role::Write` even though the agent
router is otherwise mounted at `Role::Read`. Body: `{memory_nodes:[…], memory_edges:[…]}`.

## Dreaming

On-demand and scheduled agentic memory consolidation runs. See
[Dreaming](/agent/dreaming) for the full pipeline description.

| Method | Path                                  | Min role | Effect                                                                               |
| ------ | ------------------------------------- | -------- | ------------------------------------------------------------------------------------ |
| POST   | `/v1/agent/memory/dreaming/run`       | Read     | Enqueue a manual dreaming run. `202` when queued; `200 {deduped:true}` when one is already in flight. |
| GET    | `/v1/agent/memory/dreaming/runs`      | Read     | List runs, newest first. Filters: `kind=`, `limit=` (max 200), `offset=`.           |
| GET    | `/v1/agent/memory/dreaming/runs/{id}` | Read     | Fetch one run including live `progress` JSONB for in-flight runs.                   |

`POST /v1/agent/memory/dreaming/run` optional body:

```json
{ "mode": null, "focus": null }
```

`mode` and `focus` are optional strings passed to the dreaming engine.
Response on success: `202 {job_id: "<uuid>"}`.
Response when already in flight: `200 {job_id: null, deduped: true, detail: "…"}`.

Run item shape includes: `id`, `kind`, `status`, `mode`, `trigger`,
`engine`, `model`, `started_at`, `finished_at`, `events_scanned`,
`memories_written`, `error`, `job_id`, `session_id`, `agent_run_id`,
`worker_id`, `stats`, `progress`.

## Dashboards

| Method | Path                    | Min role | Effect                                                            |
| ------ | ----------------------- | -------- | ----------------------------------------------------------------- |
| GET    | `/v1/dashboards`        | Read     | List all dashboards.                                              |
| POST   | `/v1/dashboards`        | Write    | Create a dashboard. Body: `{name, description?, panels?}`.        |
| GET    | `/v1/dashboards/{id}`   | Read     | Fetch a dashboard with all panels.                                |
| PATCH  | `/v1/dashboards/{id}`   | Write    | Update name / description, or atomically replace the panel set.   |
| DELETE | `/v1/dashboards/{id}`   | Write    | Delete the dashboard and its panels.                              |

`{id}` is a uuid. `404` for missing ids.

## Cleanup (hard-delete GC)

| Method | Path                                                 | Min role | Effect                                                                              |
| ------ | ---------------------------------------------------- | -------- | ----------------------------------------------------------------------------------- |
| POST   | `/v1/database/{db}/table/{table}/cleanup`            | Write    | Hard-delete soft-deleted extents whose `deleted_at` is strictly before `?before=`.  |

Required query param: `before=<RFC3339>` (e.g.
`2025-01-01T00:00:00Z`). Response body is the aggregate count:
`{extents_deleted, rows_freed, bytes_freed}`.

## Data sources

The data source admin surface manages periodic / scheduled data sources. The
catalog (`GET /v1/data-sources/catalog`) is what drives the UI's source picker.

| Method | Path                                  | Min role | Effect                                                                                  |
| ------ | ------------------------------------- | -------- | --------------------------------------------------------------------------------------- |
| GET    | `/v1/data-sources/catalog`              | Write    | The vendor catalog (available + coming-soon) that powers the add-data-source UI.          |
| POST   | `/v1/data-sources`                      | Write    | Create a data source. Body: `{name, type, target_database, target_table, schedule_ms, config}`. |
| GET    | `/v1/data-sources`                      | Write    | List data sources (id, name, type, enabled).                                              |
| GET    | `/v1/data-sources/{id}`                 | Write    | Read a data source. Secret-shaped fields in `config` are scrubbed to `***`.               |
| PATCH  | `/v1/data-sources/{id}`                 | Write    | Update name / schedule / enabled / config.                                              |
| DELETE | `/v1/data-sources/{id}`                 | Write    | Delete the data source.                                                                   |
| POST   | `/v1/data-sources/{id}/pause`           | Write    | Disable the data source (`disabled_reason="manual"`).                                     |
| POST   | `/v1/data-sources/{id}/resume`          | Write    | Re-enable.                                                                              |
| POST   | `/v1/data-sources/{id}/trigger`         | Write    | Enqueue an immediate tick. Returns `202 Accepted`.                                      |

`schedule_ms` must be in the range `[100, 86400000]` (100 ms → 24 h).
See [Data sources](/data-sources/) for the typed config shape per
data source type.

## Credentials

Typed, encrypted secrets referenced by data sources (and other subsystems) via a
`credential_id` in their config. List/get return a masked **preview**, never
plaintext. Requires `KYMA_SECRET_KEY`. See [OAuth data sources](/data-sources/oauth).

| Method | Path                     | Min role | Effect                                                                          |
| ------ | ------------------------ | -------- | ------------------------------------------------------------------------------- |
| POST   | `/v1/credentials`        | Write    | Create a credential. Body: `{label, value:{kind, …}, metadata?}`. Returns a summary. |
| GET    | `/v1/credentials`        | Write    | List credentials (label, kind, masked preview, metadata).                       |
| GET    | `/v1/credentials/{id}`   | Write    | Fetch one credential summary (no plaintext).                                     |
| DELETE | `/v1/credentials/{id}`   | Write    | Delete a credential.                                                            |

`value.kind` ∈ `pat`, `basic`, `oauth2`, `url`, `aws_creds`, `api_key`,
`github_app`.

## OAuth

The browser flow that mints `oauth2` credentials for OAuth data sources. The
callback is **unauthenticated** (a cross-site IdP redirect carries no bearer); the
single-use `state` token is the trust anchor. See [OAuth data sources](/data-sources/oauth).

| Method | Path                                | Min role | Effect                                                                                       |
| ------ | ----------------------------------- | -------- | -------------------------------------------------------------------------------------------- |
| POST   | `/v1/oauth/{provider}/start`        | Write    | Begin a flow. Body: `{data_source_type, scopes?, label?, client_id?, client_secret?}`. Returns `{authorize_url, state}`. |
| GET    | `/v1/oauth/{provider}/callback`     | _none_   | IdP redirect target. Exchanges the code, mints the credential, posts the result back to the opener. |
| GET    | `/v1/oauth/flows/{state}`           | Write    | Poll a flow's `{status, credential_id}` — the fallback when `postMessage` can't reach the UI.  |

`{provider}` ∈ `google`, `notion`, `atlassian`, `slack`.

## Worker fabric

The distributed compute fabric that runs data source syncs, dreaming jobs, and
arbitrary scheduled work. See [Workers](/agent/workers) for daemon setup and
job lifecycle details.

Two auth surfaces — worker-facing endpoints authenticate with a worker token
(`Authorization: Bearer kyw_…`); operator endpoints use the regular bearer
token.

### Worker-facing (Authorization: Bearer kyw\_…)

| Method | Path                          | Effect                                                                                              |
| ------ | ----------------------------- | --------------------------------------------------------------------------------------------------- |
| POST   | `/v1/workers/register`        | Register or re-register this worker. Returns `{worker_id, heartbeat_interval_secs:30, lease_secs}`. |
| POST   | `/v1/workers/heartbeat`       | Touch liveness. Returns `{status, drain, recommended_poll_ms:1000}`.                               |
| POST   | `/v1/jobs/claim`              | Claim up to `max` jobs matching `kinds`. Long-polls up to `wait_ms` (capped at 25 000 ms).        |
| POST   | `/v1/jobs/self`               | Enqueue a job pinned to this worker (e.g. a local-file source sync).                               |
| POST   | `/v1/jobs/{id}/progress`      | Snapshot partial progress JSON for a held job.                                                      |
| POST   | `/v1/jobs/{id}/complete`      | Mark a job done and supply a result payload.                                                        |
| POST   | `/v1/jobs/{id}/fail`          | Mark a job failed with an error string.                                                             |
| POST   | `/v1/jobs/{id}/lease`         | Extend the job lease (10–3 600 s). Call before the lease expires to avoid a reclaim.               |

Claim / lease lifecycle: `POST /v1/jobs/claim` atomically assigns jobs and starts their lease
(`KYMA_FABRIC_LEASE_SECS`, default 300 s). The worker must call `/v1/jobs/{id}/lease` before
the lease expires to retain ownership; otherwise the job is re-queued. `/progress` accepts any
JSON snapshot and stores it on the job row for operator visibility. `complete` and `fail` both
return `204`; `409` means the lease was lost.

`POST /v1/jobs/claim` body:

```json
{ "kinds": ["data_source_sync"], "max": 1, "wait_ms": 5000 }
```

### Operator (regular bearer, Role::Write)

| Method | Path                  | Min role | Effect                                                                                            |
| ------ | --------------------- | -------- | ------------------------------------------------------------------------------------------------- |
| POST   | `/v1/workers`         | Write    | Create a worker identity and mint its token. Body: `{name, capabilities?, labels?}`. Returns `{worker_id, token}` — token shown once. |
| GET    | `/v1/workers`         | Write    | List all registered workers.                                                                      |
| DELETE | `/v1/workers/{id}`    | Write    | Revoke a worker (soft-delete; outstanding jobs are re-queued on next sweep).                     |
| POST   | `/v1/jobs`            | Write    | Enqueue a job manually. Body is an `EnqueueJob` struct.                                          |
| GET    | `/v1/jobs`            | Write    | List jobs. Query params: `kind=`, `status=`, `limit=` (max 500, default 50).                     |
| GET    | `/v1/jobs/{id}`       | Write    | Fetch a single job with payload and progress.                                                     |

## Graph

Read-only property-graph surface. The built-in `schema` graph renders the
catalog (databases → tables → inferred column references) as a property
graph. Additional stored graphs are registered via the catalog and addressed
by name with an `X-Database` header. For graph KQL queries, see the
[Graph guide](/query/graph).

All graph routes accept an optional `X-Database` header that selects the
database context for stored-graph resolution and per-database token scope
enforcement.

| Method | Path                                    | Min role | Effect                                                                        |
| ------ | --------------------------------------- | -------- | ----------------------------------------------------------------------------- |
| GET    | `/v1/graph`                             | Read     | List available graphs (`name`, `kind`, `description`). Include `X-Database` to show stored graphs in that database. |
| GET    | `/v1/graph/{graph}/overview`            | Read     | Nodes + edges for the graph. Query params: `realm=`, `limit=` (default 800). |
| GET    | `/v1/graph/{graph}/stats`               | Read     | Node/edge counts. Query param: `realm=`.                                      |
| GET    | `/v1/graph/{graph}/schema`              | Read     | Label vocabulary and edge-type vocabulary.                                    |
| GET    | `/v1/graph/{graph}/nodes/{id}`          | Read     | Fetch one node by id. `404` when absent.                                      |
| GET    | `/v1/graph/{graph}/nodes/{id}/subgraph` | Read     | Ego subgraph around a node. Query param: `depth=` (default 2).                |
| POST   | `/v1/graph/{graph}/neighbors`           | Read     | Expand one or more nodes one hop.                                             |
| POST   | `/v1/graph/{graph}/search`              | Read     | Full-text + label search over the graph.                                      |

`{graph}` is `"schema"` for the built-in schema graph, or the registered
name of a stored graph.

`POST /v1/graph/{graph}/neighbors` body:

```json
{
  "node_ids": ["default::orders"],
  "direction": "both",
  "only_internal": false,
  "limit": 200
}
```

`direction` ∈ `"forward"`, `"backward"`, `"both"` (default).

`POST /v1/graph/{graph}/search` body:

```json
{
  "text": "payments",
  "labels": [],
  "realm": null,
  "limit": 20,
  "offset": 0
}
```

## MCP over HTTP

The Model Context Protocol surface exposes kyma's query and memory tools as
JSON-RPC 2.0 methods so any MCP-capable agent can interact with the engine.

| Method | Path       | Min role | Content-Type       | Effect                                       |
| ------ | ---------- | -------- | ------------------ | -------------------------------------------- |
| POST   | `/mcp/v1`  | Read     | `application/json` | JSON-RPC 2.0 channel (Streamable HTTP MCP).  |
| GET    | `/mcp/v1`  | Read     | —                  | SSE upgrade for streaming MCP sessions.      |

Auth is the same bearer token as `/v1/*` (`Role::Read`). Scoped tokens
(per-database) are blocked at the middleware level because MCP tools address
databases internally.

For a stdio alternative, `kyma mcp` (the CLI subcommand) exposes the same
tools without an HTTP server. See the [MCP reference](/reference/mcp) for
tool names, input schemas, and capability negotiation.

## Arrow Flight (gRPC)

Separate port — default `:9090`, configurable via `KYMA_GRPC_ADDR`
(set to `off` to disable). Auth on the gRPC surface is not yet
enforced; treat it as deployment-firewalled.

The Flight `do_get` ticket is a JSON envelope:

```json
{ "database": "default", "query": "SELECT ...", "language": "sql" }
```

`language` is `"sql"` (default) or `"kql"`. The server returns a
streaming sequence of `FlightData` messages — Arrow IPC framing,
zero-copy from kyma's columnar buffers to the wire. See
[Arrow Flight](/query/arrow-flight) for client examples.

`handshake`, `get_flight_info`, `get_schema`, `do_put`, `do_exchange`,
`do_action`, `list_actions`, `list_flights` return `Unimplemented`
for now.

When the engine is built with the `web-ui` feature, Flight is also
nested under the HTTP router at `/flight/*` as gRPC-web (auth on,
`Role::Read`). Browsers reach the same surface without a separate
gRPC client.

## OTLP gRPC (logs)

Separate gRPC server on `KYMA_OTLP_ADDR` (default `off`;
conventional port `4317`). Phase A is logs-only; received batches
land in `KYMA_OTLP_DATABASE` (default `default`) in a fixed
`otel_logs` table. See [OTLP gRPC](/ingest/otlp-grpc).
