---
title: Storage format
description: kyma-format-tlm — the on-disk format for telemetry extents. Magic-framed Arrow IPC body, JSON-tail block-stats footer, per-column distinct sets, token index, and the SegmentFormat trait it implements.
---

# Storage format

`kyma-format-tlm` is the shipping implementation of the
`SegmentFormat` trait. It defines the byte-level layout of every
extent kyma writes today. This page is the reference for someone
implementing on top of kyma — building a new format, decoding extents
out-of-band, or planning how the catalog and the format collaborate.

The conceptual companion is
[Extents and snapshots](/concepts/extents-and-snapshots), which covers
what an extent *is* — an immutable, columnar, time-bounded chunk of
table data on object storage. This page covers what an extent
*looks like on disk*.

## The trait surface

The format trait — defined in `crates/kyma-core/src/segment_format.rs`
— is intentionally small.

- `SegmentFormat` is a factory: `open_extent` and `start_extent`.
- `ExtentReader` exposes `metadata()`, `pruned_blocks(predicate)`, and
  `read_block(block, projection)`.
- `ExtentWriter` exposes `append(batch)` and `finish()`.
- `BlockPredicate` is a small lowered predicate language the format
  evaluates against block-level stats — `TimeRange`, `Equals`,
  `InSet`, `StringHas`, `DynamicPathExists`, `DynamicPathEquals`,
  plus `And` / `Or` / `Not`.

Any new format swaps in by implementing those four traits. Catalog,
planner, executor, and ingest never name a concrete format type.
This is Invariant 4 — see [the five invariants](/concepts/the-five-invariants).

## Extent layout

An extent is one immutable object on object storage. The current
on-disk version is **v2**:

```text
MAGIC_V2  ||  arrow_ipc_bytes  ||  block_stats_json  ||  stats_len u32 LE  ||  MAGIC_V2
```

Where:

- **`MAGIC_V2`** is the byte string `KYMA\x02`. It appears at both
  the start of the object and the very end. The trailing copy lets a
  reader walk *backwards* from the file end to find the footer
  without parsing the Arrow IPC body first.
- **`arrow_ipc_bytes`** is a complete Apache Arrow IPC *file* — a
  schema header, a stream of record batches, and an Arrow-native
  footer with batch offsets. Each Arrow record batch corresponds to
  one *block* in `SegmentFormat` terms.
- **`block_stats_json`** is a JSON array, one entry per block,
  carrying per-column min/max/null-count for that block. This is
  what `pruned_blocks` evaluates predicates against.
- **`stats_len`** is the JSON byte length encoded as a little-endian
  u32, so the footer-walk reader knows where the JSON starts.

A v1 extent — `KYMA\x01` magic — has the magic followed directly by
Arrow IPC bytes, no trailing footer. v1 extents stay readable; their
`pruned_blocks` falls back conservatively to "return every block."

## Reading an extent

The reader walks the object once, end to start, then start to end:

1. Range-GET the object (or fetch fully, for now — multi-part GET is
   a follow-up). Verify leading magic.
2. If `MAGIC_V2`: read the trailing magic, then `stats_len`, then
   peel off the JSON footer and parse it into `Vec<BlockStats>`.
3. The middle slice is the Arrow IPC body. Open it with
   `arrow::ipc::reader::FileReader`, which walks Arrow's own footer
   to enumerate record batches.
4. Each record batch becomes one block, indexed by `BlockId(u32)`.

`pruned_blocks` walks the parsed stats list, evaluating the
`BlockPredicate` against each block's `BlockStats` and returning the
surviving block ids. `read_block` decodes one specific batch and
projects to the requested column set. Predicate evaluation is
conservative: if the format can't be sure a block fails the
predicate, the block is kept. Unsupported predicates (notably
`StringHas` against a non-text-indexed column, and the
`DynamicPath*` family in v2) are treated as "always keep" — the exec
layer applies them above the scan.

## Block-level stats

The `BlockStats` struct — defined in `block_stats.rs` — carries one
entry per block. Each entry has a row count and a map keyed by
column *name* (not index), so a future `ALTER TABLE ADD COLUMN` that
shifts indices doesn't invalidate stored stats.

Per-column entries are tagged unions (`ColStat::Ts`, `Int64`, `Str`),
each carrying:

- **`min`, `max`** — typed bounds for the values written to that
  block.
- **`nulls`** — null count for the column in that block.

Type coverage is intentionally narrow: timestamps (nanoseconds since
Unix epoch), `Int32`/`Int64` (`Int32` widens to `Int64` for storage),
and `Utf8`/`LargeUtf8`. Columns of any other type are omitted from
the stats; the reader treats omitted columns as "no information,
keep the block."

