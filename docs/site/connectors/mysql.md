---
title: MySQL
description: Federation and binlog-CDC sync against MySQL 8.0+. Same shape as the Postgres connector, plus collation-safety in the planner so case-insensitive columns never silently corrupt federated equality.
---

# MySQL

> 🚧 **Roadmap.** This connector ships in DB-M2, after [Postgres](/connectors/postgres). The design below is committed and stable; the implementation is in progress.

The MySQL engine reuses everything DB-M1 delivered for Postgres — the
`ExternalSource` trait, the federation crate, the CDC consumer, the
schema evolver. What's MySQL-specific is the binlog reader, the GTID
checkpoint, and one correctness-critical pushdown rule: collations.

`type`: `"mysql"`. Versions: 8.0+. Behind the `federation` Cargo feature.

## Configuration

```bash
curl -sS -X POST http://localhost:8080/v1/connectors \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "mysql_orders",
  "type": "mysql",
  "mode": "both",
  "connection": {
    "url":         "mysql://app@orders-rds.example.com:3306/orders",
    "secret_ref":  "$env:MYSQL_ORDERS_PASSWORD",
    "tls":         "required",
    "pool_size":   10
  },
  "scope": {
    "include_schemas": ["orders"],
    "exclude_tables":  ["orders.audit_log"]
  },
  "sync": {
    "tables": ["orders.line_items", "orders.shipments"]
  }
}
JSON
```

The `mode`, `connection`, `scope`, and `sync` shapes are the same as
[Postgres](/connectors/postgres#modes). `tls: "required"` is the default;
disabling it requires an explicit override.

## Collation safety

This is the trap MySQL sets that nothing else does. By default many
MySQL columns use case-insensitive collations
(`utf8mb4_general_ci`, `utf8mb4_0900_ai_ci`, …). Pushed equality and
`LIKE` against such a column would return rows DataFusion's own
case-sensitive evaluation never would — and quietly corrupt every
federated query that joined on a string column.

The MySQL `Capabilities` struct carries
`string_collation_safe_columns: Set<TableQualifiedName>`, populated at
introspection time. The shared `PushdownPlanner` refuses to push any
`string` equality or `LIKE` filter unless the column's collation is one
of the explicitly verified case-sensitive variants
(`utf8mb4_bin`, `utf8mb4_0900_as_cs`). Filters on case-insensitive
columns evaluate above the scan, in DataFusion, where the semantics are
predictable.

The `pushdown_summary` flags the residual with
`agg_residual_reason` / `filters_residual` so an operator can see why a
filter that "looks" pushable didn't. To force pushdown, change the
column's collation on the source to a case-sensitive one — there is no
override knob, intentionally. A wrong answer is worse than a slow one.

## CDC sync

Two-phase pipeline per source-table, mirroring the Postgres flow:

1. **Initial snapshot.** `START TRANSACTION WITH CONSISTENT SNAPSHOT`,
   capture `gtid_executed`, stream rows, advance
   `connector_cdc_state.phase` to `streaming` with the captured GTID set
   as the cursor — atomically with the kyma extent CAS.
2. **Streaming.** `COM_BINLOG_DUMP_GTID` with the executed-GTID set as
   the resume point. Row events become rows tagged with `_kyma_op`;
   deletes are tombstones.

Cursor checkpoints are GTID sets, stored as opaque JSON in
`connector_cdc_state.checkpoint`. Reopen-from-checkpoint is the only
recovery path; the source replays, kyma's idempotency layer dedupes.

## Type mapping

| MySQL type                                                  | kyma type                                  | Notes                                                                 |
| ----------------------------------------------------------- | ------------------------------------------ | --------------------------------------------------------------------- |
| `tinyint`, `smallint`, `mediumint`, `int`                   | `int`                                      | `tinyint(1)` → `bool`                                                 |
| `bigint`, `bigint unsigned`                                 | `long`                                     | `bigint unsigned` > i64 max → `string` with warning                   |
| `decimal(p, s)`                                             | `real` for `p ≤ 15`; else `string`         |                                                                       |
| `float`, `double`                                           | `real`                                     |                                                                       |
| `bit`                                                       | `bool` (width 1) or `dynamic` (wider)      |                                                                       |
| `date`, `datetime`, `timestamp`, `time`, `year`             | `timestamp`                                | UTC; `time` and `year` stringified post-v1                            |
| `char`, `varchar`, `text`, `longtext`, `enum`, `set`        | `string`                                   | `set` comma-joined                                                    |
| `binary`, `varbinary`, `blob`                               | `string` (base64)                          |                                                                       |
| `json`                                                      | `dynamic` (CBOR)                           | Whole document; field-level inference is post-v1                      |
| Spatial (`geometry`, `point`, …)                            | `string` (WKT)                             |                                                                       |

## System columns on synced tables

Every synced row has four extra columns kyma adds automatically:

| Column           | Type        | Meaning                                                                 |
| ---------------- | ----------- | ----------------------------------------------------------------------- |
| `_kyma_pk`       | `string`    | Source PK; for composite PKs, `<col1>:<col2>:...` in `information_schema` order. |
| `_kyma_op`       | `string`    | `'insert' \| 'update' \| 'delete'`.                                     |
| `_kyma_lsn`      | `string`    | GTID at commit time.                                                    |
| `_kyma_event_at` | `timestamp` | Wall-clock the source emitted the event.                                |

A source table with no primary key is rejected at connector start with
`disabled_reason="table has no primary key — cannot CDC sync"`. Use
`mode: "federation"` for that table instead.

## Federation pushdown

Filters, projection, `LIMIT`, `ORDER BY`, single-source aggregations,
opportunistic same-source joins. The list is identical to
[Postgres](/connectors/postgres#federation-pushdown) — minus the
collation-unsafe filters described above, which always go residual.

## Failure modes

| Failure                                                | Behavior                                                              |
| ------------------------------------------------------ | --------------------------------------------------------------------- |
| Source unreachable                                     | Federation: `502 source_unreachable`. Sync: stream reopens at GTID.    |
| TLS handshake fails                                    | Connector disabled, `disabled_reason="tls_failed: <detail>"`.          |
| Binlog purged past the connector's GTID                | Connector disabled, `disabled_reason="binlog_purged"`. Manual resync. |
| `tinyint(1)` source column widens to `tinyint(4)`      | New column added; old `bool` column preserved; reads union via `coalesce`. |
| Source DDL emits unrepresentable type                  | Field routes to `dynamic`; warning in `last_error`. Sync continues.   |
| Source PK changes                                      | Connector disabled, `disabled_reason="pk_changed"`. Manual resync.    |
| `bigint unsigned` value > 2^63 - 1                     | Routed to `string` with warning. Existing typed data preserved.       |
| Pool exhausted                                         | `503 pool_exhausted`; surfaced via `last_error` on `GET /v1/connectors/:id`. |

## Where to go next

- The first DB engine and the shape both share: [Postgres](/connectors/postgres).
- Cross-source joins, the `pushdown_summary`, `live(...)`:
  [Multi-source data](/connectors/multi-source-data).
- The framework: [Connector framework](/connectors/framework).
