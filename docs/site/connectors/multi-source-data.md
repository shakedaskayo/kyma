---
title: Multi-source data
description: The practical face of federation and sync — how to register a source, the live(...) UX, the pushdown_summary, and connector run-state via GET /v1/connectors/:id and connector metrics.
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

`GET /v1/connectors/:id` returns the connector row with current run
state — `enabled`, `disabled_reason`, `last_run_at`, `last_success_at`,
`last_error`, and `last_rows_ingested`:

```json
{
  "id": "<uuid>",
  "name": "pg_prod",
  "type": "postgres",
  "target_database": "default",
  "target_table": "pg_prod",
  "schedule_ms": 0,
  "drive_model": "Continuous",
  "enabled": true,
  "disabled_reason": null,
  "last_run_at": "2024-01-15T10:23:45Z",
  "last_success_at": "2024-01-15T10:23:44Z",
  "last_error": null,
  "last_rows_ingested": 1200,
  "config": { "..." }
}
```

Secrets in `config` are redacted to `***` (unless they are unresolved
`$env:` references, which are returned verbatim).

For time-series observability, kyma exposes per-tick Prometheus metrics
for every connector — query these in [Observability](/concepts/observability):

| Metric                                          | Description                                    |
| ----------------------------------------------- | ---------------------------------------------- |
| `kyma_connector_cursor_age_seconds`             | Sync lag — how far behind the source cursor is |
| `kyma_connector_rows_ingested_total`            | Cumulative rows landed                         |
| `kyma_connector_errors_total`                   | Cumulative tick errors by connector            |
| `kyma_connector_last_success_timestamp_seconds` | Unix timestamp of last successful tick         |
| `kyma_connector_ticks_total`                    | Total tick count                               |
| `kyma_connector_duration_seconds`               | Tick duration histogram                        |

Agents check `last_error` / `last_success_at` on the detail endpoint;
dashboards and alerts consume the Prometheus metrics.

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

To pause or resume a connector, use the plain pause and resume endpoints:

```bash
curl -X POST http://localhost:8080/v1/connectors/<id>/pause
curl -X POST http://localhost:8080/v1/connectors/<id>/resume
```

`pause` sets `enabled=false` with `disabled_reason='manual'`. `resume` clears
it. For `mode: "both"` connectors, pausing stops the entire connector (both
federation and sync). Use `GET /v1/connectors/:id` to confirm the state change.

## Where to go next

- The conceptual model: [Multi-source data (concepts)](/concepts/multi-source-data).
- The framework that makes all of this work: [Connector framework](/connectors/framework).
- Engine pages: [Postgres](/connectors/postgres),
  [MySQL](/connectors/mysql), [MongoDB](/connectors/mongo).
- The shape of pushdown decisions: [Pruning and performance](/query/pruning-and-performance).
