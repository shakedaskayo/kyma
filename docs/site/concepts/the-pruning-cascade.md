---
title: The pruning cascade
description: How kyma skips 99 % of data on 99 % of queries. Three levels of pruning run before any extent bytes are decoded.
---

# The pruning cascade

Sub-second query latency over a decade of history is not a database
optimization problem. It is a "don't look at the data" problem.

kyma's planner answers most queries by reading almost no data. A query
filtered to one tenant, one error class, and the last four hours touches
maybe 0.01 % of the bytes in the bucket. The cascade is what makes that the
default, not the exception.

## Three levels, in order

```
         Incoming query
              |
              v
   +----------------------+
   | 1. Catalog pruning   |  Postgres query against manifest stats:
   |                      |  time range, min/max per column, present_paths.
   +----------+-----------+  → Candidate extents. 99 %+ eliminated.
              |
              v
   +----------------------+
   | 2. Extent pruning    |  Range-GET extent footers (cached).
   |                      |  Bloom filters, per-column stats, path bitmaps.
   +----------+-----------+  → Candidate blocks.
              |
              v
   +----------------------+
   | 3. Block / index     |  Per candidate block:
   |    pruning           |  - block footer (min/max) confirms;
   +----------+-----------+  - inverted-index posting list for text predicates.
              |              → Exact rows to decode.
              v
   +----------------------+
   | 4. Decode & execute  |  Arrow RecordBatches → DataFusion pipeline.
   +----------------------+
```

## 1. Catalog pruning

The catalog (Postgres) holds Iceberg-style manifests. For every extent —
every chunk of column data on object storage — the manifest carries:

- **Time range** — `min(_timestamp)` and `max(_timestamp)`.
- **Per-column stats** — min, max, null count, distinct estimate.
- **Present columns** — which columns are populated in this extent.
- **Token signatures** — bloom-filter-shaped digests for any tokenized
  string column.

A query like

```kql
otel_logs
| where service_name == "auth-svc"
| where _timestamp > ago(1h)
```

becomes a Postgres query that filters extents by `_timestamp` range and
`service_name` min/max overlap. On a year-old table, that is typically
hundreds or low thousands of extents to consider, of which all but a few
dozen get eliminated before the engine touches the object store.

The single most common reason a query is slow is that it gives this stage
nothing to work with. A query without a time bound, against a column without
useful min/max distribution, plans like a full scan because it *is* a full
scan.

## 2. Extent pruning

For each candidate extent, kyma issues a range-GET for the extent's footer.
Footers are tiny (kilobytes) and cached aggressively, so this stage is mostly
network-free on a warm node.

The footer carries:

- **Bloom filters** for high-cardinality columns. A `where user_id == X`
  predicate eliminates extents whose bloom doesn't contain `X`.
- **Per-column block-level stats** — stricter than the catalog's per-extent
  stats. A column whose extent-min/max says "yes" can have block-level
  stats that say "no" for many of its blocks.
- **Path bitmaps** — for the `dynamic` column, which paths are populated
  in this extent. Queries on `attributes["error.code"]` skip extents that
  never wrote that key.

Surviving extents pass a list of candidate blocks to stage 3.

## 3. Block and index pruning

The smallest unit kyma decodes is a block — typically tens of thousands of
rows in one column. A block isn't decoded until two things happen:

- The block's own footer (min/max) confirms the predicate could match.
- For text predicates, the inverted-index posting list carries at least one
  row id matching the term.

For string columns, the inverted index is what lets `where message contains
"failed connection"` execute as a row-id intersection across two posting
lists, rather than as a substring scan over decompressed strings.

What survives stage 3 is an exact set of (block, row) pairs. That's the
input to decode.

## 4. Decode and execute

Decoded blocks become Arrow `RecordBatch`es. From there it's standard
DataFusion: filter, project, aggregate, join, return. Zero-copy through
Arrow Flight to the client. The expensive part of "execute" is decoding
columns, which is why stage 3's row-precision matters.

## The principle

Each pruning stage does only the work the next stage will demand of it.
Catalog pruning runs entirely in Postgres on rows that already exist; extent
pruning reads cached kilobytes; block pruning reads index entries. Decoding
is the only stage that reads bulk data, and only after three filters
narrowed the input.

Order-of-magnitude reductions compound. Eliminating 99 % at three levels
leaves 0.0001 % of bytes to decode — which is the difference between a query
that takes a second and one that takes a day.

## What slows queries down

- **No time bound.** Stage 1 has nothing to filter on. Always include a
  `where _timestamp > ago(...)` or equivalent.
- **Predicates on unindexed `dynamic` paths.** First write to a `dynamic`
  path is fast; queries on it become bloom-filter hits, but a never-before-
  seen path can't prune at the catalog level.
- **Joins across many extents.** A join with a small left side and a large
  right side stays fast (the small side becomes a build-hash); a join with
  two large sides degrades.

## Where to go next

- How extents become snapshots: [Extents and snapshots](/concepts/extents-and-snapshots).
- The schema model: [Schema model](/concepts/schema-model).
- KQL: [KQL](/query/).
