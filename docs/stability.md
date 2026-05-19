# kyma Stability Contract

> **Status:** in force from `v1.0.0`. Until then, this document is the
> contract kyma is being hardened to meet. Any change to a "frozen" surface
> below requires the deprecation policy at the bottom of this file.

This document names every surface kyma promises not to break across minor
versions in the v1.x series, and the deprecation policy that governs
intentional changes.

If a surface is not listed here, it is not part of the v1.0 contract — it
may change in any minor release.

---

## 1. HTTP REST API

The following endpoints are part of the v1.0 frozen surface. Request and
response shapes are fixed at v1.0.0; new optional fields may be added; no
field may change meaning or be removed without going through the deprecation
policy in section 10.

### `GET /health` — liveness probe

**Frozen.**

- Headers: none required.
- Request body: none.
- Response: `200 OK` with JSON body `{ "status": "ok", "version": "<semver>" }`. Never gated behind auth.
- Stable error codes: none (this endpoint does not return error bodies).

### `GET /metrics` — Prometheus scrape

**Frozen.**

- Headers: none required.
- Request body: none.
- Response: `200 OK` with `Content-Type: text/plain; version=0.0.4` in Prometheus text exposition format v0.0.4. Metric names are governed by `docs/metrics-taxonomy.md`.
- Stable error codes: none (returns `503 Service Unavailable` with plain-text body if the recorder is not installed, which is an internal process error).

### `POST /v1/ingest` — ingest NDJSON rows into a table

**Frozen.**

- Headers:
  - `X-Database: <string>` (optional; defaults to `default`)
  - `X-Table: <string>` (required; target table name)
  - `X-Idempotency-Key: <string>` (optional; if present, duplicate requests with the same key are replayed from the ledger)
  - `X-Auto-Create: true|false` (optional; default `true`; when `false`, the table must already exist)
  - `X-Schema-Evolve: true|false` (optional; default `true`; when `false`, unknown fields are dropped silently)
  - `X-Request-ID: <uuid>` (optional; if absent, one is generated and returned)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: NDJSON — one JSON object per line; `Content-Type: application/x-ndjson`. Body limit 64 MiB.
- Response: `200 OK` with JSON body:
  ```json
  {
    "snapshot_id": "<uuid>",
    "extent_count": 1,
    "rows_ingested": 123,
    "bytes_written": 4567,
    "replayed": false
  }
  ```
  Response header `X-Request-ID` echoes the request id.
- Stable error codes: `missing_table_header`, `body_too_large`, `bad_request_body`, `table_not_found`, `ensure_table_failed`, `schema_evolve_failed`, `ingest_failed`.

### `POST /v1/admin/databases/{database}/tables/{table}` — ensure table exists

**Frozen.**

- Headers:
  - `X-Request-ID: <uuid>` (optional; generated if absent)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `200 OK` with JSON body:
  ```json
  {
    "database": "<name>",
    "table": "<name>",
    "columns": [
      { "name": "<col>", "arrow_type": "<type>", "nullable": true }
    ],
    "rows": 0
  }
  ```
- Stable error codes: `ensure_table_failed`.

### `GET /v1/admin/databases/{database}/tables/{table}` — get table metadata

**Frozen.**

- Headers:
  - `X-Request-ID: <uuid>` (optional; generated if absent)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `200 OK` with JSON body same shape as `POST /v1/admin/databases/{database}/tables/{table}` above.
- Stable error codes: `table_not_found`.

### `POST /v1/query` — execute a SQL or KQL query

**Frozen.**

- Headers:
  - `X-Database: <string>` (optional; defaults to `default`)
  - `Content-Type: application/sql` (default) or `Content-Type: application/x-kql` (for KQL queries)
  - `X-Kyma-Max-Wall-Clock-Ms: <uint>` (optional; wall-clock budget in milliseconds, minimum 10)
  - `X-Kyma-Max-Memory-Bytes: <uint>` (optional; memory budget in bytes, minimum 1 MiB)
  - `X-Kyma-Max-Object-Store-Bytes: <uint>` (optional; object-store scan budget in bytes)
  - `X-Request-ID: <uuid>` (optional; generated if absent)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: SQL statement (plain text) or KQL query text, depending on `Content-Type`. Body limit 16 MiB.
