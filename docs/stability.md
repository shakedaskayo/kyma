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

The SQL dialect that v1.0 supports is the DataFusion SQL subset at version `44.0.0` (pinned in workspace `Cargo.toml` as `datafusion = "44"`; resolved to `44.0.0` in `Cargo.lock`), minus the opt-outs below.

### What's included

The DataFusion SQL surface at the level documented at <https://datafusion.apache.org/user-guide/sql/index.html> for the pinned version, as filtered through kyma's thin HTTP and Flight query handlers. Both `POST /v1/query` and Arrow Flight `do_get` (language `"sql"`) call `SessionContext::sql()` directly — there is no kyma-level SQL parser or statement filter on either path (the agent's `run_sql` tool is the one exception; see opt-outs).

Specific guarantees (verified end-to-end in `scripts/e2e-test.sh`, `scripts/test-flight.sh`, `scripts/test-pushdown.sh`, `scripts/test-block-pruning.sh`, and `scripts/test-vectors.sh`):

- `SELECT` with column lists and `SELECT *`.
- `WHERE` with scalar predicates, comparison operators (`=`, `!=`, `<`, `<=`, `>`, `>=`), and boolean connectives (`AND`, `OR`, `NOT`).
- `WHERE` with timestamp range predicates: `col >= TIMESTAMP 'literal'`, `col < TIMESTAMP 'literal'`, `col BETWEEN TIMESTAMP 'a' AND TIMESTAMP 'b'`. These also engage kyma's extent-level and block-level pruning.
- `WHERE` with equality predicates and `IN (list)` on string/integer columns (also drives index-based extent pruning).
- `WHERE` with `LIKE` patterns (`%needle%`, `prefix%`, `%suffix`) on string columns (also drives text-token extent pruning).
- `GROUP BY` with one or more columns.
- `ORDER BY col [ASC|DESC]` with one or more keys.
- `LIMIT N`.
- `SELECT DISTINCT col, ...`.
- Aggregate functions: `COUNT(*)`, `COUNT(col)`, `COUNT(DISTINCT col)`, `SUM`, `AVG`, `MIN`, `MAX`.
- Scalar functions: `date_trunc('unit', col)`, `date_bin(interval, col, origin)` (used by KQL→SQL lowering), standard DataFusion scalar functions.
- `INNER JOIN` on equality conditions between two tables registered in the same database.
- Common table expressions (`WITH ... AS (...) SELECT ...`) — accepted by DataFusion's planner; confirmed working by the `is_read_only_sql` guard in the agent path which explicitly allows `WITH`-prefixed queries.
- `EXPLAIN` / `SHOW` — accepted by DataFusion's planner.
- kyma-specific UDFs (registered at session startup by `kyma_exec::register_vector_udfs`): `cosine_distance(a, b)`, `l2_distance(a, b)`, `inner_product(a, b)` — each accepts `FixedSizeList<Float32>` or `List<Float32>` arguments and returns `Float64`. Mismatched-length vectors return `NULL`.

