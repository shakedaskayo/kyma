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

_Filled in by Task 3._

## 3. KQL dialect

_Filled in by Task 4._

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