- Response: `200 OK` with `Content-Type: application/x-ndjson; charset=utf-8` — one JSON object per result row, one row per line. Response headers include `X-Kyma-Rows: <count>` and `X-Request-ID`.
- Stable error codes: `body_too_large`, `bad_encoding`, `empty_query`, `kql_parse_error`, `database_not_found`, `database_empty`, `sql_parse_error`, `memory_exceeded`, `wall_clock_exceeded`, `query_execution_error`, `serialization_error`, `internal`.

### `GET /v1/catalog/schema` — list all databases, tables, and columns

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: none.
- Response: `200 OK` with JSON body:
  ```json
  {
    "databases": [
      {
        "name": "<db>",
        "tables": [
          {
            "name": "<table>",
            "columns": [{ "name": "<col>", "data_type": "<type>", "nullable": true }]
          }
        ]
      }
    ]
  }
  ```
  Response is served from a server-side cache (default TTL 5 s, configurable via `KYMA_SCHEMA_CACHE_TTL_SECS`).
- Stable error codes: `catalog` (wrapped in `{ "error": { "code": "catalog", "message": "..." } }`).

### `GET /v1/dashboards` — list dashboards

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: none.
- Response: `200 OK` with JSON array of dashboard objects:
  ```json
  [
    {
      "id": "<uuid>",
      "name": "<string>",
      "description": "<string|null>",
      "time_range_preset": "<string>",
      "refresh_interval_seconds": "<int|null>",
      "created_at": "<RFC3339>",
      "updated_at": "<RFC3339>"
    }
  ]
  ```
- Stable error codes: `{ "error": "<message>" }` (catalog-level error, `500`).

### `POST /v1/dashboards` — create a dashboard

**Frozen.**

- Headers:
  - `Content-Type: application/json` (required)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: JSON object:
  ```json
  {
    "name": "<string>",
    "description": "<string|null>",
    "panels": [ { "...panel fields..." } ]
  }
  ```
  `panels` is optional. When present, panel objects must include: `title`, `panel_type`, `query` (optional), `database_name` (optional), `config` (JSON object), `grid_x`, `grid_y`, `grid_w`, `grid_h`, `display_order`. `id` is optional (generated if absent).
- Response: `201 Created` with full `DashboardWithPanels` JSON (dashboard fields + `"panels": [...]`).
- Stable error codes: `{ "error": "<message>" }` (catalog-level error, `500`).

### `GET /v1/dashboards/:id` — get a dashboard with panels

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: none.
- Response: `200 OK` with `DashboardWithPanels` JSON (all dashboard fields plus `"panels": [...]`).
- Stable error codes: `{ "error": "dashboard <id> not found" }` (`404`); `{ "error": "<message>" }` (`500`).

### `PATCH /v1/dashboards/:id` — update a dashboard

**Frozen.**

- Headers:
  - `Content-Type: application/json` (required)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: JSON patch object (all fields optional):
  ```json
  {
    "name": "<string>",
    "description": "<string|null>",
    "time_range_preset": "<string>",
    "refresh_interval_seconds": "<int|null>",
    "panels": [ { "...panel input fields..." } ]
  }
  ```
  When `panels` is present it atomically replaces all panels; absent means leave panels unchanged.
- Response: `200 OK` with updated `DashboardWithPanels` JSON.
- Stable error codes: `{ "error": "dashboard <id> not found" }` (`404`); `{ "error": "<message>" }` (`500`).

### `DELETE /v1/dashboards/:id` — delete a dashboard

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `204 No Content` on success.
- Stable error codes: `{ "error": "dashboard <id> not found" }` (`404`); `{ "error": "<message>" }` (`500`).

### `POST /v1/database/:db/table/:table/cleanup` — hard-delete soft-deleted extents

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Query parameters:
  - `before=<RFC3339 timestamp>` (required; hard-delete soft-deleted extents whose `deleted_at` is strictly before this timestamp)
- Request body: none.
- Response: `200 OK` with JSON body:
  ```json
  { "extents_deleted": 0, "rows_freed": 0, "bytes_freed": 0 }
  ```