Untested but likely working (DataFusion 44 includes these; not exercised through kyma's stack in current test scripts): window functions (`OVER (PARTITION BY ... ORDER BY ...)`), `LEFT JOIN`, `RIGHT JOIN`, `FULL OUTER JOIN`, `CROSS JOIN`, subqueries in `FROM` and `WHERE`, `UNION`/`INTERSECT`/`EXCEPT`. These are not part of the frozen surface because they have not been exercised end-to-end.

### Opt-outs (not in v1.0)

- **DDL statements** (`CREATE TABLE`, `DROP TABLE`, `ALTER TABLE`, `CREATE SCHEMA`, etc.) — kyma does not pre-filter DDL on the `POST /v1/query` or Arrow Flight `do_get` paths. DataFusion's `SessionContext::sql()` is called directly. In practice, DataFusion 44 operates on a read-only catalog (kyma registers `KymaTable` providers but does not register a mutable catalog); DDL statements that DataFusion tries to execute will fail at the execution layer with a DataFusion plan or execution error. At `POST /v1/query`, this surfaces as HTTP `500` with code `query_execution_error`; at Flight `do_get`, as gRPC status `Internal`. Behavior is not contractually defined — do not rely on DDL statements succeeding or failing with any specific error.

- **DML other than reads** (`UPDATE`, `DELETE`, `MERGE`, `INSERT INTO ... SELECT`) — same situation as DDL. kyma registers no writable `TableProvider` implementations; DataFusion will fail such statements at the execution layer. Response shape is the same as DDL failures above. Not part of the v1.0 contract.

- **`INSERT INTO ... VALUES`** — not supported. DataFusion may accept the syntax at the planning layer but kyma tables are read-only `TableProvider` implementations. Any insert attempt will fail at execution with a DataFusion internal error.

- **Transactions** (`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`) — DataFusion 44 does not support transaction-control statements. Any such statement is rejected by DataFusion at the planning stage. At `POST /v1/query` this surfaces as HTTP `400` with code `sql_parse_error`; at Flight `do_get`, as gRPC status `InvalidArgument`.

- **Runtime user-defined functions** — UDF registration is not exposed via any API surface. The only UDFs available are the three vector-distance functions compiled in at startup (`cosine_distance`, `l2_distance`, `inner_product`). There is no endpoint or mechanism to register additional UDFs at runtime.

- **Agent `run_sql` tool** — the inline AI agent's `run_sql` tool applies an additional pre-DataFusion filter (`is_read_only_sql`): only queries whose trimmed text begins with `SELECT`, `SHOW`, `EXPLAIN`, or `WITH` (case-insensitive) are forwarded to DataFusion. All other statements are rejected before reaching DataFusion with a JSON-level error `{"error": "only SELECT / SHOW / EXPLAIN supported"}`. This filter applies only to the agent tool path, not to `POST /v1/query` or Arrow Flight.

### DataFusion version policy

v1.0 ships against DataFusion `44.0.0`. v1.x minor releases may upgrade DataFusion only if the new version is fully back-compat against the queries in `scripts/backcompat-queries.txt` and any SQL queries used in `scripts/test-flight.sh`. R1 (release engineering) owns this policy.

### Error taxonomy

Stable error codes returned by the SQL frontend:

HTTP (`POST /v1/query`):
- `sql_parse_error` — DataFusion rejected the SQL at parse or plan time (returned by `ctx.sql()`); HTTP `400 Bad Request`.
- `query_execution_error` — DataFusion failed at execution time (returned by `df.collect()`); HTTP `500 Internal Server Error`.
- `memory_exceeded` — DataFusion's `GreedyMemoryPool` was exhausted during execution (error message contains `"ResourcesExhausted"` or `"Resources exhausted"`); HTTP `429 Too Many Requests` with `Retry-After: 1` and `X-Kyma-Budget-Limit` header.
- `wall_clock_exceeded` — query did not complete within the `X-Kyma-Max-Wall-Clock-Ms` budget; HTTP `429 Too Many Requests`.
- `serialization_error` — Arrow-to-NDJSON serialization failed post-execution; HTTP `500 Internal Server Error`.

Arrow Flight (`do_get`, language `"sql"`):
- `InvalidArgument` — `ctx.sql()` returned a DataFusion plan error (SQL rejected at parse or plan time).
- `Internal` — DataFusion execute-stream or encoding failed.

### Tests proving the freeze

`scripts/test-flight.sh` exercises SQL via the Flight gRPC API (delegating to Rust integration tests tagged `--ignored` in `crates/kyma-server/tests/flight_smoke.rs`). `scripts/e2e-test.sh` exercises SQL via `POST /v1/query` with the queries listed under "What's included" above. `scripts/test-pushdown.sh`, `scripts/test-block-pruning.sh`, and `scripts/test-vectors.sh` exercise timestamp-range predicates, equality pushdown, and UDF calls respectively. The back-compat workflow (Task 17 in the F1 plan) replays SQL queries from `scripts/backcompat-queries.txt` against each tagged fixture.

## 5. MCP surface

The MCP server exposed by `kyma-mcp` is part of the v1.0 frozen surface. Agents that consume this surface can rely on tool names, argument schemas, and return shapes across the v1.x series.

### Transport

- HTTP POST: `POST /mcp/v1` — JSON-RPC 2.0 channel. Request body must be a single JSON-RPC 2.0 request object or a non-empty batch array. Notifications (id absent) return `202 Accepted` with empty body. Batch responses omit entries for pure notifications; if every item in a batch is a notification the whole response is `202 Accepted` with empty body.
- HTTP GET (SSE keepalive): `GET /mcp/v1` — `Content-Type: text/event-stream`. Emits a keepalive ping every 15 seconds. MCP clients that probe SSE before falling back to POST receive a valid Streamable HTTP handshake. The stream carries no tool results; it is purely a connection-maintenance channel.
- stdio: not supported. The server is exclusively HTTP-based.
- Auth: `Authorization: Bearer <token>` (required when auth is enabled; `Role::Read`). The same bearer-token model as the REST surface applies — `kyma-bin` wraps the MCP router with the `require_role_middleware(Role::Read)` layer. Unauthenticated requests to either endpoint return `401 Unauthorized`.

### Protocol version and capability negotiation

The server speaks MCP protocol version `2025-03-26` (per `initialize.rs`). The `initialize` method responds with:

```json
{
  "protocolVersion": "2025-03-26",
  "capabilities": {
    "tools": { "listChanged": false }
  },
  "serverInfo": {
    "name": "<server-name>",
    "version": "<semver>"
  }
}
```

If the client sends a matching `protocolVersion`, the server echoes it back. Otherwise it advertises its own version. Capabilities advertise tools support; resources, prompts, and sampling are not advertised.

### Methods (frozen)

Four JSON-RPC methods are dispatched:

- `initialize` — handshake. Params: `{ "protocolVersion": "<string>", "capabilities": {}, "clientInfo": { "name": "...", "version": "..." } }`. Returns the capability block above.
- `notifications/initialized` — post-handshake notification. No response (HTTP `202`).
- `tools/list` — enumerate all tools. No params. Returns `{ "tools": [ <tool-entry>, ... ] }`. Tool entries are `{ "name", "description", "inputSchema" }`.
- `tools/call` — invoke a tool. Params: `{ "name": "<tool-name>", "arguments": { ... } }`. Returns a tool-result envelope (see below).

Any other method name returns JSON-RPC error `-32601` (MethodNotFound).

### Tool-call return envelope

A successful `tools/call` always returns:

```json
{
  "content": [
    { "type": "text", "text": "<JSON string of result>" }
  ],
  "isError": false,
  "structuredContent": { "...result fields..." }
}
```

Tool-level errors (e.g. bad arguments, catalog failures, SQL execution failures) are surfaced as `{ "error": "<message>" }` inside `structuredContent` with `"isError": false` — the outer JSON-RPC call succeeds and the error is data for the model to self-correct. JSON-RPC-level errors (`ErrorObject`) are returned only for unknown tool names (`-32601`) or dispatcher-internal failures (`-32603`).

### Tools (frozen)

All eight tools are read-only (`with_read_only(true)`) and concurrency-safe (`with_concurrency_safe(true)`).

---

- **`list_databases`** — List every database in the kyma cluster. Call first to discover what databases exist.
  - Arguments (JSON Schema):
    ```json
    { "type": "object" }
    ```
    (no arguments — empty object or omit `arguments` key)
  - Returns: `{ "databases": ["<name>", ...] }` on success; `{ "error": "<message>" }` on catalog failure.
  - Behavior: calls `catalog.list_databases()` and returns the array of database name strings.

---

- **`describe_table`** — Describe the columns of a table: names, Arrow data types, nullability. Call before writing a SQL query against an unfamiliar table.
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database", "table"],
      "properties": {
        "database": { "type": "string", "description": "Database name." },
        "table":    { "type": "string", "description": "Table name inside that database." }
      }
    }
    ```
  - Returns:
    ```json
    {
      "database": "<name>",
      "table": "<name>",
      "columns": [
        { "name": "<col>", "type": "<Arrow type string>", "nullable": true }
      ]
    }
    ```
    On failure: `{ "error": "lookup_table: <message>" }`.
  - Behavior: calls `catalog.lookup_table(database, table)` and renders each Arrow field as a `{ name, type, nullable }` object. `type` is the `Debug` representation of the Arrow `DataType` enum (e.g. `"Utf8"`, `"Int64"`, `"Timestamp(Microsecond, None)"`).

---

- **`run_sql`** — Execute a read-only SQL query via DataFusion. Use `cosine_distance` / `l2_distance` UDFs for vector similarity. Returns up to `max_rows` rows as JSON. Queries that modify data are rejected (SELECT only; SHOW/EXPLAIN also allowed).
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database", "sql"],
      "properties": {
        "database": { "type": "string", "description": "Database whose tables should be registered into the DataFusion session for this query." },
        "sql":      { "type": "string", "description": "Full SQL query text. Only SELECT / SHOW / EXPLAIN are accepted." },
        "max_rows": { "type": "integer", "description": "Row cap applied to the JSON response. Default 200.", "default": 200 }
      }
    }
    ```
  - Returns:
    ```json
    {
      "columns": [{ "name": "<col>", "type": "<Arrow type string>" }],
      "rows": [{ "<col>": "<value>", ... }],
      "row_count": 123,
      "truncated": false
    }
    ```
    On failure: `{ "error": "<message>" }`. If the query does not begin with `SELECT`, `SHOW`, `EXPLAIN`, or `WITH` (case-insensitive), returns `{ "error": "only SELECT / SHOW / EXPLAIN supported" }` without reaching DataFusion.
  - Behavior: builds a fresh DataFusion `SessionContext`, registers all tables in `database`, applies the `is_read_only_sql` guard, executes the query, and streams results as JSON-serialized Arrow batches capped at `max_rows` (clamped to requested value; default 200). Memory hard-capped at 256 MiB per call via `GreedyMemoryPool`. Vector UDFs (`cosine_distance`, `l2_distance`, `inner_product`) are registered at session startup.

