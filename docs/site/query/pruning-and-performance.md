---
title: Pruning and performance
description: Practical rules for writing fast pensieve queries. What the pruning cascade can and can't do for you, what makes a query slow, and how to read the diagnostics.
---

# Pruning and performance

The [pruning cascade](/concepts/the-pruning-cascade) is the conceptual
piece — three levels of catalog/extent/block elimination before any
data is decoded. This page is the practical companion: rules for
writing queries that the cascade can actually prune, and for diagnosing
queries that are slower than they should be.

## The single rule that matters

**Always include a time bound.** Almost every pensieve table is
time-partitioned. The catalog stores `min(_timestamp)` and
`max(_timestamp)` per extent. A query without a time predicate has
nothing to filter on at stage 1 — it plans like a full scan because
it _is_ a full scan.

```kql
// Yes
otel_logs | where _timestamp > ago(1h) | where ...

// Slow
otel_logs | where ...    // every extent ever, considered.
```

If your table genuinely doesn't have a `_timestamp`-equivalent column
(say, a small reference table synced from Postgres), this rule doesn't
apply. For everything else, default to a time bound.

## Predicates the cascade prunes well

Stage 1 (catalog) prunes on:

- **Time range** — `_timestamp > ago(...)`, `_timestamp between datetime(...) .. datetime(...)`.
- **Equality** on columns with useful min/max statistics —
  `service_name == "auth-svc"`, `tenant_id == 42`.
- **Token containment** for tokenized string columns — `body contains
  "OOM"`, `attributes["error.code"] == "ECONNRESET"`.

Stage 2 (extent footers) sharpens those with bloom filters and per-
block stats. Stage 3 (block index) intersects posting lists for
exact rows.

A query whose predicates are all in this list runs at sub-second
latency over years of history. A query that adds a UDF or a free-form
expression to the mix doesn't get worse — but the unused predicates
fall out of the prunable set, so it's slower than it could be.

## Predicates the cascade can't help with

These can still be answered, but only after decode:

- **Function calls on a column** — `where lower(service_name) == "auth"`.
  The cascade doesn't see through the function. Pre-normalize on
  ingest, or filter on the original column.
- **Computed expressions** — `where (a + b) > 100`. Same reason.
- **Cross-extent operations** — `where row_number() > 100` only knows
  itself after the rows are arranged.

Equivalent rewrites:

```kql
// Slow — function on a column hides the value from stats.
otel_logs
| where lower(severity_text) == "error"

// Fast — pensieve's string equality is case-sensitive; if your data is
// dirty, normalize on ingest with X-Schema-Evolve and a coercer.
otel_logs
| where severity_text == "ERROR"
```

## Cardinality matters in joins

A join's cost is dominated by the **build side** — the smaller of the
two inputs becomes a hash table, the larger streams through. Two
patterns:

- **Small-left join big-right.** Fetches the small side fully, hashes
  it, scans the big side once. Cheap.
- **Big-left join big-right.** Both sides materialize. Expensive.
  Avoid.

For [federation joins](/concepts/multi-source-data), the small side
is usually a Postgres table you've prefiltered. The pruning cascade
on the pensieve side handles the big side.

```sql
SELECT u.email, COUNT(*)
  FROM pg_prod.public.users u   -- small after filter
  JOIN otel_logs l               -- big, but pruned by time
    ON l.user_id = u.id
 WHERE u.region = 'eu'
   AND l._timestamp > now() - INTERVAL '1 hour'
   AND l.severity_text = 'ERROR'
 GROUP BY u.email
 ORDER BY 2 DESC
 LIMIT 5
```

DataFusion picks the build side automatically. If you find yourself
writing a query where the small side genuinely isn't small, materialize
it first into a pensieve table (one ingest call), then join.

## Query budgets

Two safeguards run on every query:

- **Wall-time budget.** Default 30 s; configurable per request via
  `X-Query-Budget-Ms: 5000`. Exceeded queries return `504
  query_timeout` and are cancelled. The pruning cascade's elimination
  ratio is included in the response; if you see a high ratio and a
  timeout, the bottleneck is on the source side (or DataFusion exec),
  not on storage.
- **Result-size budget.** Default 100 MiB; configurable via
  `X-Query-Max-Bytes`. A query that would return more streams what
  it can and returns `500 result_too_large` after the cap. Use
  Arrow Flight for unbounded streams instead.

Cancellation is cooperative: the planner checks the budget at every
extent boundary and at every Arrow `RecordBatch` decode. A pathological
query that decodes a single huge batch can run a few hundred ms past
the budget while the current batch finishes.

## Diagnostics — `pushdown_summary`

For [federated queries](/concepts/multi-source-data), every response
carries one entry per `FederatedScan`:

```json
{
  "source": "pg_prod",
  "table": "public.users",
  "filters_pushed":   ["region = $1"],
  "filters_residual": [],
  "projection_pushed": true,
  "limit_pushed": null,
  "agg_pushed": null,
  "agg_residual_reason": "cross-source group-by",
  "scan_duration_ms": 14,
  "rows_returned": 3127
}
```

`filters_residual` populated for a filter you'd expect to push is
the most common "why is this slow?" answer. See
[Observability](/concepts/observability) for the full schema.

## Diagnostics — Prometheus metrics

```
pensieve_query_duration_seconds{language="kql", quantile="0.99"}
pensieve_query_extents_pruned_ratio_bucket{le="0.99"}
pensieve_query_decode_bytes_total
```

`pensieve_query_extents_pruned_ratio` is the headline. A healthy
deployment has the p99 above 0.99 — most queries eliminate most
extents. A regression here often points to a missing time bound in
some new dashboard, not a pensieve bug. See
[Observability](/concepts/observability).

## A short cookbook

**Top N over a time window** — text book pruning shape.

```kql
otel_logs
| where _timestamp > ago(1h)
| where severity_text == "ERROR"
| summarize n = count() by service_name
| order by n desc
| take 10
```

**Group by time bucket** — `bin()` plays well with the cascade.

```kql
otel_logs
| where _timestamp > ago(24h)
| where service_name == "payments-svc"
| extend bucket = bin(_timestamp, 5m)
| summarize errors = count() by bucket
| order by bucket asc
```

**Cross-source enrichment** — small left, big right.

```sql
SELECT t.bucket, t.errors, u.tier
  FROM (
    SELECT bin(_timestamp, 5m) AS bucket,
           user_id,
           COUNT(*) AS errors
      FROM otel_logs
     WHERE _timestamp > now() - INTERVAL '1 hour'
       AND severity_text = 'ERROR'
     GROUP BY bucket, user_id
  ) t
  JOIN pg_prod.public.users u ON u.id = t.user_id
 WHERE u.tier = 'enterprise'
 ORDER BY t.bucket, t.errors DESC
```

## Where to go next

- The conceptual model: [The pruning cascade](/concepts/the-pruning-cascade).
- Federation pushdown rules: [Multi-source data](/concepts/multi-source-data).
- Reading the metrics: [Observability](/concepts/observability).