- Stable error codes: `{ "error": "table '<db>'.'<table>' not found" }` (`404`); `{ "error": "<message>" }` (`500`).

### `POST /v1/connectors` — create a connector

**Frozen.**

- Headers:
  - `Content-Type: application/json` (required)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: JSON object:
  ```json
  {
    "name": "<string>",
    "type": "<connector_type_id>",
    "target_database": "<string>",
    "target_table": "<string>",
    "schedule_ms": 60000,
    "config": { "...connector-specific config..." }
  }
  ```
  `schedule_ms` must be in `[100, 86400000]`.
- Response: `201 Created` with JSON body `{ "id": "<uuid>" }`.
- Stable error codes: `{ "error": "unknown type <type>" }` (`400`); `{ "error": "<validation message>" }` (`400`); `{ "error": "schedule_ms must be in [100, 86400000]" }` (`400`); `{ "error": "<message>" }` (`500`).

### `GET /v1/connectors` — list connectors

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `200 OK` with JSON body:
  ```json
  { "items": [{ "id": "<uuid>", "name": "<string>", "type": "<string>", "enabled": true }] }
  ```
- Stable error codes: `{ "error": "<message>" }` (`500`).

### `GET /v1/connectors/:id` — get a connector

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `200 OK` with JSON body containing full connector detail: `id`, `name`, `type`, `target_database`, `target_table`, `schedule_ms`, `drive_model` (the connector's execution model; currently `"periodic"` for all connectors), `enabled`, `disabled_reason`, `last_run_at`, `last_success_at`, `last_error`, `last_rows_ingested`, `config` (secret fields scrubbed to `"***"`).
- Stable error codes: `404 Not Found` (no body); `{ "error": "<message>" }` (`500`).

### `PATCH /v1/connectors/:id` — update a connector

**Frozen.**

- Headers:
  - `Content-Type: application/json` (required)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: JSON patch object (all fields optional): `name`, `schedule_ms`, `enabled`, `config`.
- Response: `204 No Content` on success.
- Stable error codes: `{ "error": "schedule_ms must be in [100, 86400000]" }` (`400`); `{ "error": "<validation message>" }` (`400`); `404 Not Found` (no body); `{ "error": "<message>" }` (`500`).

### `DELETE /v1/connectors/:id` — delete a connector

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `204 No Content`.
- Stable error codes: `{ "error": "<message>" }` (`500`).

### `POST /v1/connectors/:id/pause` — pause a connector

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `204 No Content`.
- Stable error codes: none (errors are swallowed internally; the response is always `204`).

### `POST /v1/connectors/:id/resume` — resume a paused connector

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `204 No Content`.
- Stable error codes: none (errors are swallowed internally; the response is always `204`).

### `POST /v1/connectors/:id/trigger` — immediately trigger a connector run

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Write`)
- Request body: none.
- Response: `202 Accepted`.
- Stable error codes: none (enqueue errors are swallowed internally; the response is always `202`).

### `POST /mcp/v1` — MCP JSON-RPC channel

**Frozen.**

- Headers:
  - `Content-Type: application/json` (required)
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: a single JSON-RPC 2.0 request object or a batch array. Supported methods: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`.
- Response: `200 OK` with JSON-RPC 2.0 response object (or array for batch). `202 Accepted` with empty body for id-less notifications. Parse errors return a JSON-RPC error envelope with `id: null`.
- Stable error codes (JSON-RPC `error.code`): standard JSON-RPC 2.0 codes (`-32700` parse error, `-32601` method not found, `-32602` invalid params).

### `GET /mcp/v1` — MCP SSE keepalive (Streamable HTTP handshake)

**Frozen.**

- Headers:
  - `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`)
- Request body: none.
- Response: `200 OK` with `Content-Type: text/event-stream`. Emits a keepalive ping every 15 seconds. Clients that probe SSE before falling back to POST get a valid Streamable HTTP handshake.
- Stable error codes: none.

---

### Not in v1.0 (experimental / not yet frozen)

#### `POST /v1/agent/ask` — inline AI agent (experimental)

The inline agent surface is driven by an LLM and its behavior is expected to
evolve. It is not part of the v1.0 frozen surface.

