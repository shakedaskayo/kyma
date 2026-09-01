---
title: Postgres
description: Federation for live reads, replication-slot CDC for sync, both at once. Filter / projection / LIMIT / single-source aggregation pushdown. Type mapping and exactly-once cursor commits in line with the multi-source spec.
---

# Postgres

> 🚧 **Roadmap.** This data source ships in DB-M1. The design below is committed and stable; the implementation is in progress.

The Postgres engine is the first of three operational-database
data sources. It registers as a DataFusion catalog (federation), replays
the source's logical replication into pensieve extents (sync), or both at
once. The same connection pool, the same schema introspection, the same
status surface.

`type`: `"postgres"`. Versions: 15+, 16+. Behind the `federation` Cargo
feature.

## Modes

```bash
curl -sS -X POST http://localhost:8080/v1/data-sources \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "pg_prod",
  "type": "postgres",
  "mode": "both",
  "connection": {
    "url":         "postgres://app@prod-rds.example.com:5432/app",
    "secret_ref":  "$env:PG_PROD_PASSWORD",
    "tls":         "required",
    "pool_size":   10
  },
  "scope": {
    "include_schemas": ["public", "billing"],
    "exclude_tables":  ["public.audit_log"]
  },
  "sync": {
    "tables":      ["public.users", "billing.invoices"],
    "schedule_ms": null
  }
}
JSON
```

- **`mode: "federation"`** — register the source as a DataFusion catalog
  only. Live reads against the source; nothing copied.
- **`mode: "sync"`** — initial snapshot under `REPEATABLE READ`, then
  logical replication into pensieve extents. Queries hit pensieve, not the source.
- **`mode: "both"`** — both at once. Bare references resolve to synced
  extents; `live(table)` opts into the federated path. See
  [Multi-source data](/data-sources/multi-source-data).

`tls` is `"required"` by default; `"disabled"` works but emits a loud
warning into `last_error` and refuses to start without an explicit
override.

## Federation pushdown

DataFusion calls `FederatedTableProvider::scan(filters, projection,
limit, sort)`; the shared `PushdownPlanner` reads the Postgres
`Capabilities` and produces a `PushedPlan::Sql` plus a residual.

**Always pushed:**

- Column projection.
- Filters: `=`, `!=`, `<`, `<=`, `>`, `>=`, `IN`, `IS NULL`, `LIKE`
  (with translated wildcards), AND/OR/NOT trees.
