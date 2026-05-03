---
title: Retention and compaction
description: How kyma keeps storage tidy without rewriting history. Per-table retention policies drop old data; compaction merges small extents into larger ones for better pruning.
---

# Retention and compaction

kyma's storage shape — append-only columnar extents on object storage —
makes ingest fast and queries predictable. It doesn't, by itself, keep
the bucket tidy. Two background workers handle that: **retention**
drops data older than a per-table policy, and **compaction** merges
many small extents into fewer larger ones.

Both run as work-unit rows pulled from a catalog table with
`FOR UPDATE SKIP LOCKED`. Adding capacity is starting another node;
that node picks up the next work unit.

## Retention

A retention policy is per-table, set at table creation time and
adjustable later via `ALTER TABLE`:

```bash
kyma-cli alter-table otel_logs set-retention --days 30
```

The retention sweeper does two things:

- **Drop manifest rows** older than the policy. Catalog-level: the
  affected extents are no longer reachable from any active snapshot.
- **GC the underlying object-store files**, but only after a grace
  period. The grace period guarantees in-flight queries against an
  older snapshot still see consistent data; once the snapshot is no
  longer referenced and the grace period has elapsed, the files
  are deleted.

Retention is **per-table**, not per-database. You can keep traces for
7 days and audit logs for 7 years in the same engine without per-tenant
ops gymnastics.

### What retention isn't

Retention doesn't:

- **Mutate extents.** Old extents are removed wholesale; never
  partially rewritten.
- **Rewrite history.** A query at an old snapshot still sees the data
  that snapshot referenced, until the snapshot itself is dropped.
- **Affect ingest.** Retention runs in the background; ingest isn't
  blocked or coordinated with it.

## Compaction

Compaction merges small adjacent extents into larger ones. The
motivation is pruning, not size: a query against 10,000 small extents
runs more catalog queries than a query against 100 fat ones, even if
both store the same bytes.

Compaction policy:

- **Group adjacent extents** with overlapping time ranges.
- **Skip extents under a size threshold** if they're recent — recent
  data tends to be re-queried, and merging it costs more than it saves.
- **Skip extents over a size threshold** unconditionally — those are
  already healthy.

Each compaction run:

1. Reads the candidate extents.
2. Writes a new merged extent.
3. CAS-commits a new snapshot that points at the merged extent and
   removes the originals from the manifest.
4. The originals become eligible for retention GC after the grace
   period.

Compaction is also where tombstone collapse happens.

## Tombstone collapse

Synced tables (from connectors in sync mode) get tombstone rows when
the source deletes a row. A tombstone is `_kyma_op = 'delete'` plus
the primary key.

Default reads filter `_kyma_op != 'delete'`, so tombstones are
invisible by default. They still occupy storage. Compaction collapses
them: a compacted extent drops any row whose latest event for the same
`_kyma_pk` is a delete older than `retention.tombstone_days` (default
30).

Tunable per-table:

```bash
kyma-cli alter-table pg_prod.public.users \
  set-tombstone-retention --days 7
```

A tombstone older than the retention period is also a useful signal:
the row was deleted upstream and won't come back. After collapse, the
storage is reclaimed.

## When to tune

Defaults work for most tables. Reasons to deviate:

- **Long retention but rarely queried older data.** Run a tighter
  compaction policy on older time windows; fewer, larger extents
  cost less catalog space.
- **Very high-cardinality dynamic columns.** Token indices grow with
  unique tokens; aggressive compaction here helps the planner skip
  extents that don't contain a query's terms.
- **Bursty ingest.** A flood of small extents from a misbehaving
  emitter compacts more efficiently if you raise the size threshold
  temporarily.

The defaults are conservative — they work without supervision. Tuning
is for known patterns, not pre-emptive optimization.

## Observability

Both workers emit metrics on the `/metrics` endpoint:

- `kyma_retention_runs_total{table, status}`
- `kyma_retention_rows_dropped_total{table}`
- `kyma_retention_bytes_freed_total{table}`
- `kyma_compaction_runs_total{table, status}`
- `kyma_compaction_extents_merged_total{table}`
- `kyma_compaction_duration_seconds{table}`

Plus structured logs from the work-unit dispatcher. See
[Observability](/concepts/observability) for the full list.

## Where to go next

- Storage shape compaction operates on: [Extents and snapshots](/concepts/extents-and-snapshots).
- Tombstones in sync mode: [Multi-source data](/concepts/multi-source-data).
- The `/metrics` surface: [Observability](/concepts/observability).