- Request body: JSON `{ "question": "<string>", "database": "<string|null>", "include_thinking": false }`.
- Response: `text/event-stream` SSE. Events: `run_started`, `answer_delta`, `thinking_delta`, `tool_call`, `tool_result`, `answer_final`, `run_error`, `run_finished`.

#### `GET /v1/agent/runs/:run_id` — look up a past agent run (experimental)

Not frozen. Returns a JSON object with `run_id`, `question`, `model_id`,
`status`, `started_at`, `finished_at`, `usage`, `trace`.

#### `GET /` and `GET /assets/*path` — embedded web UI (web-ui feature only)

Static asset serving for the embedded SPA. Compiled in only with the
`web-ui` feature. Not part of the stability contract.

#### `GET /flight/*` — Arrow Flight over gRPC-web (web-ui feature only)

Available only when compiled with the `web-ui` feature. The Arrow Flight
gRPC API itself is covered separately in section 2. This HTTP transport
wrapper is not independently frozen.

## 2. Arrow Flight gRPC API

**Frozen.** The server exposes Arrow Flight on `:9090` by default (configurable via `KYMA_FLIGHT_ADDR`). The following surface is part of the v1.0 contract.

### Flight methods implemented

- `do_get(ticket)` — execute a query and stream results as Arrow RecordBatches. Ticket shape documented below. This is the only method clients need for querying.
- `handshake` — accepts unauthenticated connections; returns an empty response stream. No token exchange occurs. Auth hardening is planned for a future phase.

### Flight methods not in v1.0 (return `Unimplemented`)

- `list_flights` — not in v1.0; returns `Unimplemented` ("list_flights not supported; issue do_get with a JSON ticket").
- `get_flight_info` — not in v1.0; returns `Unimplemented` ("get_flight_info not supported; issue do_get directly").
- `get_schema` — not in v1.0; returns `Unimplemented` ("get_schema not supported").
- `do_put` — ingest-via-Flight not in v1.0; returns `Unimplemented` ("do_put not supported; use POST /v1/ingest for now").
- `do_action` — not in v1.0; returns `Unimplemented` ("do_action not supported").
- `list_actions` — not in v1.0; returns `Unimplemented` ("list_actions not supported").
- `do_exchange` — not in v1.0; returns `Unimplemented` ("do_exchange not supported").
- `poll_flight_info` — not in v1.0; returns `Unimplemented` ("poll_flight_info not supported").

### `do_get` ticket shape

The ticket bytes deserialize as UTF-8 JSON. There are two distinct ticket kinds, selected by the `kind` field.

#### Kind `"query"` (default) — user-facing query ticket

| Field | Type | Required | Meaning |
|---|---|---|---|
| `kind` | string | no | Must be `"query"` or omitted; defaults to `"query"`. |
| `database` | string | no | Target database name. Defaults to `"default"`. |
| `query` | string | no | The query text. Defaults to `""` (empty string yields an error). |
| `language` | string | no | Query language: `"sql"` (default) or `"kql"`. Any other value is rejected. |

#### Kind `"extent"` — internal node-to-node ticket (not a client-facing API)

This ticket kind is used by the read-fan-out router when a peer node fetches a raw extent from another node. It is part of the internal cluster protocol, not the external v1.0 client contract. It is documented here for completeness; client code must not construct `kind:"extent"` tickets.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `kind` | string | yes | Must be `"extent"`. |
| `database` | string | no | Database name containing the table. Defaults to `"default"`. |
| `table` | string | no | Table name to open. Defaults to `""`. |
| `object_path` | string | no | Object-store path of the extent file. Defaults to `""`. |
| `byte_size` | u64 | no | Declared byte size of the extent. Defaults to `0`. |

Unknown fields in either ticket kind are ignored by `serde` (no `deny_unknown_fields`). New optional fields may be added; existing fields' meanings are fixed.

### Actions

No actions are implemented in v1.0. `do_action` and `list_actions` both return `Unimplemented`.

### Schemas returned

Arrow schemas returned by `do_get` (kind `"query"`) mirror the table schemas registered in the catalog for the requested database. The schema header is emitted automatically by the `FlightDataEncoderBuilder` as the first `FlightData` frame before any data batches.

