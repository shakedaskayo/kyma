---
title: SQL
description: POST /v1/query with Content-Type application/sql. Standard SQL parsed and executed by DataFusion against every pensieve table, plus three vector-distance UDFs and federation across registered external sources.
---

# SQL

SQL is one of three frontends on the same query path. The body of a `POST
/v1/query` request is parsed by DataFusion, lowered to the same logical plan
KQL produces via `pensieve-plan`, and executed against the registered pensieve
tables. Use it when you want SQL — joins, CTEs, window functions, the works.

## Request shape

```
POST /v1/query
X-Database:    <database>      (default: "default")
Content-Type:  application/sql
```

The body is the SQL text. The default `Content-Type` is already
`application/sql`, so a `curl` without `-H 'Content-Type: ...'` lands on
the SQL path. Set `application/x-kql` for KQL.

The response is NDJSON — one JSON object per row, terminated by `\n`. The
`X-Pensieve-Rows` response header carries the row count for cheap header-only
pagination checks.

## Example

```bash
curl -sS -X POST http://localhost:8080/v1/query \
  -H "X-Database: default" \
  -H "Content-Type: application/sql" \
  --data-binary @- <<'SQL'
SELECT service_name,
       severity_text,
       COUNT(*) AS n
  FROM otel_logs
 WHERE _timestamp > now() - INTERVAL '1 hour'
   AND severity_text IN ('ERROR', 'FATAL')
 GROUP BY service_name, severity_text
 ORDER BY n DESC
 LIMIT 20
SQL
```

The `_timestamp` predicate is the single most important thing in the query.
It feeds the catalog pruner directly — see [The pruning
cascade](/concepts/the-pruning-cascade). A query without a time bound plans
like a full scan because it _is_ a full scan.

## Supported grammar

The full DataFusion SQL surface is available. Highlights:

- `SELECT`, `WHERE`, `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`, `OFFSET`.
- Joins: `INNER`, `LEFT`, `RIGHT`, `FULL OUTER`, `CROSS`. Time-bounded
  small-side joins are the fast shape; see the pruning-cascade page for
  the slow one.
- CTEs (`WITH foo AS (...) SELECT ...`) and recursive CTEs.
- Window functions: `ROW_NUMBER()`, `RANK()`, `LAG`, `LEAD`, aggregate
  windows. Frame clauses (`ROWS BETWEEN ... AND ...`, `RANGE BETWEEN`)
  follow ANSI semantics.
- `UNION`, `INTERSECT`, `EXCEPT` (including `ALL` variants).
- Subqueries — scalar, `IN (SELECT ...)`, `EXISTS`, correlated.
- Set-returning table functions and `LATERAL` joins.

For the complete grammar — every function, every operator, every type
coercion rule — see the [DataFusion SQL
reference](https://datafusion.apache.org/user-guide/sql/index.html). What
DataFusion supports, pensieve supports.

## pensieve-specific UDFs

Three vector-distance scalar functions register on every query session.
They take two list-typed arguments (`FixedSizeList<Float32, N>` or
`List<Float32>` — both work) and return `Float64`:

| UDF                      | What it computes              | Range    |
| ------------------------ | ----------------------------- | -------- |
| `cosine_distance(a, b)`  | `1 - cos(a, b)`               | `[0, 2]` |
| `l2_distance(a, b)`      | Euclidean distance            | `[0, ∞)` |
| `inner_product(a, b)`    | Dot product                   | `(-∞, ∞)`|

Mismatched lengths return `NULL` rather than erroring, so a malformed row
in one extent can't crash the query.

```sql
SELECT id, body,
       cosine_distance(embedding, make_array(0.1, 0.2, ...)) AS dist
  FROM articles
 ORDER BY dist
 LIMIT 5
```

Cast literal embeddings via `make_array(0.1::float, ...)` if you need the
inner element type to be `Float32` exactly; otherwise `Float64` literals
are downcast automatically.

## Cross-source SQL

Every external source registered with `mode: "federation"` or `mode:
"both"` shows up as a first-class DataFusion catalog. A single SQL query
can join a pensieve-native table with a remote Postgres table:

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

DataFusion plans this as: filtered + projected scan pushed down to
Postgres for the small side, pensieve's pruning cascade for the big side,
hash-join in the middle. Pushdown rules and the `pushdown_summary`
returned with every federated response are documented in [Multi-source
data](/concepts/multi-source-data).

## Failure modes

- **Bad SQL.** The DataFusion parser returns its message verbatim with
  `400 sql_parse_error`. Includes the column position when available.
- **Unknown table.** Surfaces as a planning error (`sql_parse_error`)
  rather than a separate `404` — the SessionContext only registers tables
  that exist in the database, so `FROM not_a_table` fails to resolve at
  plan time.
- **Wall-clock budget exceeded.** `429 wall_clock_exceeded` with
  `Retry-After: 1` and `X-Pensieve-Budget-Limit: <ms> wall_clock_ms`. Override
  the default with `X-Pensieve-Max-Wall-Clock-Ms`.
- **Memory budget exceeded.** `429 memory_exceeded` when DataFusion's
  memory pool runs dry — typically a hash join with too-large a build
  side. Override with `X-Pensieve-Max-Memory-Bytes`.
- **Empty database.** `404 database_empty` when the named database has
  no tables registered. `404 database_not_found` when the database
  itself is unknown.
- **Body too large.** `413 body_too_large` past 16 MiB.

Every error response is a JSON envelope with `error.code`,
`error.message`, and `error.request_id`. The `X-Request-ID` header on the
response matches the inbound one, or a fresh UUID if none was supplied —
quote it when filing tickets.

## Where to go next

- The other frontends and the gRPC surface: [Query](/query/).
- Why most queries return in milliseconds: [The pruning
  cascade](/concepts/the-pruning-cascade).
- Pushdown rules and the `live(...)` wrapper: [Multi-source
  data](/concepts/multi-source-data).
- Vector columns and embedding backends: [Dynamic and
  vectors](/concepts/dynamic-and-vectors).
