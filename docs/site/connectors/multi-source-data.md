---
title: Multi-source data
description: The practical face of federation and sync — how to register a source, the live(...) UX, the pushdown_summary, and the kyma_connector_health view that makes status queryable.
---

# Multi-source data

> 🚧 **Roadmap.** The federation + sync experience lands in DB-M1 (Postgres), with MySQL and MongoDB following in DB-M2 / DB-M3. The conceptual page at [Multi-source data](/concepts/multi-source-data) explains why the design looks like this; this page is the operator's hands-on view of what to type.

This page is the operational counterpart to the conceptual
[Multi-source data](/concepts/multi-source-data). Read that one first
for the why; come here for the curl commands, the SQL syntax, and the
status surface.

The model in one sentence: every external source registers as a
DataFusion catalog (federation), or replays into kyma extents via CDC
(sync), or both — the same connection, the same schema introspection,
the same status row, on the same connector framework as
[Prometheus](/connectors/prometheus).

## Register a source

```bash
curl -sS -X POST http://localhost:8080/v1/connectors \
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
    "include_schemas": ["public", "billing"]
  },
  "sync": {
    "tables": ["public.users", "billing.invoices"]
  }
}
JSON
```

Response: `{"id": "<uuid>"}`. After this:

- The `pg_prod.public.*` and `pg_prod.billing.*` namespaces are valid in
  any KQL or SQL query — federated reads run live against the source.
- `public.users` and `billing.invoices` have started snapshotting into
  kyma extents. Once the initial snapshot is done the connector advances
  to `phase=streaming` and stays there.

Per-engine config and the type-mapping tables live on each engine page:
[Postgres](/connectors/postgres), [MySQL](/connectors/mysql),
[MongoDB](/connectors/mongo).

The admin API runs `validate_secrets_resolvable` before persisting, so
a typo'd `secret_ref` fails at create time, not silently in the runner.

## Query it

For sources registered with `mode: "federation"`, references resolve to
the live source. For `mode: "sync"`, they resolve to the kyma extents
the CDC pipeline lands rows in. For `mode: "both"`, bare references
resolve to the synced extent (predictable, fast); `live(table)` opts
into the federated path:

```sql
-- Synced read — sub-second, no source load.
SELECT * FROM pg_prod.public.users WHERE id = 42;

-- Federated read — fresh-as-of-now, costs the source a query.
SELECT * FROM live(pg_prod.public.users) WHERE id = 42;

-- Cross-source join — federated small side, kyma big side.
SELECT u.email, COUNT(*) AS errors
  FROM pg_prod.public.users u
  JOIN otel_logs l ON l.user_id = u.id
 WHERE l.severity_text = 'ERROR'
   AND l._timestamp > now() - INTERVAL '1 hour'
 GROUP BY u.email
 ORDER BY errors DESC
 LIMIT 5;
```

Use `live(...)` when you need read-after-write freshness — e.g., right
after a transaction the agent triggered. Default to the synced path for
everything else; sync lag is bounded (typically seconds), and you avoid
hitting the source for routine queries.

## The `pushdown_summary`

Every response that touched a federated source carries a
`pushdown_summary` array — one entry per `FederatedScan`:

```json
{
  "source": "pg_prod",
  "table": "public.users",
  "filters_pushed": ["region = $1"],
  "filters_residual": [],
  "projection_pushed": true,
  "limit_pushed": 50,
  "sort_pushed": null,
  "agg_pushed": null,
  "agg_residual_reason": "cross-source group-by",
  "join_pushed": false,
  "scan_duration_ms": 14,
  "rows_returned": 50,
  "bytes_received": 4128
}
```

