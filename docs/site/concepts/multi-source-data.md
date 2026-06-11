---
title: Multi-source data
description: How kyma joins your operational databases — Postgres, MySQL, MongoDB — with its own tables. Federation for live reads, sync via CDC for fast historical queries, both for the same source.
---

# Multi-source data

Most production data isn't in kyma. It's in Postgres tables, Mongo
collections, MySQL databases. kyma federates with those, so a single
query can join a `kyma.default.otel_logs` table with a
`pg_prod.public.users` table — and DataFusion plans the join across
both.

Two integration modes. Both can run on the same source at the same time.

<KymaMultiSourceFlow caption="The two paths a query can take through the engine. Federation is live; sync stages data into kyma extents." />

## Federation

Register an external database as a DataFusion catalog. Queries push
filters, projection, `LIMIT`, `ORDER BY`, and single-source aggregations
down to the source. Cross-source joins happen in DataFusion, with the
small side fetched live.

```bash
curl -X POST http://localhost:8080/v1/data-sources \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "pg_prod",
    "type": "postgres",
    "mode": "federation",
    "connection": {
      "url": "postgres://app@prod-rds.example.com:5432/app",
      "secret_ref": "pg_prod_password",
      "tls": "required",
      "pool_size": 10
    },
    "scope": {
      "include_schemas": ["public", "billing"]
    }
  }'
```

After registration, the source is a first-class DataFusion catalog.

```sql
SELECT u.email, COUNT(*) AS errors
  FROM pg_prod.public.users u
  JOIN otel_logs l ON l.user_id = u.id
 WHERE l.severity_text = 'ERROR'
   AND l._timestamp > now() - INTERVAL '1 hour'
 GROUP BY u.email
 ORDER BY errors DESC
 LIMIT 5
```

DataFusion plans this as: pushdown a filtered, projected scan to
Postgres for the small side; scan kyma's pruning cascade for the big
side; hash-join the two; aggregate; limit.

The `pushdown_summary` returned with every federation response tells
you exactly what got pushed and what didn't — see
[Observability](/concepts/observability).

### Always pushed

- Column projection.
- Filters: `=`, `!=`, `<`, `<=`, `>`, `>=`, `IN`, `IS NULL`, `LIKE`,
  `AND`/`OR`/`NOT` trees.
- `LIMIT` and `ORDER BY`.
- Single-source aggregations: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`,
  `COUNT(DISTINCT)`, `GROUP BY`.

### Pushed when safe

- Joins where both sides are the same external source and same
  connection. The whole join becomes a single SQL query to the source.
- Common scalar functions with verified semantics: `LOWER`, `UPPER`,
  `COALESCE`, date truncation.

### Never pushed

- kyma-specific UDFs (`cosine_distance`, `dynamic` accessors).
- Cross-source joins. Each side runs at its source; DataFusion joins
  the streams.
- Anything where source semantics diverge from DataFusion's — most
  notably MySQL case-insensitive collations, which silently change
  string equality results.

## Sync

The same source registered with `mode: "sync"` (or `"both"`) replays
the source's change log into kyma extents. After the initial snapshot,
ongoing inserts/updates/deletes stream in via the source's native
CDC mechanism:

| Source     | CDC mechanism                                                   |
| ---------- | --------------------------------------------------------------- |
| Postgres   | Logical replication slots (`CREATE_REPLICATION_SLOT`, `pgoutput`).|
| MySQL      | Binlog row events (`COM_BINLOG_DUMP_GTID`).                     |
| MongoDB    | Change streams (`$changeStream` with `startAtOperationTime`).   |

The exactly-once knot: every batch's commit advances the source's
cursor (LSN, GTID, resumeToken) atomically with the kyma snapshot CAS.
Either both land or neither does.

After sync, the same query runs against kyma extents — sub-second over
years of history, no live load on the source DB.

## Both at once

`mode: "both"` registers both paths. Queries default to the synced
extents (predictable, fast):

```sql
SELECT * FROM pg_prod.public.users WHERE id = 42
```

Wrap the table in `live(...)` to opt into the federated path:

```sql
SELECT * FROM live(pg_prod.public.users) WHERE id = 42
```

Use `live()` when you need the freshest possible read (e.g., right after
a transaction). Default to the synced path for everything else.

## System columns on synced tables

Synced tables get four extra columns kyma adds automatically. They
don't exist on internal kyma tables:

| Column            | Type        | Meaning                                |
| ----------------- | ----------- | -------------------------------------- |
| `_kyma_pk`        | `string`    | Concatenated source primary key.       |
| `_kyma_op`        | `string`    | `'insert' \| 'update' \| 'delete'`.    |
| `_kyma_lsn`       | `string`    | Engine-specific cursor at commit time. |
| `_kyma_event_at`  | `timestamp` | When the source emitted this event.    |

Deletes are tombstone rows with `_kyma_op = 'delete'`. Default reads
hide them via the federation/agent layer's predicate; raw scans see
everything. See [Retention and compaction](/concepts/retention-and-compaction)
for how tombstones get garbage-collected.

## Schema evolution on synced sources

The data source framework runs the same schema-evolver as native ingest.
A new column on the source becomes a new typed column on the kyma
table after enough events with consistent type. A column whose type
becomes polymorphic falls back to `dynamic`. Old data is preserved
either way; reads union typed and dynamic via `coalesce()`.

The hard rule: if the source table has no primary key, kyma refuses
to sync it. CDC without a PK can't dedupe replays or build tombstones
correctly; you'd silently lose data. Use federation instead.

## Where to go next

- Data source administration: [Data sources](/data-sources/).
- Pushdown details and the `pushdown_summary`: [Observability](/concepts/observability).
