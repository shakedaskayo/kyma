---
title: KQL functions
description: Lookup index for every KQL pipe-operator and every scalar / aggregation function the kyma-kql parser recognises today.
---

# KQL functions

Every operator and every function `kyma-kql` knows about today. Use
this page as the lookup index — see [KQL](/query/kql) for the
prose-and-examples version.

KQL queries are pipelines: a table name followed by `|` operators.
Each operator consumes a row stream and produces a row stream. The
parser is recursive-descent; it streams operators directly into a
SQL builder — there's no intermediate AST.

## Pipe operators

The full set the parser dispatches on. Operator names are bare
identifiers; KQL is case-sensitive on operator names today.

| Operator               | Shape                                                            | Effect                                                                        |
| ---------------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `where`                | `where <expr>`                                                   | Filter rows by predicate.                                                     |
| `project`              | `project col1, col2, ...`                                        | Restrict the projection list to the named columns.                            |
| `project-away`         | `project-away col1, col2, ...`                                   | Drop the named columns from the projection.                                   |
| `extend`               | `extend new_col = <expr>, ...`                                   | Add computed columns. Existing columns are kept.                              |
| `summarize`            | `summarize agg = expr, ... [by group_expr, ...]`                 | Group + aggregate. Without `by`, produces one row.                            |
| `count`                | `count`                                                          | Single-row `count(*)` aliased `Count`.                                        |
| `take` / `limit`       | `take N`                                                         | Cap output rows. Aliases.                                                     |
| `sort` / `order`       | `sort by col [asc \| desc], ...`                                 | Sort rows. KQL default is `desc` when omitted.                                |
| `top`                  | `top N by col [asc \| desc]`                                     | `sort by col` + `take N` in one operator.                                     |
| `distinct`             | `distinct [col1, col2, ...]`                                     | Deduplicate. With column list, also restricts the projection.                 |
| `graph-traverse`       | `graph-traverse source <v> from <src> to <dst> max-hops N [direction forward\|backward\|both]` | Recursive edge-walk over a table where each row is an edge. Adds `depth` to the projection. |
| `graph-shortest-path`  | `graph-shortest-path source <v> target <w> from <src> to <dst> max-hops N [direction ...]`   | Single-row result: `{depth, found}`. |

`from`, `to`, `source`, `target`, `max-hops`, `direction`, `forward`,
`backward`, `both`, `by`, `asc`, `desc` are reserved keywords inside
their respective operators.

## Time helpers

Scalar functions you'll use inside `where` and `extend`. All of them
return values you can compare against `timestamp` columns or use
inside `summarize ... by`.

| Function              | SQL it lowers to                                          | Purpose                                                                                |
| --------------------- | --------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `now()`               | `now()`                                                   | Current wall-clock timestamp.                                                          |
| `ago(d)`              | `(now() - d)`                                             | Subtract a duration. Pass a duration literal (`1h`, `5m`, `30d`).                      |
| `bin(col, d)`         | `date_bin(d, col, '1970-01-01')`                          | Floor `col` to a bucket of size `d`. Use inside `summarize ... by` for time series.    |
| `startofhour(col)`    | `date_trunc('hour', col)`                                 | Truncate to the start of the containing hour.                                          |
| `startofday(col)`     | `date_trunc('day', col)`                                  | Truncate to the start of the containing day.                                           |
| `startofmonth(col)`   | `date_trunc('month', col)`                                | Truncate to the start of the containing month.                                         |
| `datetime(x)`         | `CAST(x AS TIMESTAMP)`                                    | Cast a literal or string to timestamp.                                                 |

Duration literals are bare-suffix numbers: `1ms`, `30s`, `5m`, `1h`,
`7d`. They lower to SQL `INTERVAL` literals.

## String functions

| Function          | SQL it lowers to     | Purpose                                                  |
| ----------------- | -------------------- | -------------------------------------------------------- |
| `strcat(a, b, ...)` | `concat(a, b, ...)` | Concatenate strings.                                     |
| `tolower(s)`      | `lower(s)`           | Lower-case.                                              |
| `toupper(s)`      | `upper(s)`           | Upper-case.                                              |
| `strlen(s)`       | `char_length(s)`     | Length in characters.                                    |

## Predicates and conditional

| Function          | SQL it lowers to                                | Purpose                                                                |
| ----------------- | ----------------------------------------------- | ---------------------------------------------------------------------- |
| `isnull(x)`       | `(x IS NULL)`                                   | Null-test.                                                             |
| `isnotnull(x)`    | `(x IS NOT NULL)`                               | Non-null test.                                                         |
| `iff(c, a, b)`    | `(CASE WHEN c THEN a ELSE b END)`               | Conditional value.                                                     |

In addition to the function form, KQL supports infix operators:
`==`, `!=`, `<`, `<=`, `>`, `>=`, `+`, `-`, `*`, `/`, `%`, plus the
boolean keywords `and`, `or`, `not`. The four word-form string
operators — `contains`, `startswith`, `endswith`, `has` — lower to
`LIKE` patterns the planner can push down to the token index. Use
`between (low .. high)` for inclusive range, `in (v1, v2, ...)` for
set membership.

## Aggregations (used inside `summarize`)

| Aggregation        | SQL equivalent                                                  |
| ------------------ | --------------------------------------------------------------- |
| `count()`          | `count(*)`                                                      |
| `count(x)`         | `count(x)`                                                      |
| `sum(x)`           | `sum(x)`                                                        |
| `avg(x)`           | `avg(x)`                                                        |
| `min(x)`           | `min(x)`                                                        |
| `max(x)`           | `max(x)`                                                        |
| `dcount(x)`        | `count(DISTINCT x)`                                             |
| `dcountif(x, c)`   | `count(DISTINCT CASE WHEN c THEN x ELSE NULL END)`              |

## Literals

| Literal              | Notes                                                       |
| -------------------- | ----------------------------------------------------------- |
| `42`, `3.14`         | Integer / float.                                            |
| `'hello'`            | Single-quoted string. `''` escapes a quote.                 |
| `true`, `false`      | Boolean.                                                    |
| `null`               | Null.                                                       |
| `1h`, `5m`, `30s`, `1d`, `100ms` | Duration. Lowers to a SQL `INTERVAL`.           |

Identifiers that contain only `[A-Za-z0-9_]` and don't start with a
digit are emitted bare; everything else is double-quoted.

## What's not here

Dotted paths into `dynamic` columns (`data.user.id`) are recognised
by the lexer but the parser rejects them with a clear error
("dotted path … not yet supported (dynamic columns land with
format-v1)"). Functions outside this list error as
`unsupported function: <name>/<arity>` — the parser is closed,
not extensible.

The complete operator set lives in `crates/kyma-kql/src/parser.rs`.
This page is current as of the source there; D2 will replace it with
generated content from the parser itself.