---

- **`run_kql`** — Execute a KQL query against kyma — the primary query tool. KQL is pipe-syntax: `requests | where status >= 500 | summarize n=count() by url | top 10 by n`.
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database", "kql"],
      "properties": {
        "database": { "type": "string", "description": "Database whose tables should be registered into the DataFusion session for this query." },
        "kql":      { "type": "string", "description": "KQL query text. Starts with a table reference, pipe-separated operators." },
        "max_rows": { "type": "integer", "description": "Row cap applied to the JSON response. Default 200.", "default": 200 }
      }
    }
    ```
  - Returns:
    ```json
    {
      "columns": [{ "name": "<col>", "type": "<Arrow type string>" }],
      "rows": [{ "<col>": "<value>", ... }],
      "row_count": 123,
      "truncated": false,
      "compiled_sql": "<the SQL lowered from KQL>"
    }
    ```
    On KQL parse error: `{ "error": "kql_parse: <message>", "hint": "Check pipe syntax..." }`. On execution failure: `{ "error": "<message>" }`.
  - Behavior: compiles KQL to SQL via `kyma_kql::kql_to_sql`, then delegates to the same `execute_sql` path as `run_sql`. The compiled SQL is attached to the result as `compiled_sql` for debugging. Memory hard-capped at 256 MiB per call. The KQL surface is governed by section 3 of this document.

---

- **`sample_rows`** — Fetch N representative rows from a table. Use when `describe_table`'s column types aren't enough to understand the data shape (e.g. JSON/dynamic columns, text formats).
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database", "table"],
      "properties": {
        "database": { "type": "string" },
        "table":    { "type": "string" },
        "n":        { "type": "integer", "description": "Number of rows to return. Default 5, clamped to [1, 1000].", "default": 5 }
      }
    }
    ```
  - Returns: same shape as `run_sql` — `{ "columns", "rows", "row_count", "truncated" }`. On failure: `{ "error": "<message>" }`. If `database` or `table` contain non-alphanumeric/underscore characters, returns `{ "error": "database and table must be ascii-alphanumeric / underscore only" }`.
  - Behavior: validates identifier safety, clamps `n` to `[1, 1000]`, issues `SELECT * FROM <database>.<table> LIMIT <n>` via `execute_sql`.

---