- `LIMIT`, `ORDER BY`, and the combined TopK shape.
- Single-source aggregations: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`,
  `COUNT(DISTINCT)`, `GROUP BY` — when every column referenced is on the
  same source.

**Pushed when safe:**

- Joins where both sides are this same connection. The whole join becomes
  one `SELECT` to Postgres.
- Common scalar functions with verified semantics: `LOWER`, `UPPER`,
  `COALESCE`, `date_trunc`.

**Never pushed:**

- pensieve-specific UDFs (`cosine_distance`, token-index functions, `dynamic`
  accessors).
- Cross-source joins. Each side scans at its own source; DataFusion joins
  the streams.
- Window functions (v1).

Every federated response carries a `pushdown_summary` array — one entry
per `FederatedScan` — listing what got pushed, what stayed residual, and
why. See [Multi-source data](/data-sources/multi-source-data#status-health-and-observability).

## CDC sync

Two-phase pipeline per source-table:

1. **Initial snapshot.** `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY`,
   `pg_create_logical_replication_slot('pensieve_<data_source_id>', 'pgoutput', true)`,
   `SET TRANSACTION SNAPSHOT '<exported>'`. Stream rows in batches; on the
   final batch, advance `data_source_cdc_state.phase` to `streaming` and
   commit the cursor at the slot's LSN — atomically with the pensieve
   extent CAS.
2. **Streaming.** `START_REPLICATION SLOT pensieve_<data_source_id> LOGICAL <lsn>`
   with `proto_version=2`. Group-commit batches to pensieve extents.
   `INSERT`/`UPDATE`/`DELETE` events become rows tagged with `_pensieve_op`
   and a tombstone row for deletes.

The exactly-once knot: the cursor advance is a payload in the same
`tables.current_snapshot_id` CAS that already commits the extent
manifest. Either both land or neither does. On `kill -9` mid-batch, the
runner reopens from `data_source_cdc_state.checkpoint`; the source
replays; pensieve's idempotency layer dedupes any partway-through rows.

## Type mapping

| Postgres type                              | pensieve type                          | Notes                                                                 |
| ------------------------------------------ | ---------------------------------- | --------------------------------------------------------------------- |
| `smallint`, `int`                          | `int`                              | 32-bit signed                                                         |
| `bigint`, `oid`                            | `long`                             |                                                                       |
| `real`, `double precision`                 | `real`                             | 64-bit float                                                          |
| `numeric(p, s)`                            | `real` for `p ≤ 15`; else `string` | Force via `connection.numeric_mode = "string"`                        |
| `boolean`                                  | `bool`                             |                                                                       |
| `text`, `varchar`, `char`, `name`, `uuid`  | `string`                           | UUIDs in canonical form                                               |
| `bytea`                                    | `string` (base64)                  | Bytes-typed columns post-v1                                           |
| `timestamp`, `timestamptz`, `date`, `time` | `timestamp`                        | Always UTC; loses sub-microsecond precision                           |
| `json`, `jsonb`                            | `dynamic` (CBOR)                   | Whole document; field-level inference inside JSONB is post-v1         |
| `int[]`, `text[]`, etc.                    | `dynamic`                          | Arrow `List<T>` mapping post-v1                                       |
| `int4range`, `tstzrange`, etc.             | `dynamic`                          | `{lower, upper, lower_inc, upper_inc}`                                |
| `geometry` (PostGIS)                       | `string` (WKT)                     | Opt-in via `scope.geometry_mode = "wkt"`                              |
| `enum`                                     | `string`                           |                                                                       |
| `hstore`                                   | `dynamic`                          | Map representation in CBOR                                            |
| `inet`, `cidr`, `macaddr`                  | `string`                           |                                                                       |
| `vector` (pgvector)                        | `vector(N)`                        | Dimension fixed; mismatch errors loudly                               |

Composite types and domains unwrap to their base type. Out-of-band types
not in this table land in `dynamic` with a warning in `last_error`.

## System columns on synced tables

Every synced row has four extra columns pensieve adds automatically:

| Column           | Type        | Meaning                                          |
| ---------------- | ----------- | ------------------------------------------------ |
| `_pensieve_pk`       | `string`    | Source PK; for composite PKs, `<col1>:<col2>:...` in `information_schema` order. |
| `_pensieve_op`       | `string`    | `'insert' \| 'update' \| 'delete'`.              |
| `_pensieve_lsn`      | `string`    | Postgres LSN at commit time.                     |
| `_pensieve_event_at` | `timestamp` | Wall-clock the source emitted the event.         |

A source table with no primary key is rejected at data source start with
`disabled_reason="table has no primary key — cannot CDC sync"`. CDC
without a PK can't dedupe replays or build tombstones correctly. Use
`mode: "federation"` for that table instead, or add a PK on the source.

## Failure modes

| Failure                                    | Behavior                                                                   |
| ------------------------------------------ | -------------------------------------------------------------------------- |
| Source unreachable                         | Federation: `502 source_unreachable`. Sync: stream reopens at last cursor. |
| TLS handshake fails                        | Data source disabled, `disabled_reason="tls_failed: <detail>"`.              |
| Replication slot dropped externally        | Data source disabled, `disabled_reason="replication_slot_missing"`. Does NOT auto-recreate. Operator decides: re-enable to re-snapshot, or restore externally. |
| Source DDL adds an unrepresentable type    | Field routes to `dynamic`; warning in `last_error`. Sync continues.        |
| Source DDL drops a column                  | Column null-fills going forward. Existing data preserved.                  |
| Source PK changes                          | Data source disabled, `disabled_reason="pk_changed"`. Manual resync.         |
| Pool exhausted                             | `503 pool_exhausted`; surfaced via `last_error` on `GET /v1/data-sources/:id`. |
| Pushdown produced incorrect SQL (bug)      | Source SQL parse error; `5xx pushdown_failed`; logs SQL + residual. CI property tests gate this. |

## Where to go next

- Cross-source joins, the `live(...)` wrapper, the `pushdown_summary`:
  [Multi-source data](/data-sources/multi-source-data).
- The conceptual model: [Multi-source data](/concepts/multi-source-data).
- The framework: [Data source framework](/data-sources/framework).