String stats are size-bounded. Any value longer than
`STRING_MINMAX_CAP` (256 bytes) disables string min/max for that
(block, column) pair — correctness preserved, pruning skipped. This
keeps the JSON footer bounded even when one log line carries a
multi-kilobyte payload.

## Per-column statistics — written to the catalog

Block-level stats live inside the extent. *Extent-level* stats live
in the catalog manifest, computed during write and returned in
`ExtentWriteResult.column_stats`. The writer maintains two
per-indexable-column sets across all blocks:

- **Distinct values.** For string and integer columns, a hash set of
  values seen anywhere in the extent. Capped at `DISTINCT_SET_CAP`
  (1 000) — past the cap, the column's distinct set is dropped to
  `null` and equality pruning at this extent degrades to no pruning
  (correct, just slower).
- **Token set.** For string columns, a hash set of word-level tokens
  (lowercased, ASCII-alphanumeric chunks ≥ 2 chars). Capped at
  `TOKEN_SET_CAP` (10 000) — past the cap, dropped to `null` and
  text-search pruning at this extent is disabled (DataFusion still
  applies the predicate above the scan).

Both sets are sorted at finish time and emitted as JSON of the shape:

```json
{
  "service_name": {
    "distinct": ["auth-svc", "payments-svc"],
    "tokens":   null
  },
  "message": {
    "distinct": null,
    "tokens":   ["connection", "failed", "timeout", ...]
  }
}
```

The catalog stores this in the manifest's `column_stats jsonb` column
for stage-1 [pruning](/concepts/the-pruning-cascade). A query like
`where service_name == "auth-svc"` becomes a Postgres predicate over
this JSON; a query like `where message contains "timeout"` becomes
the equivalent token-set check.

## Time bounds

The writer tracks `min_timestamp_nanos` and `max_timestamp_nanos`
across every appended batch, scanning whichever schema column has
the `Timestamp(Nanosecond, _)` Arrow type. These are returned in
`ExtentWriteResult` and stored on the manifest row alongside the
extent itself. They are the cheapest pruning signal in the cascade —
any query with a `_timestamp` bound starts by eliminating extents
whose `[min_ts, max_ts]` doesn't overlap the bound.

## The dynamic-column path bitmap

A `dynamic` column carries CBOR-encoded values whose paths vary per
row — `attributes["http.method"]`, `attributes["error.code"]`, and
so on (see [Dynamic and vectors](/concepts/dynamic-and-vectors)).
For pruning, the relevant signal is *which paths exist anywhere in
the extent*. The catalog manifest carries a `present_paths` bitmap
per extent; a query like `where attributes["error.code"] == "X"`
skips every extent that never wrote `error.code`.

Phase A — the format that ships today — does not yet populate
`present_paths`; the field exists in `ExtentWriteResult` and
returns an empty list. Phase B replaces the Arrow IPC body with
custom column encoders and lights up path tracking as a first-class
writer pass. The catalog shape is final; only the writer's
contribution to it changes.

## Phase B — the custom column format

The trait contract and the magic-framed envelope are stable. What
will change is the body between the magic markers. Phase B replaces
Arrow IPC with per-type column encoders:

- Delta-of-delta for nanosecond timestamps.
- Gorilla compression for floats.
- Inverted-index posting lists for tokenized string columns —
  promoting the current per-extent token set to a per-block
  posting list, which lights up stage 3 of the pruning cascade.
- CBOR + path bitmap for `dynamic` columns.

The footer grows to carry block offsets per *column* (not per row
group, as in Arrow IPC), so projections do bytes-precise range-GETs.
The trailing magic and the JSON stats footer survive — keeping the
"walk the file end-to-start" reader logic unchanged.

## What the catalog gets back from `finish()`

`ExtentWriteResult` is the contract between the format and the
catalog. Every field below is consumed by manifest insert:

- `extent_id` — fresh v4 UUID, opaque outside the catalog.
- `object_path` — the object-store key (under the format's
  configured `path_prefix`).
- `byte_size` — total object size; used for compaction sizing.
- `row_count`, `block_count` — extent-level counters.
- `min_timestamp_nanos`, `max_timestamp_nanos` — time bounds.
- `present_paths` — dynamic-column path bitmap (Phase B).
- `column_stats` — the JSON described above; stored verbatim in
  the manifest's `column_stats jsonb` column.

The catalog's CAS publish is what turns these fields into a visible
snapshot. Until then, the extent is an orphan object that the GC
work-unit will eventually sweep.

## Where to go next

- The trait definitions: `crates/kyma-core/src/segment_format.rs`.
- The format implementation: `crates/kyma-format-tlm/src/`.
- The query side that consumes these footers:
  [The pruning cascade](/concepts/the-pruning-cascade) and
  [Pruning and performance](/query/pruning-and-performance).
- The roadmap that ties Phase B to a delivery slice:
  [Slice roadmap](/architecture/slice-roadmap).