This is the trust mechanism for federation. If a query is slow, the
summary tells you exactly which filter went residual and why, so you
can rewrite the query, change the source schema (e.g., a MySQL
collation — see [MySQL](/connectors/mysql#collation-safety)), or
register a different mode.

The CI suite asserts `pushdown_summary` is non-degrading on a curated
set of queries, so accidental planner regressions get caught at PR
time, not in production.

## What gets pushed

The exact list lives on each engine page; the shared rules are:

- **Always pushed:** projection, `=`/`!=`/`<`/`<=`/`>`/`>=`/`IN`/`IS NULL`/`LIKE`,
  AND/OR/NOT trees, `LIMIT`, `ORDER BY`, single-source aggregations.
- **Pushed when safe:** same-source same-connection joins; verified
  scalar functions (`LOWER`, `UPPER`, `COALESCE`, `date_trunc`).
- **Never pushed:** kyma UDFs (`cosine_distance`, dynamic accessors),
  cross-source joins, window functions, anything where source semantics
  diverge from DataFusion's (most notably MySQL case-insensitive
  collations).

If the planner is uncertain, it leaves the expression in the residual.
Slow-but-right beats fast-and-wrong — and every pushdown rule has a
property test that runs the same query both ways and asserts identical
Arrow output.

## Status, health, and observability

`GET /v1/connectors/:id/status` returns a structured doc per source —
federation pool stats, sync phase, lag in seconds, last error, schema
drift if any:

```json
{
  "id": "...",
  "type": "postgres",
  "mode": "both",
  "source":     { "reachable": true, "version": "PG 15.4", ... },
  "federation": { "status": "healthy", "pool_in_use": 2, "pool_max": 10,
                  "p50_query_ms": 14, "p99_query_ms": 230, ... },
  "sync":       { "status": "streaming", "lag_seconds": 4,
                  "events_per_sec": 1200, "rows_synced": 5240000, ... }
}
```

The same data is exposed as a queryable kyma table:

```sql
SELECT connector_name, mode, phase, lag_seconds, events_per_sec
  FROM kyma_connector_health
 WHERE lag_seconds > 60;
```

Schema:

| Column           | Type        | Description                                               |
| ---------------- | ----------- | --------------------------------------------------------- |
| `connector_id`   | `string`    |                                                           |
| `connector_name` | `string`    |                                                           |
| `type`           | `string`    | `postgres` / `mysql` / `mongo` / `prometheus` / …          |
| `mode`           | `string`    | `federation` / `sync` / `both`                             |
| `phase`          | `string`    | per source-table                                           |
| `lag_seconds`    | `real`      | sync-mode lag                                              |
| `events_per_sec` | `real`      | sync throughput                                            |
| `pool_in_use`    | `int`       | federation pool snapshot                                   |
| `pool_max`       | `int`       |                                                            |
| `last_error`     | `string`    |                                                            |
| `last_event_at`  | `timestamp` |                                                            |
| `timestamp`      | `timestamp` | row emit time (matches kyma's standard column name)        |

Agents query this with KQL/SQL; dashboards chart it; alerts fire on it.
This is what closes the "kyma observing kyma observing your databases"
loop without dedicated alerting config.

`GET /v1/connectors/:id/events` returns the last 100 state transitions
— phase changes, error onsets, recoveries — for postmortem.

## System columns on synced tables

Synced rows always have four columns kyma adds automatically:
`_kyma_pk`, `_kyma_op`, `_kyma_lsn`, `_kyma_event_at`. Deletes are
tombstone rows with `_kyma_op = 'delete'`; default reads at the
federation/agent layer hide them via predicate. See
[Multi-source data (concepts)](/concepts/multi-source-data#system-columns-on-synced-tables)
for the row-semantics rules and
[Retention and compaction](/concepts/retention-and-compaction) for how
tombstones get garbage-collected.

## Mode-isolated pause

`mode: "both"` connectors can have federation healthy while sync is
errored, and vice versa. Pause one without re-snapshotting the other:

```bash
curl -X POST 'http://localhost:8080/v1/connectors/<id>/pause?scope=sync'
curl -X POST 'http://localhost:8080/v1/connectors/<id>/resume?scope=sync'
```

Default `scope` is `all`. Useful when an upstream source is doing
maintenance and you want to keep federation answering while sync
catches up later.

## Where to go next

- The conceptual model: [Multi-source data (concepts)](/concepts/multi-source-data).
- The framework that makes all of this work: [Connector framework](/connectors/framework).
- Engine pages: [Postgres](/connectors/postgres),
  [MySQL](/connectors/mysql), [MongoDB](/connectors/mongo).
- The shape of pushdown decisions: [Pruning and performance](/query/pruning-and-performance).
- The full design and milestones:
  [the spec](https://github.com/shaked/engine/blob/main/docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md).
