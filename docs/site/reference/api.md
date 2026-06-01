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
{ "question": "...", "database": "default", "include_thinking": false }
```

The SSE response carries seven event names: `run_started`,
`thinking_delta` (only when `include_thinking: true`),
`answer_delta`, `tool_call`, `tool_result`, `answer_final`,
`run_error`, `run_finished`. `run_finished` is always the last
frame. See [Agent endpoint](/query/agent-endpoint) for the full
event payload reference.

Run wall-clock cap: 60 s. Per-run tool-call cap: 12. Both surface as
`run_error` codes (`timeout` and `tool_loop`) before the stream ends.

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

## Connectors

The connector admin surface manages periodic / scheduled connectors. The
catalog (`GET /v1/connectors/catalog`) is what drives the UI's source picker.

| Method | Path                                  | Min role | Effect                                                                                  |
| ------ | ------------------------------------- | -------- | --------------------------------------------------------------------------------------- |
| GET    | `/v1/connectors/catalog`              | Write    | The vendor catalog (available + coming-soon) that powers the Add-connector UI.          |
| POST   | `/v1/connectors`                      | Write    | Create a connector. Body: `{name, type, target_database, target_table, schedule_ms, config}`. |
| GET    | `/v1/connectors`                      | Write    | List connectors (id, name, type, enabled).                                              |
| GET    | `/v1/connectors/{id}`                 | Write    | Read a connector. Secret-shaped fields in `config` are scrubbed to `***`.               |
| PATCH  | `/v1/connectors/{id}`                 | Write    | Update name / schedule / enabled / config.                                              |
| DELETE | `/v1/connectors/{id}`                 | Write    | Delete the connector.                                                                   |
| POST   | `/v1/connectors/{id}/pause`           | Write    | Disable the connector (`disabled_reason="manual"`).                                     |
| POST   | `/v1/connectors/{id}/resume`          | Write    | Re-enable.                                                                              |
| POST   | `/v1/connectors/{id}/trigger`         | Write    | Enqueue an immediate tick. Returns `202 Accepted`.                                      |

`schedule_ms` must be in the range `[100, 86400000]` (100 ms → 24 h).
See [Connectors](/connectors/) for the typed config shape per
connector type.

## Credentials

Typed, encrypted secrets referenced by connectors (and other subsystems) via a
`credential_id` in their config. List/get return a masked **preview**, never
plaintext. Requires `KYMA_SECRET_KEY`. See [OAuth connectors](/connectors/oauth).

| Method | Path                     | Min role | Effect                                                                          |
| ------ | ------------------------ | -------- | ------------------------------------------------------------------------------- |
| POST   | `/v1/credentials`        | Write    | Create a credential. Body: `{label, value:{kind, …}, metadata?}`. Returns a summary. |
| GET    | `/v1/credentials`        | Write    | List credentials (label, kind, masked preview, metadata).                       |
| GET    | `/v1/credentials/{id}`   | Write    | Fetch one credential summary (no plaintext).                                     |
| DELETE | `/v1/credentials/{id}`   | Write    | Delete a credential.                                                            |

`value.kind` ∈ `pat`, `basic`, `oauth2`, `url`, `aws_creds`, `api_key`,
`github_app`.

## OAuth

The browser flow that mints `oauth2` credentials for OAuth connectors. The
callback is **unauthenticated** (a cross-site IdP redirect carries no bearer); the
single-use `state` token is the trust anchor. See [OAuth connectors](/connectors/oauth).

| Method | Path                                | Min role | Effect                                                                                       |
| ------ | ----------------------------------- | -------- | -------------------------------------------------------------------------------------------- |
| POST   | `/v1/oauth/{provider}/start`        | Write    | Begin a flow. Body: `{connector_type, scopes?, label?, client_id?, client_secret?}`. Returns `{authorize_url, state}`. |
| GET    | `/v1/oauth/{provider}/callback`     | _none_   | IdP redirect target. Exchanges the code, mints the credential, posts the result back to the opener. |
| GET    | `/v1/oauth/flows/{state}`           | Write    | Poll a flow's `{status, credential_id}` — the fallback when `postMessage` can't reach the UI.  |

`{provider}` ∈ `google`, `notion`, `atlassian`, `slack`.

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
