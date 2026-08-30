---
title: Extents and snapshots
description: How pensieve stores data on object storage. Append-only columnar extents, atomically-committed snapshots, and the catalog manifests that let queries skip everything.
---

# Extents and snapshots

The unit of storage in pensieve is an extent — a chunk of columnar Arrow data
on object storage. The unit of visibility is a snapshot — a catalog
transaction that publishes one or more new extents atomically. Once written,
neither is mutated. New writes produce new extents; new states produce new
snapshots.

<PensieveArchitectureDiagram caption="pensieve's two lanes — ingest and query — share the storage spine. Object storage is the only source of truth; the catalog is externalized; compute is stateless." />

This shape is what makes the pruning cascade possible, what makes ingest
exactly-once, and what makes "every node is stateless" structurally true.

## Extents

An extent is one immutable file on object storage holding column chunks for
some rows in some table.

- **Columnar.** Each column lives in its own contiguous region inside the
  file. Reads can range-GET only the columns the query projects.
- **Self-describing.** Each extent ends with a footer carrying per-column
  block offsets, per-block min/max, present-paths bitmaps for the dynamic
  column, and an inverted index for tokenized columns. Footers are tiny;
  caches keep them resident.
- **Time-bounded.** Every extent records `min(_timestamp)` and
  `max(_timestamp)` of its rows. The catalog uses this for the cheapest
  pruning step.
- **Append-only.** An extent is never updated in place. Schema evolution,
  late-arriving rows, deletes, or backfills all produce *new* extents.
  Old extents stay readable until retention sweeps them.

## Snapshots

A snapshot is the table's state at one CAS-committed point in catalog time.
Reading from a table means reading at a snapshot. Writing to a table means
proposing a new snapshot via compare-and-swap on `tables.current_snapshot_id`.

The CAS is what makes concurrent writes correct without locks:

- Two writers race; both build a new snapshot from the same parent.
- Whoever's CAS lands first wins.
- The loser observes the conflict, rebases, retries.

Because snapshots are catalog rows and CAS is a single Postgres `UPDATE …
WHERE current_snapshot_id = $expected`, the cost is one round-trip — not a
distributed quorum.

## Manifests

A manifest is the catalog's record of which extents make up a snapshot,
plus the per-extent stats that the planner reads at stage 1 of the
[pruning cascade](/concepts/the-pruning-cascade).

The shape is Iceberg-style:

- One row per extent.
- Columns: `(table_id, extent_id, snapshot_id, byte_range, time_range,
  row_count, column_stats jsonb, present_paths bytea, schema_version)`.
- Indexed for fast lookup by `(table_id, snapshot_id, time_range)`.

A query at snapshot N gets the manifest rows whose `snapshot_id` is in
N's ancestry. From there, time-range and column-stats predicates trim the
list before any object-store I/O happens.

## How an ingest produces extents

The path from JSON-on-the-wire to a queryable extent:

1. **Frontend.** REST, OTLP, Kafka, or file-drop coerces input into Arrow
   `RecordBatch`es against the table's catalog-stored schema.
2. **Staging buffer.** A per-table buffer group-commits writes from many
   concurrent ingesters. Group-commit means a single `flush` produces one
   well-sized extent rather than a flood of tiny ones.
3. **Commit coordinator.** When the buffer flushes, the coordinator writes
   the extent to object storage, builds the manifest entry, and proposes
   the new snapshot via CAS.
4. **Cleanup.** On CAS conflict, the coordinator rebases and retries. The
   extent it wrote is now orphaned; a periodic GC sweeps orphans by
   matching object-store keys against the manifest.

Group-commit is the reason "many small writers" doesn't fragment object
storage. Hundreds of services hammering `/v1/ingest` produce dozens of
healthy extents, not millions of tiny files.

## Schema evolution

A schema change — adding a column, widening a type — produces extents
under the new schema. Old extents stay under the old schema. Reads
null-fill missing columns at decode time, so a query that references the
new column simply sees `null` for rows from old extents.

Mid-batch evolution is handled by force-flushing the staging buffer
before appending a new-schema batch. History is never rewritten.

The hard rule: **schema only widens.** Add columns, widen types, demote
typed-but-polymorphic fields back to the `dynamic` overflow column.
Never narrow, never delete, never rewrite.

## Compaction and retention

Background workers do two things to extents:

- **Compaction** merges many small adjacent extents into fewer larger ones,
  preserving the original snapshots in the manifest history. Queries plan
  against the compacted layout; compacted extents prune better.
- **Retention** drops manifest rows older than a per-table retention
  policy and, separately, GCs the underlying object-store files. Because
  retention is per-table, you can keep traces for 7 days and audit logs
  for 7 years in the same engine without per-customer ops gymnastics.

Both run as work-unit rows in a catalog table, pulled with `FOR UPDATE
SKIP LOCKED`. Adding capacity is starting another node; that node picks
up the next work unit.

## Where to go next

- The pruning cascade these manifests power: [The pruning cascade](/concepts/the-pruning-cascade).
- The schema model: [Schema model](/concepts/schema-model).
- The data shape that makes ingest fast: [Ingest](/ingest/).
