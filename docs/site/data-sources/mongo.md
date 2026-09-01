---
title: MongoDB
description: Federation and change-stream sync against MongoDB 7.0+. Nested-document flattening, BSON type coercion, polymorphic-field demotion to dynamic.
---

# MongoDB

> 🚧 **Roadmap.** This data source ships in DB-M3, after [Postgres](/data-sources/postgres) and [MySQL](/data-sources/mysql). The design below is committed and stable; the implementation is in progress.

The MongoDB engine is the document-store member of the family. The
trait is the same; what changes is everything below it: BSON instead of
SQL types, nested objects flattened to dotted columns, change streams
instead of replication slots, an aggregation pipeline as the pushed
plan instead of a SQL string.

`type`: `"mongo"`. Versions: 7.0+. Behind the `federation` Cargo
feature.

## Configuration

```bash
curl -sS -X POST http://localhost:8080/v1/data-sources \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "mongo_shop",
  "type": "mongo",
  "mode": "both",
  "connection": {
    "url":         "mongodb://app@cluster0.example.com:27017/shop",
    "secret_ref":  "$env:MONGO_SHOP_PASSWORD",
    "tls":         "required",
    "pool_size":   10
  },
  "scope": {
    "include_databases": ["shop"],
    "include_collections": ["shop.orders", "shop.customers"],
    "flatten_depth": 2
  },
  "sync": {
    "tables": ["shop.orders", "shop.customers"]
  }
}
JSON
```