`ALTER TABLE ADD COLUMN` may extend a schema; an existing column's Arrow type never changes after the table is created.

### Error model

Errors are returned as gRPC `Status` codes. The status-code-to-meaning mapping for `do_get` is:

- `InvalidArgument` — the ticket bytes are not valid JSON; the `language` field is not `"sql"` or `"kql"`; the KQL query fails to parse; the SQL query fails to plan (DataFusion plan error).
- `NotFound` — the database does not exist or contains no tables (kind `"query"`); the catalog `lookup_table` call fails for the extent's database/table combination (kind `"extent"`).
- `Internal` — DataFusion runtime initialization failed; a table could not be registered into the session context; query execution failed after planning; Arrow Flight encoding failed; extent open, block listing, or block read failed (kind `"extent"`).
- `Unimplemented` — the called Flight method is not implemented in v1.0 (see list above).

## 3. KQL dialect

The KQL dialect that v1.0 supports is a defined subset of Kusto Query Language. Anything in this section is part of the v1.0 frozen contract; anything not in this section is "best-effort, not v1.0 surface" — the parser may accept it but the behavior is not contract.

The implementation is a direct-lowering KQL→SQL translator (`crates/kyma-kql/`). It does not build an AST; it streams parsed operators into a `QueryState` accumulator and renders SQL once. This means multi-operator compositions that require IR-level rewrites (e.g., `join`, `make-series`) are deferred to a future phase and are explicitly not in v1.0.

**Known doc-debt:** The project README (`README.md`) contains example KQL queries that use `tostring(...)` and `toint(...)` functions — 8 occurrences total across five query examples. These functions are NOT implemented in the v1.0 KQL parser and will be rejected with a `kql_parse_error` if executed. The README examples are illustrative of the *engine's intent* (extracting typed values from the dynamic `attributes` object), not the *parser's actual surface*. Until the README is updated, treat those queries as reference documentation rather than working code. See Task 4 of the F1 plan.

### Operators (frozen)

| Operator | Form | Notes |
|---|---|---|
| `where` | `T \| where <expr>` | Predicates may use any expression from the expression table below. Multiple `where` clauses AND together. |
| `project` | `T \| project col1, col2, ...` | Selects the named columns; any column not listed is dropped from the result. |
| `extend` | `T \| extend name = <expr>, ...` | Adds computed columns. Arithmetic, string, and time functions are usable in the expression. |
| `summarize` | `T \| summarize agg() [by col, ...]` | Aggregates rows. Optional `by` clause groups by one or more columns or bin-expressions. |
| `sort by` / `order by` | `T \| sort by col [asc\|desc], ...` | Sorts result. `asc`/`desc` keyword optional; default is `desc` (matching KQL semantics). Both `sort` and `order` are accepted as synonyms. |
| `take` / `limit` | `T \| take N` | Returns at most N rows. Both `take` and `limit` are accepted as synonyms. N must be a non-negative integer literal. |
| `top` | `T \| top N by <expr> [asc\|desc]` | Equivalent to `sort by <expr> [asc\|desc] \| take N`. |
| `count` | `T \| count` | Returns a single row with column `Count` holding the row count for the table or the result of earlier pipeline operators. |
| `distinct` | `T \| distinct col1, col2, ...` | Returns distinct combinations of the listed columns. When no columns are listed, returns distinct rows over all columns. |

### Operators explicitly not in v1.0

| Operator | Status | Notes |
|---|---|---|
| `project-away` | Parser accepts; behavior is best-effort | Lowered to `SELECT *`; excluded columns are not actually removed. Behavior diverges from Kusto. Freeze deferred. |
| `join` | Not implemented | Parser rejects with "unsupported operator". Deferred to Phase E.2 (unified-plan IR). |
| `make-series` | Not implemented | Parser rejects. Requires IR-level auto-fill semantics not yet present. |
| `mv-expand` | Not implemented | Parser rejects. Deferred. |
| `graph-traverse` | Parser accepts; not v1.0 | CTE-based recursive SQL emitted; semantics are non-standard relative to Kusto's native graph operators. No test coverage in `scripts/test-kql.sh`. |
| `graph-shortest-path` | Parser accepts; not v1.0 | Same status as `graph-traverse`. No test coverage. |
| `parse` | Not implemented | Parser rejects. |
| `evaluate` | Not implemented | Parser rejects. |
| `render` | Not implemented | Parser rejects. |

