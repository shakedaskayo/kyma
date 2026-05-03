---
title: Schema mappings
description: Postgres / MySQL / MongoDB type → kyma type mappings the connector framework will use when DB-M1, DB-M2, and DB-M3 ship.
---

# Schema mappings

> 🚧 **Roadmap.** These mappings are the source-of-truth tables the
> Postgres, MySQL, and MongoDB connectors will use once they land
> (DB-M1, DB-M2, DB-M3 in the connector roadmap). They are not yet
> in the running engine. The Prometheus connector that exists today
> doesn't introspect a relational schema — it ingests metrics with a
> fixed shape.

When a relational or document source is synced, kyma's connector
framework introspects the source's schema and projects each source
column / field to a kyma column type. Three rules govern the
projection:

1. Synced data lands in normal kyma extents using the eight kyma
   column types (`int`, `long`, `real`, `bool`, `string`,
   `timestamp`, `dynamic`, `vector(N)`).
2. Schema only widens — never narrows, never drops, never re-types.
3. Every inferred column is nullable (Mongo's polymorphism +
   Postgres's missing-column semantics both require it).

Every synced row also carries four system identity columns:

| Column            | Type        | Meaning                                                    |
| ----------------- | ----------- | ---------------------------------------------------------- |
| `_kyma_pk`        | `string`    | Concatenated source primary key (or `_id` for Mongo).      |
| `_kyma_op`        | `string`    | One of `'insert'`, `'update'`, `'delete'`.                 |
| `_kyma_lsn`       | `string`    | Engine-specific cursor at commit time.                     |
| `_kyma_event_at`  | `timestamp` | Wall-clock time the source emitted the event.              |

Deletes are tombstones, not row removal — reads filter
`_kyma_op != 'delete'` by default. Compaction collapses tombstones
older than `retention.tombstone_days` (default 30).

## Postgres → kyma

| Postgres type                                    | kyma type                                  | Notes                                                                                |
| ------------------------------------------------ | ------------------------------------------ | ------------------------------------------------------------------------------------ |
| `smallint`, `int`                                | `int`                                      | 32-bit signed.                                                                       |
| `bigint`, `oid`                                  | `long`                                     |                                                                                      |
| `real`, `double precision`                       | `real`                                     | 64-bit float.                                                                        |
| `numeric(p, s)`                                  | `real` for `p ≤ 15`; `string` otherwise    | Forced via `connection.numeric_mode = "string"`.                                     |
| `boolean`                                        | `bool`                                     |                                                                                      |
| `text`, `varchar`, `char`, `name`, `uuid`        | `string`                                   | UUIDs in canonical form.                                                             |
| `bytea`                                          | `string` (base64)                          | Bytes-typed columns post-v1.                                                         |
| `timestamp`, `timestamptz`, `date`, `time`       | `timestamp`                                | Always UTC; loses sub-microsecond precision.                                         |
| `json`, `jsonb`                                  | `dynamic` (CBOR)                           | Whole document; field-level inference inside JSONB is post-v1.                       |
| `int[]`, `text[]`, etc.                          | `dynamic`                                  | Arrow `List<T>` mapping post-v1.                                                     |
| `int4range`, `tstzrange`, etc.                   | `dynamic`                                  | `{lower, upper, lower_inc, upper_inc}`.                                              |
| `geometry` (PostGIS)                             | `string` (WKT)                             | Opt-in via `scope.geometry_mode = "wkt"`.                                            |
| `enum`                                           | `string`                                   |                                                                                      |
| `hstore`                                         | `dynamic`                                  | Map representation in CBOR.                                                          |
| `inet`, `cidr`, `macaddr`                        | `string`                                   |                                                                                      |
| `vector` (pgvector)                              | `vector(N)`                                | Dimension fixed; mismatch errors loudly.                                             |

Composite types and domains unwrap to their base type. Out-of-band
types not in this table land in `dynamic` with a `last_error`
warning surfaced through the connector status endpoint.

## MySQL → kyma

| MySQL type                                                   | kyma type                                       | Notes                                                                                  |
| ------------------------------------------------------------ | ----------------------------------------------- | -------------------------------------------------------------------------------------- |
| `tinyint`, `smallint`, `mediumint`, `int`                    | `int`                                           | `tinyint(1)` → `bool`.                                                                 |
| `bigint`, `bigint unsigned`                                  | `long`                                          | `bigint unsigned` > i64 max → `string` with warning.                                  |
| `decimal(p, s)`                                              | `real` for `p ≤ 15`; `string` otherwise         |                                                                                        |
| `float`, `double`                                            | `real`                                          |                                                                                        |
| `bit`                                                        | `bool` (width 1) or `dynamic` (wider)           |                                                                                        |
| `date`, `datetime`, `timestamp`, `time`, `year`              | `timestamp`                                     | UTC; `time` and `year` stringified post-v1.                                           |
| `char`, `varchar`, `text`, `longtext`, `enum`, `set`         | `string`                                        | `set` comma-joined.                                                                    |
| `binary`, `varbinary`, `blob`                                | `string` (base64)                               |                                                                                        |
| `json`                                                       | `dynamic` (CBOR)                                | Same rule as Postgres JSONB.                                                           |
| Spatial (`geometry`, `point`, …)                             | `string` (WKT)                                  |                                                                                        |

**Collation safety.** The MySQL pushdown planner refuses to push any
`string` equality / `LIKE` filter unless the column's collation is
one of the case-sensitive variants (`utf8mb4_bin`,
`utf8mb4_0900_as_cs`). Filters on case-insensitive columns evaluate
above the scan in DataFusion. Without this rule, federated queries
on case-insensitive collations would return silently incorrect
results.

## MongoDB → kyma

Top-level fields each get one inferred kyma column. Nested objects
flatten with dotted names up to `scope.flatten_depth` (default 2).
Anything deeper, polymorphic, or array-shaped lands in `dynamic`.

| BSON type                                                                | kyma type                                       |
| ------------------------------------------------------------------------ | ----------------------------------------------- |
| `Int32`                                                                  | `int`                                           |
| `Int64`, `Decimal128` (when fits)                                        | `long`                                          |
| `Double`, `Decimal128` (otherwise)                                       | `real`; force `string` via `scope.decimal128_mode`. |
| `Boolean`                                                                | `bool`                                          |
| `String`                                                                 | `string`                                        |
| `ObjectId`                                                               | `string` (24-char hex)                          |
| `UUID` (binary subtype 4)                                                | `string`                                        |
| `Date`                                                                   | `timestamp` (UTC, ms precision)                 |
| `Timestamp` (BSON timestamp)                                             | `timestamp`                                     |
| `Binary` (other subtypes)                                                | `string` (base64)                               |
| `Null`, `Undefined`                                                      | NULL                                            |
| `Array` of homogeneous primitive                                         | `dynamic` (Arrow `List<T>` post-v1)             |
| `Array` of mixed / objects                                               | `dynamic`                                       |
| `Object` (≤ flatten_depth)                                               | flattened to dotted columns                     |
| `Object` (> flatten_depth)                                               | `dynamic`                                       |
| `RegExp`, `JavaScript`, `MinKey`, `MaxKey`, `DBPointer`, `Symbol`        | `dynamic` (rare; warn)                          |

**Polymorphic field handling.** If a field is observed as `int` 800
times then `string` shows up, the schema evolver stops promoting
that field; it stays `dynamic` from then on. Existing typed-column
data is preserved. Future events go into `dynamic`. Queries
union-read the typed and dynamic copies via
`coalesce(typed_col, dynamic.field)`.

## `_kyma_pk` derivation

| Source                                       | `_kyma_pk` is                                                                                |
| -------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Postgres / MySQL — single-column PK          | string-cast of the PK.                                                                       |
| Postgres / MySQL — composite PK              | `<col1>:<col2>:...` (URL-safe, ordered by `information_schema`).                            |
| Postgres / MySQL — no PK                     | Connector **fails to start** for that table with `last_error = "table has no primary key"`. |
| MongoDB                                      | Stringified `_id`.                                                                           |

## What's deferred

- **JSONB / nested-relational drilling.** Postgres `JSONB` and MySQL
  `JSON` columns land whole in `dynamic` for v1. Field-level
  inference is v1.5; the path is a `scope.jsonb_drill: { columns,
  stability_threshold }` knob that runs the schema evolver
  recursively over `dynamic` content.
- **Arrow `List<T>` mapping.** Homogeneous arrays land in `dynamic`
  for v1. Once `List<T>` is wired through the format and parser,
  arrays will be typed.
- **Raw bytes.** `bytea`, `binary`, `varbinary`, `blob`, and Mongo
  binary subtypes all land base64-encoded in `string` for v1.

The full design lives in
`docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`.
This page is the user-facing extract; the spec is the source of
truth.