`mode`, `connection`, and `tls` semantics match the SQL engines —
[Postgres](/data-sources/postgres#modes) is the reference. The
Mongo-specific bit is `scope.flatten_depth` (default `2`), which
controls how deep nested objects flatten to dotted columns.

## Nested-document flattening

Top-level fields each get one inferred pensieve column. Nested objects
flatten into dotted column names up to `scope.flatten_depth`. Anything
deeper, or anything polymorphic, lands in `dynamic`.

With `flatten_depth = 2`:

```
Mongo doc:
{
  "_id":   ObjectId("..."),
  "user":  { "id": 42, "email": "a@b.com",
             "addr": { "city": "Berlin" } },
  "total": 99.5,
  "tags":  ["a", "b"]
}

→ pensieve columns:
  _pensieve_pk        string  "<ObjectId hex>"
  user.id         int     42
  user.email      string  "a@b.com"
  user.addr       dynamic {city: "Berlin"}    ← depth 3, lands in dynamic
  total           real    99.5
  tags            dynamic ["a", "b"]           ← arrays go to dynamic in v1
```

Arrays always land in `dynamic` for v1; typed Arrow `List<T>` for
homogeneous-primitive arrays is a v1.5 follow-up.

## Polymorphic field handling

A Mongo collection can have `count` as `int` for 800 docs and then
`string` for one. Pensieve's `SchemaEvolver` notices and:

1. Stops promoting that field. It stays `dynamic`.
2. Existing typed-column data is preserved. Reads union the typed and
   dynamic copies via `coalesce(typed_col, dynamic.field)`.
3. Future events of any type land in `dynamic`.

Schema only widens — pensieve never narrows, never deletes, never re-types.

The default stability threshold is **100 events with one consistent
type within a sliding window of 1000 events** (or the entire snapshot if
under 1000 docs). Tunable per-data-source via
`sync.inference.stability_threshold`.

## BSON type coercion

| BSON type                                                 | pensieve type                                          |
| --------------------------------------------------------- | -------------------------------------------------- |
| `Int32`                                                   | `int`                                              |
| `Int64`, `Decimal128` (when fits)                         | `long`                                             |
| `Double`, `Decimal128` (otherwise)                        | `real`; force `string` via `scope.decimal128_mode` |
| `Boolean`                                                 | `bool`                                             |
| `String`                                                  | `string`                                           |
| `ObjectId`                                                | `string` (24-char hex)                             |
| `UUID` (binary subtype 4)                                 | `string`                                           |
| `Date`                                                    | `timestamp` (UTC, ms precision)                    |
| `Timestamp` (BSON timestamp)                              | `timestamp`                                        |
| `Binary` (other subtypes)                                 | `string` (base64)                                  |
| `Null`, `Undefined`                                       | NULL                                               |
| `Array` of homogeneous primitive                          | `dynamic` (Arrow `List<T>` post-v1)                |
| `Array` of mixed / objects                                | `dynamic`                                          |
| `Object` (≤ flatten_depth)                                | flattened to dotted columns                        |
| `Object` (> flatten_depth)                                | `dynamic`                                          |
| `RegExp`, `JavaScript`, `MinKey`, `MaxKey`, `DBPointer`, `Symbol` | `dynamic` (rare; warn)                       |

## CDC sync

Two-phase pipeline per source-collection:

1. **Initial snapshot.** Take a `$changeStream` resume token via
   `startAtOperationTime` taken **before** the snapshot read; stream the
   collection's documents in batches; on the final batch advance
   `data_source_cdc_state.phase` to `streaming` with the resume token as
   the cursor — atomically with the pensieve extent CAS.
2. **Streaming.** Open a change stream with `resumeAfter=<token>`;
   group-commit batches. Inserts/updates/deletes become rows tagged with
   `_pensieve_op`; deletes are tombstones.

Cursor checkpoints are change-stream resume tokens, stored as opaque
JSON in `data_source_cdc_state.checkpoint`. Reopen-from-token is the
recovery path; the change stream replays from the token; pensieve's
idempotency layer dedupes any partway-through events.

## System columns on synced collections

Every synced row has four extra columns pensieve adds automatically:

| Column           | Type        | Meaning                                                  |
| ---------------- | ----------- | -------------------------------------------------------- |
| `_pensieve_pk`       | `string`    | Stringified `_id`. Always present — `_id` is mandatory in MongoDB. |
| `_pensieve_op`       | `string`    | `'insert' \| 'update' \| 'delete'`.                      |
| `_pensieve_lsn`      | `string`    | Resume token at commit time.                             |
| `_pensieve_event_at` | `timestamp` | Wall-clock the source emitted the event.                 |

Mongo collections always have `_id`, so the no-PK rejection that affects
the SQL engines doesn't fire here. Composite or compound shard keys
don't change `_pensieve_pk` — `_id` is enough.

## Federation pushdown

Pushed plans for Mongo are aggregation pipelines, not SQL strings.
`PushdownPlanner` produces a `PushedPlan::MongoPipeline(Vec<Document>)`
the engine consumes opaquely.

The pushed surface is the same shape as the SQL engines: filters,
projection, `$limit`, `$sort`, `$group` for single-source aggregations.
Cross-collection joins (`$lookup`) are not pushed in v1 — the operator
shape is too engine-specific to share with the SQL planner. Each side
runs at its source; DataFusion joins the streams.

## Failure modes

| Failure                                                  | Behavior                                                              |
| -------------------------------------------------------- | --------------------------------------------------------------------- |
| Source unreachable                                       | Federation: `502 source_unreachable`. Sync: stream reopens at token.  |
| TLS handshake fails                                      | Data source disabled, `disabled_reason="tls_failed: <detail>"`.         |
| Resume token expired (oplog rolled past it)              | Data source disabled, `disabled_reason="resume_token_invalid"`. Manual resync. |
| New polymorphic field shows up                           | Demoted to `dynamic`; existing typed-column data preserved.           |
| Document depth exceeds `flatten_depth`                   | Subtree lands in `dynamic` for that field. Reads via dynamic accessors. |
| Decimal128 overflow under default mode                   | Routed to `string` with warning. Set `scope.decimal128_mode` to fix.   |
| UTF-8 decode fails on `String` field                     | Field becomes `dynamic` with raw bytes base64'd; warning.             |
| Pool exhausted                                           | `503 pool_exhausted`; surfaced via `last_error` on `GET /v1/data-sources/:id`. |

## Where to go next

- The SQL siblings: [Postgres](/data-sources/postgres), [MySQL](/data-sources/mysql).
- Cross-source queries, `live(...)`, the `pushdown_summary`:
  [Multi-source data](/data-sources/multi-source-data).
- How `dynamic` works in queries: [Dynamic and vectors](/concepts/dynamic-and-vectors).