### Expressions (frozen)

All frozen operators accept the following expression forms:

| Category | Forms |
|---|---|
| Comparison | `==`, `!=`, `<`, `<=`, `>`, `>=` |
| Logical | `and`, `or`, `not` |
| Arithmetic | `+`, `-`, `*`, `/`, `%` |
| String predicates | `contains "s"`, `startswith "s"`, `endswith "s"`, `has "s"` |
| Range | `col between (low .. high)` — inclusive on both ends |
| Set membership | `col in (v1, v2, ...)` |
| Literals | Integer (`42`), float (`3.14`), string (`"x"` or `'x'`), bool (`true`/`false`), `null`, duration (`30s`, `5m`, `2h`, `7d`), datetime (`datetime(2026-04-19T10:00:00Z)`) |

Notes on string predicates: `contains`, `startswith`, `endswith` lower to SQL `LIKE`. `has` lowers to `LIKE '%needle%'` (not true word-boundary semantics). Case-sensitivity follows the underlying DataFusion/Arrow behavior (case-sensitive).

### Scalar functions (frozen)

| Function | Signature | Returns | SQL lowering |
|---|---|---|---|
| `now` | `now()` | Current timestamp | `now()` |
| `ago` | `ago(duration)` | Timestamp N duration before now | `(now() - INTERVAL 'N unit')` |
| `bin` | `bin(col, duration)` | Floor of col to bucket | `date_bin(bucket, col, TIMESTAMP '1970-01-01')` |
| `startofhour` | `startofhour(col)` | Timestamp truncated to hour | `date_trunc('hour', col)` |
| `startofday` | `startofday(col)` | Timestamp truncated to day | `date_trunc('day', col)` |
| `startofmonth` | `startofmonth(col)` | Timestamp truncated to month | `date_trunc('month', col)` |
| `datetime` | `datetime(literal)` | Timestamp value | `CAST(literal AS TIMESTAMP)` |
| `strcat` | `strcat(a, b, ...)` | Concatenated string | `concat(a, b, ...)` |
| `tolower` | `tolower(s)` | Lowercased string | `lower(s)` |
| `toupper` | `toupper(s)` | Uppercased string | `upper(s)` |
| `strlen` | `strlen(s)` | Length of string | `char_length(s)` |
| `isnull` | `isnull(x)` | Boolean — whether x is NULL | `(x IS NULL)` |
| `isnotnull` | `isnotnull(x)` | Boolean — whether x is not NULL | `(x IS NOT NULL)` |
| `iff` | `iff(cond, a, b)` | a if cond is true, else b | `(CASE WHEN cond THEN a ELSE b END)` |

### Aggregate functions (frozen)

Used inside `summarize`. All are used in the SQL aggregation context.

| Function | Signature | Notes |
|---|---|---|
| `count` | `count()` or `count(col)` | `count(*)` or `count(col)`. When used as a bare pipe operator (`T \| count`), emits `count(*) AS "Count"`. |
| `sum` | `sum(col)` | Sum of column values. |
| `avg` | `avg(col)` | Arithmetic mean. |
| `min` | `min(col)` | Minimum value. |
| `max` | `max(col)` | Maximum value. |
| `dcount` | `dcount(col)` | Distinct count; lowers to `count(DISTINCT col)`. |
| `dcountif` | `dcountif(col, cond)` | Conditional distinct count; lowers to `count(DISTINCT CASE WHEN cond THEN col ELSE NULL END)`. |

### Type names

KQL type casts use the `datetime(...)` function form. There are no explicit `toint`, `tostring`, etc. type-cast functions in v1.0 — the parser does not implement them and will reject them. The `datetime(x)` function is the only explicit type conversion in v1.0.

Duration literals are lexed as a native Duration token and lowered to SQL INTERVAL at parse time. They are not a type-cast function.