- **`explore_schema`** — Return the full schema of a database in one call: every table, every column, types, and a few sample values per column. Use first for any question that spans multiple tables or when entity relationships are unknown. More efficient than calling `list_databases` + `describe_table` per table.
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database"],
      "properties": {
        "database":           { "type": "string", "description": "Database whose schema graph to surface." },
        "samples_per_column": { "type": "integer", "description": "Number of sample values per column (cap 10; default 3).", "default": 3 }
      }
    }
    ```
  - Returns:
    ```json
    {
      "database": "<name>",
      "tables": [
        {
          "name": "<table>",
          "columns": [{ "name": "<col>", "type": "<Arrow type string>", "nullable": true }],
          "sample_values": { "<col>": ["<val>", ...], ... }
        }
      ],
      "table_count": 3,
      "hint": "Columns whose sample values look like ids (...) are likely foreign-key candidates..."
    }
    ```
    On failure: `{ "error": "<message>" }`.
  - Behavior: lists all tables via `catalog.list_tables_in_database`, runs `SELECT * LIMIT <samples_per_column>` per table to gather sample values (capped at 10 samples per column), and assembles the result in one response.

---

- **`find_references_to`** — Find every (database, table, column) where a given value appears in the catalog's distinct-value index. The relationship-traversal primitive — use when the user asks "what else references X?" or "where does X show up?". Returns up to 200 matches.
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["value"],
      "properties": {
        "database": { "type": ["string", "null"], "description": "Database to search within. Pass null or omit for cluster-wide search.", "default": null },
        "value":    { "type": "string", "description": "Value to locate across all columns." }
      }
    }
    ```
  - Returns:
    ```json
    {
      "value": "<searched value>",
      "matches": [
        { "database": "<db>", "table": "<table>", "column": "<col>" }
      ],
      "match_count": 5,
      "hint": "For each match, you can call run_kql to fetch the rows: `<table> | where <column> == \"<value>\"`"
    }
    ```
    On failure: `{ "error": "pg_query: <message>" }`.
  - Behavior: queries the Postgres `extents.column_stats` JSONB index directly (no full table scan). For string values, performs a `@>` containment check against the `distinct` array in each column's stats. For numeric-looking values, also attempts a numeric cast and tests for numeric containment. Results are limited to 200 rows, ordered by `(database_name, table_name, column_name)`.

---

- **`graph_traverse`** — Traverse a graph stored as edges in a kyma table. Wraps the KQL `graph-traverse` operator. Returns reachable nodes as `(node, depth)` pairs. Use for connectivity questions: "what services depend on X?", "which users trigger Y?".
  - Arguments (JSON Schema):
    ```json
    {
      "type": "object",
      "required": ["database", "edges_table", "source", "from_column", "to_column"],
      "properties": {
        "database":    { "type": "string" },
        "edges_table": { "type": "string", "description": "Table whose rows represent edges (each row has a source and target)." },
        "source":      { "type": "string", "description": "Starting node value." },
        "from_column": { "type": "string", "description": "Column in edges_table that holds the source node id." },
        "to_column":   { "type": "string", "description": "Column in edges_table that holds the target node id." },
        "max_hops":    { "type": "integer", "description": "Maximum hops. Default 5, clamped to [1, 20].", "default": 5 },
        "direction":   { "type": "string", "description": "\"forward\" (default), \"backward\", or \"both\".", "default": "forward" }
      }
    }
    ```
  - Returns: same `{ "columns", "rows", "row_count", "truncated" }` shape as `run_sql` (row cap 1000), plus:
    ```json
    {
      "compiled_sql": "<SQL from KQL lowering>",
      "compiled_kql": "<KQL string passed to kql_to_sql>"
    }
    ```
    On identifier validation failure: `{ "error": "edges_table / from_column / to_column must be ascii-alphanumeric / underscore only" }`. On invalid `direction`: `{ "error": "direction must be forward | backward | both" }`. On KQL compile failure: `{ "error": "kql_compile: <message>", "kql": "<kql>" }`.
  - Behavior: validates that `edges_table`, `from_column`, and `to_column` are safe identifiers (ASCII alphanumeric / underscore only), clamps `max_hops` to `[1, 20]`, builds a KQL `graph-traverse` expression, compiles it to SQL via `kyma_kql::kql_to_sql`, and executes via `execute_sql` with a 1000-row cap. Note: `graph-traverse` is not part of the v1.0 frozen KQL surface (section 3 lists it as "not v1.0"), but this tool's argument schema and return shape are frozen.

---

### Resources (frozen)

None in v1.0. The `resources/list` method (and any `resources/*` method) returns JSON-RPC error `-32601` (MethodNotFound). No URI templates are registered.

### Prompts (frozen)

None in v1.0. The `prompts/list` method (and any `prompts/*` method) returns JSON-RPC error `-32601` (MethodNotFound). No prompt templates are registered.

### Error model

MCP errors follow the JSON-RPC 2.0 error spec. The `error` field in a response is an object `{ "code": <integer>, "message": "<string>" }`. Error codes emitted:

- `-32700` (`ParseError`) — the request body is not valid JSON. Returned with `"id": null`.
- `-32600` (`InvalidRequest`) — the JSON-RPC envelope is structurally invalid: `jsonrpc` field is not `"2.0"`, the top-level value is neither object nor array, or the batch array is empty.
- `-32601` (`MethodNotFound`) — the requested method is not one of `initialize`, `notifications/initialized`, `tools/list`, `tools/call`. Also returned when `tools/call` names an unknown tool.
- `-32602` (`InvalidParams`) — `tools/call` was called without a `params` object, or without a `name` field inside params. Also returned if `initialize` params cannot be deserialized.
- `-32603` (`InternalError`) — a registered tool returned an ADK execution error (distinct from tool-level `{"error": "..."}` results; this is only raised if the tool's async execution itself panics or returns an Err).

Tool-level failures (bad arguments, catalog errors, SQL execution errors, KQL parse errors) are **not** JSON-RPC errors — they are returned as `{ "error": "<message>" }` values inside `structuredContent` with `"isError": false`. The outer JSON-RPC call succeeds with `200 OK`.

### Tests proving the freeze

`crates/kyma-mcp/tests/end_to_end.rs` — integration test `full_mcp_handshake_against_seeded_server` exercises the complete MCP lifecycle against a real TCP listener: `initialize` handshake, `notifications/initialized`, `tools/list` (asserts 8 tools), and `tools/call list_databases`. `rejects_request_without_bearer_token` asserts `401` when the bearer token is absent.

`crates/kyma-mcp/tests/jsonrpc_framing.rs` — wire-protocol tests covering: parse error for invalid JSON (`-32700`), invalid request for wrong JSON-RPC version (`-32600`), method not found for unknown method (`-32601`), invalid params for `tools/call` without `name` (`-32602`), batch with mixed results, batch of only notifications returning `202`, empty batch returning `InvalidRequest`.

`crates/kyma-mcp/src/tools_unit_tests.rs` — unit tests asserting `tools/list` returns exactly 8 named entries (`list_databases`, `describe_table`, `run_sql`, `run_kql`, `sample_rows`, `explore_schema`, `find_references_to`, `graph_traverse`) each with an `inputSchema` object and non-trivial description.

The back-compat workflow (Task 17) may add MCP smoke tests (`initialize` → `tools/list` → `tools/call list_databases`) against each tagged version's fixture in the future.

## 6. Catalog Postgres schema

The catalog schema is **forward-only** across the v1.x series. v1.x migrations may:

- Add new tables.
- Add new columns to existing tables (must be nullable or have a default).
- Add new indexes.
- Add new constraints that the existing data already satisfies.

v1.x migrations may **not**:

- Rename a table or a column.
- Drop a table or a column.
- Change the type of a column.
- Tighten a constraint in a way that requires data backfill.

These rules apply across the v1.x series. Breaking the schema requires the deprecation policy in section 10.

### Current tables (as of v1.0 freeze)

| Table | Purpose | Key columns | Notes |
|---|---|---|---|
| `databases` | Logical database registry | `id` (PK), `tenant_id`, `name` | UNIQUE `(tenant_id, name)`; parent of `tables` |
| `tables` | Table metadata; mirrors Iceberg table hierarchy | `id` (PK), `tenant_id`, `database_id` | FK → `databases`, `snapshots`, `schema_snapshots`; deferred FKs allow bootstrap insert |
| `schema_snapshots` | Immutable Arrow schema versions (JSON) per table | `id` (PK), `tenant_id`, `table_id` | FK → `tables` (deferred); referenced by `tables`, `snapshots`, `extents` |
| `snapshots` | Iceberg-style snapshot chain per table | `id` (PK), `tenant_id`, `table_id`, `sequence_number` | FK → `tables`, parent `snapshots`, `schema_snapshots`; UNIQUE `(table_id, sequence_number)` |
| `manifests` | Per-snapshot manifest groupings | `id` (PK), `tenant_id`, `snapshot_id` | FK → `snapshots`; `kind` IN `('data','delete','compaction')` |
| `extents` | Individual data-file metadata (object-store paths, stats) | `id` (PK), `tenant_id`, `table_id` | FK → `tables`, `manifests`, `schema_snapshots`; soft-delete via `deleted_at`; GIST + GIN + partial indexes |
| `nodes` | Cluster membership and heartbeat leases | `id` (PK), `lease_id` | Not tenant-scoped; `role` IN `('all_in_one','ingest','query','compaction')` |
| `ingest_ledger` | Idempotency records for POST /v1/ingest | `(tenant_id, idempotency_key)` (composite PK) | FK → `tables`, `snapshots`; TTL via `ttl_expires_at` |
| `background_tasks` | Distributed work queue (compaction, GC, connectors) | `id` (PK), `tenant_id` | FK → `tables`, `nodes`; `status` IN `('pending','claimed','done','failed')` |
| `connectors` | Ingestion-connector definitions | `id` (PK), `tenant_id`, `name` | UNIQUE `(tenant_id, name)`; `drive_model` IN `('periodic','continuous')`; KMS columns added in 007 |
| `connector_cursors` | Per-connector checkpoint/cursor state | `connector_id` (PK) | FK → `connectors`; one row per connector |
| `connector_leases` | Streaming-connector node leases (reserved for Slice 2) | `connector_id` (PK) | FK → `connectors`; unused in v1.0 |
| `dashboards` | Dashboard metadata | `id` (PK), `tenant_id` | No name-uniqueness constraint (names may repeat across tenants) |
| `dashboard_panels` | Individual panels within a dashboard | `id` (PK), `tenant_id`, `dashboard_id` | FK → `dashboards`; `panel_type` IN `('chart','table','markdown','stat')` |
| `column_metadata` | Per-column semantic annotations for the agent | `(tenant_id, database, table_name, column_name)` (composite PK) | Optional embedding link via `embedding_model_id` |
| `schema_embeddings` | Vector embeddings of table/column descriptions | `id` (PK), `tenant_id` | pgvector `vector(384)`; HNSW index; UNIQUE partial indexes per kind |
| `agent_runs` | Completed NL-query agent run records | `run_id` (PK), `tenant_id` | `status` IN `('success','error','budget_exceeded','cancelled','replay_miss')` |
| `agent_sessions` | Agent conversation session state | `session_id` (PK), `tenant_id` | Parent of `agent_session_turns` |
| `agent_session_turns` | Individual turns within an agent session | `(session_id, turn_index)` (composite PK), `tenant_id` | FK → `agent_sessions`, `agent_runs`; `role` IN `('user','assistant')` |
| `agent_replay_cache` | Deterministic replay cache for agent generate/run layers | `(tenant_id, cache_key)` (composite PK) | `layer` IN `('generate','run')`; `cache_key` is `BYTEA` (hash) |

### Per-table schema detail

#### `databases`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `name` | `TEXT` | no | none | Part of UNIQUE `(tenant_id, name)` (added in 007, replacing single-column UNIQUE) |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `databases_tenant_name_uniq` UNIQUE on `(tenant_id, name)` (replaces `databases_name_key` from 001). `databases_tenant_idx` on `(tenant_id)`.

---

#### `schema_snapshots`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `table_id` | `UUID` | no | none | FK → `tables(id)` ON DELETE CASCADE, DEFERRABLE INITIALLY DEFERRED |
| `arrow_schema` | `JSONB` | no | none | Serialized Arrow schema in JSON form |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `schema_snapshots_tenant_idx` on `(tenant_id)`.

---

#### `tables`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `database_id` | `UUID` | no | none | FK → `databases(id)` ON DELETE CASCADE |
| `name` | `TEXT` | no | none | Part of UNIQUE `(database_id, name)` |
| `current_snapshot_id` | `UUID` | yes | none | FK → `snapshots(id)` DEFERRABLE INITIALLY DEFERRED; NULL during bootstrap window only |
| `schema_snapshot_id` | `UUID` | yes | none | FK → `schema_snapshots(id)`; NULL during bootstrap window only |
| `config` | `JSONB` | no | `'{}'::jsonb` | Table-level configuration (retention, partitioning, etc.) |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `tables_tenant_idx` on `(tenant_id)`. UNIQUE `(database_id, name)`.

---

#### `snapshots`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `table_id` | `UUID` | no | none | FK → `tables(id)` ON DELETE CASCADE |
| `parent_id` | `UUID` | yes | none | FK → `snapshots(id)` (self-referential); NULL for snapshot #0 |
| `sequence_number` | `BIGINT` | no | none | Monotonically increasing per table; UNIQUE `(table_id, sequence_number)` |
| `schema_snapshot_id` | `UUID` | no | none | FK → `schema_snapshots(id)` |
| `summary` | `JSONB` | no | `'{}'::jsonb` | Operation metadata (e.g. `{ "operation": "bootstrap" }`) |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `snapshots_tenant_idx` on `(tenant_id)`. UNIQUE `(table_id, sequence_number)`.

---

#### `manifests`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `snapshot_id` | `UUID` | no | none | FK → `snapshots(id)` ON DELETE CASCADE |
| `kind` | `TEXT` | no | none | CHECK `kind IN ('data','delete','compaction')` |
| `extent_count` | `INTEGER` | no | `0` | |
| `byte_size` | `BIGINT` | no | `0` | |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `manifests_tenant_idx` on `(tenant_id)`.

---

#### `extents`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `table_id` | `UUID` | no | none | FK → `tables(id)` ON DELETE CASCADE |
| `manifest_id` | `UUID` | yes | none | FK → `manifests(id)` ON DELETE CASCADE |
| `schema_snapshot_id` | `UUID` | no | none | FK → `schema_snapshots(id)` |
| `object_path` | `TEXT` | no | none | Object-store path of the data file |
| `byte_size` | `BIGINT` | no | none | File size in bytes |
| `row_count` | `BIGINT` | no | none | Number of rows in the extent |
| `min_timestamp` | `TIMESTAMPTZ` | yes | none | Time-range pruning lower bound |
| `max_timestamp` | `TIMESTAMPTZ` | yes | none | Time-range pruning upper bound |
| `column_stats` | `JSONB` | no | `'{}'::jsonb` | Per-column distinct-value and token indexes for extent pruning |
| `present_paths` | `TEXT[]` | no | `'{}'::text[]` | Dynamic-path index for nested-field pruning |
| `bloom_filters` | `BYTEA` | yes | none | Reserved; not yet used |
| `compaction_gen` | `INTEGER` | no | `0` | Compaction generation counter |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `deleted_at` | `TIMESTAMPTZ` | yes | none | Soft-delete timestamp; NULL means live |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `extents_tbl_ts_range` GIST on `(table_id, tstzrange(min_timestamp, max_timestamp))` — time-range pruning. `extents_present_paths` GIN on `(present_paths)` — dynamic-path pruning. `extents_live` partial on `(tenant_id, table_id) WHERE deleted_at IS NULL` (recreated in 007 to include `tenant_id`). `extents_tenant_idx` on `(tenant_id)`.

---

#### `nodes`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `role` | `TEXT` | no | none | CHECK `role IN ('all_in_one','ingest','query','compaction')` |
| `endpoint` | `TEXT` | no | none | gRPC/HTTP address of this node |
| `capabilities` | `JSONB` | no | `'{}'::jsonb` | Feature flags and node-level metadata |
| `lease_id` | `UUID` | no | none | Rotating lease; heartbeat must match to update |
| `last_heartbeat` | `TIMESTAMPTZ` | no | `now()` | Updated by heartbeat calls |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |

Indexes: `nodes_last_heartbeat` on `(last_heartbeat)`. **Not tenant-scoped** — node identity is cluster-global.

---

#### `ingest_ledger`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `idempotency_key` | `TEXT` | no | none | Part of composite PK `(tenant_id, idempotency_key)` (PK changed in 007 from single-column) |
| `table_id` | `UUID` | no | none | FK → `tables(id)` ON DELETE CASCADE |
| `snapshot_id` | `UUID` | no | none | FK → `snapshots(id)` ON DELETE CASCADE |
| `rows_ingested` | `BIGINT` | no | none | |
| `bytes_written` | `BIGINT` | no | none | |
| `applied_at` | `TIMESTAMPTZ` | no | `now()` | |
| `ttl_expires_at` | `TIMESTAMPTZ` | no | none | Row expires after this timestamp; filtered on read |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration; part of composite PK |

Indexes: `ingest_ledger_ttl` on `(ttl_expires_at)`.

---

#### `background_tasks`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `kind` | `TEXT` | no | none | Task kind: `'compaction'`, `'retention_sweep'`, `'physical_gc'`, `'connector_tick'`, etc. |
| `table_id` | `UUID` | yes | none | FK → `tables(id)` ON DELETE CASCADE; NULL for non-table tasks |
| `payload` | `JSONB` | no | `'{}'::jsonb` | Task-specific parameters |
| `status` | `TEXT` | no | `'pending'` | CHECK `status IN ('pending','claimed','done','failed')` |
| `claimed_by` | `UUID` | yes | none | FK → `nodes(id)` ON DELETE SET NULL |
| `claim_expires_at` | `TIMESTAMPTZ` | yes | none | Dead-worker lease expiry |
| `priority` | `INTEGER` | no | `0` | Higher values are claimed first |
| `attempt` | `INTEGER` | no | `0` | Number of claim attempts made |
| `max_attempts` | `INTEGER` | no | `3` | Maximum allowed attempts before status → `'failed'` |
| `last_error` | `TEXT` | yes | none | Error message from the last failed attempt |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `updated_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `background_tasks_pending` partial on `(kind, priority DESC, created_at) WHERE status = 'pending'`. `background_tasks_claimed_expiry` partial on `(claim_expires_at) WHERE status = 'claimed'`. `background_tasks_connector_tick_uniq` UNIQUE partial on `((payload->>'connector_id'), (payload->>'scheduled_for')) WHERE kind = 'connector_tick' AND status IN ('pending', 'claimed')` (added in 005). `background_tasks_tenant_idx` on `(tenant_id)`.

---

#### `connectors`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `uuid_generate_v4()` | PK |
| `name` | `TEXT` | no | none | Part of UNIQUE `(tenant_id, name)` (added in 007, replacing single-column UNIQUE) |
| `type` | `TEXT` | no | none | Connector type identifier |
| `target_database` | `TEXT` | no | none | Destination database name |
| `target_table` | `TEXT` | no | none | Destination table name |
| `config_jsonb` | `JSONB` | no | none | Connector-specific configuration (secret fields encrypted via KMS in Slice 2) |
| `schedule_ms` | `BIGINT` | no | none | Polling interval in milliseconds; CHECK `(schedule_ms >= 100 AND schedule_ms <= 86400000)` |
| `drive_model` | `TEXT` | no | none | CHECK `drive_model IN ('periodic','continuous')` |
| `enabled` | `BOOLEAN` | no | `TRUE` | |
| `disabled_reason` | `TEXT` | yes | none | Human-readable reason when `enabled = FALSE` |
| `last_run_at` | `TIMESTAMPTZ` | yes | none | |
| `last_success_at` | `TIMESTAMPTZ` | yes | none | |
| `last_error` | `TEXT` | yes | none | |
| `last_rows_ingested` | `BIGINT` | yes | none | |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `updated_at` | `TIMESTAMPTZ` | no | `now()` | Updated by writers; no trigger |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |
| `kms_key_id` | `TEXT` | yes | none | Added in 007; KMS key reference for secret encryption (Slice 2) |
| `encrypted_secrets` | `BYTEA` | yes | none | Added in 007; encrypted connector secrets (Slice 2) |

Indexes: `connectors_enabled_drive_idx` partial on `(drive_model, enabled) WHERE enabled = TRUE`. `connectors_tenant_name_uniq` UNIQUE on `(tenant_id, name)`. `connectors_tenant_idx` on `(tenant_id)`.

---

#### `connector_cursors`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `connector_id` | `UUID` | no | none | PK; FK → `connectors(id)` ON DELETE CASCADE; one row per connector |
| `cursor_jsonb` | `JSONB` | yes | none | Last checkpoint (API cursor, timestamp, etc.) |
| `updated_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `connector_cursors_tenant_idx` on `(tenant_id)`.

---

#### `connector_leases`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `connector_id` | `UUID` | no | none | PK; FK → `connectors(id)` ON DELETE CASCADE |
| `node_id` | `TEXT` | no | none | Claiming node identity |
| `expires_at` | `TIMESTAMPTZ` | no | none | Lease expiry |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `connector_leases_tenant_idx` on `(tenant_id)`. **Note:** this table is pre-provisioned for the `Continuous` drive model (streaming connectors) and is unused in v1.0.

---

#### `dashboards`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `gen_random_uuid()` | PK |
| `name` | `TEXT` | no | none | |
| `description` | `TEXT` | yes | none | |
| `time_range_preset` | `TEXT` | no | `'1h'` | |
| `refresh_interval_seconds` | `INTEGER` | yes | none | |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `updated_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `dashboards_tenant_idx` on `(tenant_id)`.

---

#### `dashboard_panels`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `gen_random_uuid()` | PK |
| `dashboard_id` | `UUID` | no | none | FK → `dashboards(id)` ON DELETE CASCADE |
| `title` | `TEXT` | no | none | |
| `panel_type` | `TEXT` | no | none | CHECK `panel_type IN ('chart','table','markdown','stat')` |
| `query` | `TEXT` | yes | none | SQL or KQL query text |
| `database_name` | `TEXT` | yes | none | Target database for the query |
| `config` | `JSONB` | no | `'{}'` | Panel display configuration |
| `grid_x` | `INTEGER` | no | none | |
| `grid_y` | `INTEGER` | no | none | |
| `grid_w` | `INTEGER` | no | none | |
| `grid_h` | `INTEGER` | no | none | |
| `display_order` | `INTEGER` | no | none | Sort key within dashboard |
| `created_at` | `TIMESTAMPTZ` | no | `now()` | |
| `updated_at` | `TIMESTAMPTZ` | no | `now()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `dashboard_panels_dashboard_id_idx` on `(dashboard_id, display_order)`. `dashboard_panels_tenant_idx` on `(tenant_id)`.

---

#### `column_metadata`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `database` | `TEXT` | no | none | Part of composite PK `(tenant_id, database, table_name, column_name)` (PK changed in 007) |
| `table_name` | `TEXT` | no | none | Part of composite PK |
| `column_name` | `TEXT` | no | none | Part of composite PK |
| `column_type` | `TEXT` | no | none | Arrow type string |
| `description` | `TEXT` | yes | none | Human-readable column description for embedding |
| `embedding_model_id` | `TEXT` | yes | none | Model used to generate the embedding |
| `dimension` | `INT` | yes | none | Embedding vector dimension |
| `distance_metric` | `TEXT` | yes | none | Distance metric (`cosine`, `l2`, etc.) |
| `updated_at` | `TIMESTAMPTZ` | no | `NOW()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration; part of composite PK |

No additional indexes beyond the composite PK.

---

#### `schema_embeddings`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `id` | `UUID` | no | `gen_random_uuid()` | PK |
| `database` | `TEXT` | no | none | |
| `table_name` | `TEXT` | no | none | |
| `column_name` | `TEXT` | yes | none | NULL when `kind = 'table'` |
| `kind` | `TEXT` | no | none | CHECK `kind IN ('table','column')` |
| `text_source` | `TEXT` | no | none | The text that was embedded |
| `text_source_sha256` | `BYTEA` | no | none | SHA-256 of `text_source` for change detection |
| `text_format_version` | `TEXT` | no | `'v1'` | Template version for the text rendering |
| `model_id` | `TEXT` | no | none | Embedding model identifier |
| `embedding` | `vector(384)` | no | none | pgvector embedding; requires `vector` extension |
| `updated_at` | `TIMESTAMPTZ` | no | `NOW()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `schema_embeddings_uniq_table` UNIQUE partial on `(tenant_id, database, table_name, model_id) WHERE column_name IS NULL` (rebuilt in 007). `schema_embeddings_uniq_column` UNIQUE partial on `(tenant_id, database, table_name, column_name, model_id) WHERE column_name IS NOT NULL` (rebuilt in 007). `schema_embeddings_hnsw` HNSW on `(embedding vector_cosine_ops)` — ANN vector search. `schema_embeddings_db` on `(tenant_id, database)` (rebuilt in 007).

---

#### `agent_runs`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `run_id` | `UUID` | no | none | PK (no default — caller supplies) |
| `question` | `TEXT` | no | none | Natural-language question |
| `model_id` | `TEXT` | no | none | LLM model identifier |
| `auth_subject` | `TEXT` | no | none | Authenticated user subject |
| `session_id` | `UUID` | yes | none | FK → `agent_sessions(session_id)`; NULL for one-shot runs |
| `started_at` | `TIMESTAMPTZ` | no | none | |
| `finished_at` | `TIMESTAMPTZ` | no | none | |
| `status` | `TEXT` | no | none | CHECK `status IN ('success','error','budget_exceeded','cancelled','replay_miss')` |
| `usage_json` | `JSONB` | no | none | Token counts and cost metadata |
| `trace_json` | `JSONB` | no | none | Full agent execution trace |
| `replay_cache_hit` | `BOOL` | no | `FALSE` | Whether the result was served from `agent_replay_cache` |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `agent_runs_subject_time` on `(auth_subject, started_at DESC)`. `agent_runs_session` on `(session_id, started_at DESC)`. `agent_runs_tenant_idx` on `(tenant_id, started_at DESC)`.

---

#### `agent_sessions`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `session_id` | `UUID` | no | none | PK (no default — caller supplies) |
| `auth_subject` | `TEXT` | no | none | |
| `created_at` | `TIMESTAMPTZ` | no | `NOW()` | |
| `last_active` | `TIMESTAMPTZ` | no | `NOW()` | |
| `metadata_json` | `JSONB` | no | `'{}'` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `agent_sessions_tenant_idx` on `(tenant_id)`.

---

#### `agent_session_turns`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `session_id` | `UUID` | no | none | Part of composite PK `(session_id, turn_index)`; FK → `agent_sessions(session_id)` ON DELETE CASCADE |
| `turn_index` | `INT` | no | none | Part of composite PK; zero-based turn counter |
| `role` | `TEXT` | no | none | CHECK `role IN ('user','assistant')` |
| `content_json` | `JSONB` | no | none | Turn content (messages, tool calls, etc.) |
| `run_id` | `UUID` | yes | none | FK → `agent_runs(run_id)`; NULL for user turns |
| `created_at` | `TIMESTAMPTZ` | no | `NOW()` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration |

Indexes: `agent_session_turns_tenant_idx` on `(tenant_id)`.

---

#### `agent_replay_cache`

| Column | Type | Nullable | Default | Notes |
|---|---|---|---|---|
| `cache_key` | `BYTEA` | no | none | Part of composite PK `(tenant_id, cache_key)` (PK changed in 007 from single-column) |
| `layer` | `TEXT` | no | none | CHECK `layer IN ('generate','run')` |
| `response_json` | `JSONB` | no | none | Cached response |
| `model_id` | `TEXT` | no | none | |
| `created_at` | `TIMESTAMPTZ` | no | `NOW()` | |
| `hit_count` | `INT` | no | `0` | |
| `tenant_id` | `UUID` | no | none | Added in 007; no default after migration; part of composite PK |

Indexes: `agent_replay_cache_layer_model` on `(layer, model_id)`.

---

### Migration discipline

- Each migration is a single SQL file at `crates/kyma-catalog/migrations/<NNNN_name.sql>`.
- File name format: `NNN_snake_case_description.sql` (three-digit prefix, underscore-separated words).
- Numbering is monotonic but not required to be gapless; current series is 001–007 with no gaps.
- The back-compat workflow (Task 17 in F1 plan) runs every PR's migrations against the previous version's catalog dump and fails if any of the forward-only rules above is violated.

### Migration tooling

The migrations are applied by `sqlx::migrate!("./migrations")` embedded at compile time in `crates/kyma-catalog/src/lib.rs`. The macro runs all pending migrations at startup via `sqlx-migrate` (the `sqlx` crate's built-in migrate feature). Migration state is tracked in the standard `_sqlx_migrations` table (created and managed automatically by sqlx; not part of the application schema freeze).

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
