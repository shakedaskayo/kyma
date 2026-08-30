---
title: Idempotency and coercion
description: Cross-cutting rules every ingest path follows — JSON-to-Arrow type coercion, schema evolution mid-batch, and the three idempotency keys.
---

# Idempotency and coercion

Every ingest path — REST, OTLP, Kafka, file-drop — funnels into the
same write path. Three things behave the same regardless of how the
bytes arrived: how JSON values become Arrow values, how the schema
widens mid-batch, and how duplicates are detected. This page is the
canonical reference for those three.

## JSON → Arrow coercion

NDJSON is parsed against the table's catalog-stored schema. Each
top-level field maps to a column; anything not in the schema is
either dropped, evolved into a new column, or stored in `props` —
depending on the path's settings.

| JSON               | Schema column type                   | Result                                                                  |
| ------------------ | ------------------------------------ | ----------------------------------------------------------------------- |
| `null`             | any nullable                         | `null`.                                                                 |
| `true` / `false`   | `bool`                               | Direct.                                                                 |
| Integer            | `int` (Int32) / `long` (Int64)       | Range-checked; a value larger than `Int32::MAX` against `int` errors.   |
| Number             | `real` (Float64)                     | Direct. NaN/Infinity rejected by the underlying Arrow JSON reader.      |
| String             | `string` (Utf8)                      | Direct.                                                                 |
| RFC3339 string     | `timestamp`                          | Parsed to nanosecond UTC.                                               |
| Number (epoch ns)  | `timestamp`                          | Treated as nanoseconds since epoch.                                     |
| Array of numbers   | `vector(N)` (`FixedSizeList<Float32, N>`) | Length must equal `N`; mismatches return a precise error.          |
| Anything           | `dynamic` (Binary)                   | Stored as the raw JSON-bytes representation; queryable via path syntax. |

Two columns get special handling: vectors and dynamic. Both are split
out of the arrow-json fast path and decoded by a pensieve-specific NDJSON
reader so the JSON shape matches the typed shape exactly.

A field present in JSON but absent from the schema is not an error.
Where it ends up depends on the path:

- REST and file-drop with `schema_evolve=true` (default) — a new
  `string` column is added on first sight, capped at 32 new columns
  per request.
- REST with `schema_evolve=false`, OTLP, Kafka — the field is dropped
  silently.

A field present in the schema but absent from a JSON row is `null`
for that row. Old extents stay readable through schema widening: a
query that references a column added later sees `null` for rows that
predate it.

## Schema evolution mid-batch

The hard rule from [Schema model](/concepts/schema-model): schema
only widens. Any ingest path that triggers an `ALTER TABLE ADD COLUMN`
mid-batch goes through the same dance:

1. Pre-scan the parsed records once for unknown top-level keys.
2. Filter to identifier-safe names (lowercase ASCII, `[a-z0-9_]`,
   not starting with a digit, not one of the reserved default columns).
3. Cap at 32 new columns; further unknown fields land in `props` for
   that request.
4. Run one `alter_table_add_column` per accepted name. Each new
   column is `string` (Utf8), nullable.
5. Re-look up the table so the parse runs against the freshest schema.

Why `string` and not the JSON's actual type? Two reasons. NDJSON
arrives line by line — the next record may write a different type for
the same key, and once the column exists every later write is locked
into that type. And `cast(col as float)` at query time is cheap; an
incorrectly-narrow column that has to be rewritten is not. Strong
typing comes from pre-creating the table.

Old data is never rewritten. If a `dynamic` field stabilizes to the
point where you want a typed column, promote it explicitly; both
representations coexist and reads union them via `coalesce()`.

## Idempotency keys

Three shapes, one ledger. `ingest_ledger` (Postgres) holds one row per
applied key; a re-applied key returns the cached ack with `replayed:
true` and writes nothing.

### REST: `X-Idempotency-Key`

Opaque string up to whatever your gateway permits — UUIDs are typical.
Scope is global to the catalog; collisions across tables are your
problem to avoid (a `tenant:table:hash` shape is good practice).

A replayed key returns `replayed: true` and the original
`snapshot_id`. The body is read but discarded.

Race window: two concurrent requests with the same key may both
ingest before either records to the ledger. The ledger's
`INSERT … ON CONFLICT` ensures only one row wins; the loser logs a
warning and a small duplicate is accepted. Tighter atomicity is part
of the M2 staging-buffer work.

TTL: 24 hours. A background sweeper deletes entries past 25 hours
(1-hour grace) hourly. Replays inside the TTL no-op; outside, the
key is treated as fresh.

### File-drop: `filedrop:{sha256}`

The watcher computes SHA256 over the full object bytes, prefixes it
with `filedrop:`, and uses the result as the idempotency key. Re-scans
of the same bucket pick up the same files and no-op against the ledger.

Because the key derives from content, a one-byte change is a fresh
file — and ingests as a new extent. This is the intended behavior;
files in the drop bucket are an audit trail you can replay by
clearing the ledger.

### Kafka: catalog-tracked offsets

Kafka does not use `ingest_ledger`. Instead, after each successful
batch ingest the consumer commits the partition's offset back to
Kafka. On restart, the consumer seeks each partition to the last
committed offset + 1.

Today this is at-least-once at the catalog boundary: a crash between
the snapshot CAS and the offset commit re-consumes the unflushed
window. True exactly-once — extent commit and offset commit in one
Postgres transaction — is tracked work. See
[Kafka](/ingest/kafka) for the practical consequences.

## Where to go next

- Type promotion and the `dynamic` column in depth: [Schema model](/concepts/schema-model).
- The CAS commit that ties this all together: [Extents and snapshots](/concepts/extents-and-snapshots).
- The path-specific docs: [REST / NDJSON](/ingest/rest-ndjson), [OTLP gRPC](/ingest/otlp-grpc), [Kafka](/ingest/kafka), [File-drop](/ingest/file-drop).
