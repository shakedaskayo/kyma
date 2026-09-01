---
title: KQL
description: pensieve's pipe-style query language. Operators, aggregations, time helpers, and the operator order that keeps queries fast. Translates to the same logical plan as SQL via `pensieve-kql`.
---

# KQL

KQL is the operator-friendly query language. Read left-to-right, one pipe
at a time:

```kql
otel_logs
| where _timestamp > ago(1h)
| where severity_text == "ERROR"
| summarize n = count() by service_name
| order by n desc
| take 5
```

Same query in SQL costs three `WHERE` clauses, a `GROUP BY`, an `ORDER
BY`, and a `LIMIT`. KQL reads the way you write the question.

`pensieve-kql` parses KQL into pensieve's unified logical plan, which is the
same IR SQL produces. From there, the planner, the
[pruning cascade](/concepts/the-pruning-cascade), and DataFusion don't
know — or care — which surface the query came in through.

## Calling it

```bash
curl -sS -X POST http://localhost:8080/v1/query \
  -H "X-Database: default" \
  -H "Content-Type: application/x-kql" \
  --data-binary 'otel_logs | where severity_text == "ERROR" | take 10'
```

Response is NDJSON — one JSON object per row.

## Operators

Pipe operators run in the order written. Each one consumes a row stream
and produces a row stream.

| Operator     | Shape                                          | Effect                                       |
| ------------ | ---------------------------------------------- | -------------------------------------------- |
| `where`      | `where <predicate>`                            | Filter rows.                                 |
| `project`    | `project col1, col2 = expr, ...`               | Reduce / rename columns.                     |
| `extend`     | `extend col = expr, ...`                       | Add a computed column (no removal).          |
| `summarize`  | `summarize <agg> by <col>, ...`                | Group + aggregate.                           |
| `count`      | `count`                                        | Single-row count of input.                   |
| `take` / `limit` | `take 100`                                 | Cap row output.                              |
| `sort` / `order` | `order by col [asc \| desc]`               | Sort.                                        |
| `distinct`   | `distinct col1, col2`                          | Deduplicate.                                 |

Operator names are case-insensitive. `take` and `limit` are aliases;
`sort` and `order` are aliases.

## Aggregations

In `summarize`:

| Aggregation       | SQL equivalent                |
| ----------------- | ----------------------------- |
| `count()`         | `COUNT(*)`                    |
| `count(col)`      | `COUNT(col)`                  |
| `sum(col)`        | `SUM(col)`                    |
| `avg(col)`        | `AVG(col)`                    |
| `min(col)`        | `MIN(col)`                    |
| `max(col)`        | `MAX(col)`                    |
| `dcount(col)`     | `COUNT(DISTINCT col)`         |

```kql
otel_logs
| where _timestamp > ago(24h)
| summarize
    n = count(),
    services = dcount(service_name),
    last_seen = max(_timestamp)
  by severity_text
| order by n desc
```

Multiple aggregations in one `summarize` produce one row per group.

## Time helpers

| Function                | Returns                                |
| ----------------------- | -------------------------------------- |
| `now()`                 | Current timestamp.                     |
| `ago(<duration>)`       | `now() - duration`. Use it everywhere. |
| `datetime("...")`       | Parse RFC 3339 → timestamp.            |
| `bin(col, <duration>)`  | Truncate to bucket boundary.           |

Durations: `1s`, `30s`, `5m`, `1h`, `7d`, `30d`. They don't need quoting:

```kql
otel_logs
| where _timestamp > ago(1h)
| extend bucket = bin(_timestamp, 1m)
| summarize n = count() by bucket
| order by bucket asc
```

## String predicates

| Operator              | Matches                                         |
| --------------------- | ----------------------------------------------- |
| `==`, `!=`            | Exact equality. Token-indexed; very fast.       |
| `contains`            | Substring match. Token-indexed when prefix-friendly. |
| `startswith`          | Prefix match.                                   |
| `endswith`            | Suffix match.                                   |
| `in (a, b, c)`        | Membership.                                     |
| `!in (a, b, c)`       | Negated membership.                             |

```kql
otel_logs
| where service_name in ("auth-svc", "payments-svc", "checkout-svc")
| where body contains "timeout"
```

## Dynamic-column access

Bracketed paths read from the
[`dynamic` column](/concepts/dynamic-and-vectors):

```kql
otel_logs
| where attributes["http.method"] == "POST"
| where attributes["http.status_code"] >= 500
| project _timestamp, attributes["http.url"], attributes["error.code"]
```

The path doesn't need to be declared anywhere. The token index handles
the predicate; the path bitmap handles extent pruning.

## Joins

`join` takes a left side (the current pipeline) and a right side (a
named table or subquery), with a key:

```kql
otel_logs
| where _timestamp > ago(1h)
| join kind=inner (
    pg_prod.public.users
    | project id, email
  ) on $left.user_id == $right.id
| project _timestamp, email, severity_text, body
```

`kind=inner` is the default; `leftouter`, `rightouter`, `fullouter` are
also recognized. The cross-source case here — a pensieve table joined with
a Postgres table — works because of
[multi-source data](/concepts/multi-source-data).

## Operator order matters

Two queries that return the same rows can differ in cost by orders of
magnitude. The rule: **prune as early as possible.**

```kql
// fast — time + service filter eliminates 99 % at the catalog
otel_logs
| where _timestamp > ago(1h)
| where service_name == "auth-svc"
| where body contains "OOM"
| take 50

// slow — token scan over the whole table because no time bound
otel_logs
| where body contains "OOM"
| take 50
```

The planner doesn't reorder for you. Put time and equality predicates
first so the
[pruning cascade](/concepts/the-pruning-cascade) has something to work
with. See [Pruning and performance](/query/pruning-and-performance)
for the full rules.

## What KQL doesn't do (yet)

Compared to Kusto KQL, pensieve's KQL is a working subset:

- No `let` bindings yet — use SQL CTEs if you need named subqueries.
- No `materialize()`, no `mv-expand`, no `parse_json` (`dynamic` access
  is direct via brackets).
- No time-series operators (`make-series`, `series_decompose_anomalies`).
- No regex predicates yet — `matches regex` parses but lowers to
  `LIKE`-shaped patterns. Full PCRE lands later.

For everything in that list, falling back to [SQL](/query/sql) works
today. New operators land as `QueryFrontend` extensions.

## Where to go next

- The other surface: [SQL](/query/sql).
- Why operator order matters: [Pruning and performance](/query/pruning-and-performance).
- The agent endpoint: [Agent endpoint](/query/agent-endpoint).
- KQL function index (autogenerated): [Reference](/reference/) — landing in D2.