| Literal / form | Backing SQL type | Example |
|---|---|---|
| Integer literal | int64 | `42` |
| Float literal | float64 | `3.14` |
| String literal | UTF-8 string | `"hello"` or `'hello'` |
| Boolean literal | boolean | `true`, `false` |
| `null` | null | `null` |
| Duration literal | SQL INTERVAL | `30s`, `5m`, `2h`, `7d` |
| `datetime(x)` | TIMESTAMP | `datetime("2026-04-19T10:00:00Z")` or `datetime("2026-04-19")` |

Note on `datetime`: the argument `x` is cast to TIMESTAMP via SQL's native type coercion. Both ISO 8601 formats (with `T` and `Z`) and space-separated date-time strings are accepted, depending on the underlying SQL engine's timestamp parser. Recommended: use ISO 8601 format with explicit `T` separator and timezone (e.g., `datetime("2026-04-19T10:00:00Z")`). Formats without timezone information may be interpreted in the server's local timezone.

### Error taxonomy

The KQL frontend (`crates/kyma-kql/`) produces a single error type: `ParseError(String)`. At the HTTP layer, a parse failure causes a `400 Bad Request` with a JSON error body using code `kql_parse_error`. At the Arrow Flight layer, a parse failure returns gRPC status `InvalidArgument`.

There is no structured sub-code within `ParseError` — the message string is free-form. The only stable error code at the API level is:

- `kql_parse_error` — emitted by `POST /v1/query` when `Content-Type: application/x-kql` and the KQL parser returns a `ParseError`. The `message` field contains the parser's free-form error string (table name not found as identifier, unsupported operator name, unsupported function name, unexpected token, unterminated string literal, bad duration unit, unexpected end of query, trailing tokens). Free-form message text is not contract.

The lexer emits `LexError` (converted to `ParseError` before surfacing) in these cases: unterminated double-quoted string, unterminated single-quoted string, dangling backslash in string, unexpected character.

### Tests proving the freeze

`scripts/test-kql.sh` exercises the following frozen operators end-to-end: `count` (bare), `where`, `project`, `take`, `summarize … by`, `sort by`, `extend`, `distinct`, plus the scalar functions `ago`, `now`, and string predicates `contains` and `startswith`. The back-compat workflow (Task 14) replays `scripts/test-kql.sh`-style queries against each tagged version's fixture.

**Operators frozen but not yet exercised by `scripts/test-kql.sh`** (gaps for A2 to close):

| Operator / Feature | Gap |
|---|---|
| `top N by` | No dedicated test case in `scripts/test-kql.sh`. Covered by unit tests in `crates/kyma-kql/src/parser.rs`. |
| `limit` (synonym for `take`) | Not explicitly tested; tested implicitly via `take`. |
| `order by` (synonym for `sort by`) | Not explicitly tested. |
| `bin(col, duration)` | Used only implicitly; no dedicated E2E test. |
| `startofhour`, `startofday`, `startofmonth` | Not tested in `scripts/test-kql.sh`. |
| `strcat`, `tolower`, `toupper`, `strlen` | Not tested in `scripts/test-kql.sh`. |
| `isnull`, `isnotnull` | Not tested in `scripts/test-kql.sh`. |
| `iff` | Not tested in `scripts/test-kql.sh`. |
| `sum`, `avg`, `min`, `max`, `dcount`, `dcountif` | Not tested in `scripts/test-kql.sh`. |
| `endswith`, `has` string predicates | Not tested in `scripts/test-kql.sh`. |
| `between` range expression | Not tested in `scripts/test-kql.sh` (covered by unit tests). |
| `in` set membership | Not tested in `scripts/test-kql.sh` (covered by unit tests). |

## 4. SQL dialect

_Filled in by Task 5._

## 5. MCP surface

_Filled in by Task 6._

## 6. Catalog Postgres schema

_Filled in by Task 7._

## 7. Configuration keys and environment variables

_Filled in by Task 8._

## 8. Extent on-disk format

_Filled in by Task 9. Final freeze blocked on P0 (format v1)._

## 9. Metrics, structured logs, internal traces

See [`docs/metrics-taxonomy.md`](metrics-taxonomy.md) for the rules. The
concrete metric/log/trace inventories are owned by the per-area specs
(A1–A4) and listed under each area's runbook.

## 10. Deprecation policy

_Filled in by Task 10. Mirrored in `CONTRIBUTING.md` by Task 11._
