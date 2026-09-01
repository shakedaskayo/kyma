---
title: Schema model
description: pensieve's column types, schema-only-widens rule, and how late-arriving fields land in the dynamic overflow column.
---

# Schema model

pensieve is column-aware. Every table is a sequence of typed columns; every
write is checked against — and may evolve — the catalog-stored schema.
Two non-obvious rules: schema only widens, and anything you didn't predict
lands in the `dynamic` column.

## Column types

The catalog recognizes eight types:

| Type        | Arrow representation                | Notes                                  |
| ----------- | ----------------------------------- | -------------------------------------- |
| `int`       | `Int32`                             | 32-bit signed.                         |
| `long`      | `Int64`                             | 64-bit signed.                         |
| `real`      | `Float64`                           | IEEE-754 double.                       |
| `bool`      | `Boolean`                           |                                        |
| `string`    | `Utf8`                              | Token-indexed when materialized.       |
| `timestamp` | `Timestamp(microsecond, UTC)`       | Always UTC; sub-µs is dropped.         |
| `dynamic`   | CBOR-encoded `Binary`               | Arbitrary structured data; see below.  |
| `vector(N)` | `FixedSizeList<Float32, N>`         | Fixed dimension; ANN indices in M-B.   |

Plus the four system columns pensieve adds to tables synced from external
sources via the data source framework — `_pensieve_pk`, `_pensieve_op`, `_pensieve_lsn`,
`_pensieve_event_at`. Internal pensieve tables don't carry these.

## Schema only widens

A table's schema is a versioned object in the catalog. Every CAS commit
either keeps the schema or widens it. Widening means:

- **Add a column.** Old extents continue to read with the new column
  null-filled.
- **Promote a `dynamic` field to typed.** A field that's been seen with
  a consistent type often enough is allowed to graduate. Old data stays
  in `dynamic`; new data goes to the typed column. Reads union the two.
- **Loosen a constraint.** Nullable becomes nullable-still; never the
  other way.

Things pensieve does not do:

- **Narrow a type.** Once a column is `long`, it cannot become `int`.
- **Delete a column.** Schemas only ever add. The visual hint that a
  column is "gone" is that nothing writes it; old data still reads.
- **Rewrite history.** Schema changes don't migrate old extents. A
  query against the new schema reads old extents through the schema
  evolution path: missing columns null-fill at decode time.

This rule is what makes ingest correct under concurrent schema evolution.
Two writers committing different schemas both succeed; the catalog
assembles the wider of the two; the loser of the CAS rebases against
the new schema and retries.

## Mid-batch evolution

If a row in a batch references a column that's not in the schema yet,
the staging buffer:

1. Flushes whatever's already in the batch under the current schema.
2. Proposes a schema widening to the catalog.
3. Resumes the batch under the new schema.

Writers never see "your row was rejected because of a column you didn't
declare." They see at most a small flush boundary in the middle of a big
batch.

## The `dynamic` column

Arbitrary structured data — Mongo documents, Postgres JSONB blobs, raw
log attributes, OTLP resource maps — lands here.

`dynamic` values are stored as CBOR. Indices land alongside:

- A **token index** over leaf strings, so
  `where attributes["error.code"] == "ECONNRESET"` plans like a normal
  string-equality query.
- A **path bitmap** per extent, recording which paths are populated. A
  query on a path that was never written to this extent skips it
  immediately.

Querying `dynamic` looks like KQL with bracketed paths:

```kql
otel_logs
| where attributes["service.name"] == "auth-svc"
| where attributes["http.status_code"] >= 500
| project _timestamp, body, attributes["error.code"]
```

You can promote a `dynamic` path to a typed column once the schema
stabilizes. A data source with sync mode does this automatically — see
[Multi-source data](/concepts/multi-source-data).

## When to use what

- **`int` / `long`** for IDs, counts, status codes.
- **`real`** for measured quantities. Don't store currency here; use
  `string` (or two `long` columns for major+minor units).
- **`string`** for tokenized text — service names, error codes, log
  bodies. Token indexing kicks in automatically.
- **`timestamp`** for the ingest's `_timestamp` and any other event time.
- **`dynamic`** for everything that's still finding its shape. Promote
  fields to typed columns once the shape is stable.
- **`vector(N)`** for embeddings. The dimension is fixed at table
  creation; mismatched-dimension writes fail loudly.

## Where to go next

- The `dynamic` column in depth: [Dynamic and vectors](/concepts/dynamic-and-vectors).
- How extents evolve through schema changes: [Extents and snapshots](/concepts/extents-and-snapshots).
- KQL reference: [Query](/query/).
