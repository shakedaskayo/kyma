---
title: File-drop
description: Drop NDJSON files into an object-store prefix; kyma polls, hashes, and ingests. SHA256-keyed idempotency makes re-scans free.
---

# File-drop

Drop NDJSON files into a configured object-store prefix and kyma picks
them up. Each file's SHA256 is the idempotency key — re-running a scan
against the same file is a no-op, and the original files stay in the
bucket as a replayable audit trail.

Use it for: nightly batch jobs, off-Kafka producers, anything that
already writes files, and any pipeline where keeping the raw drop is
useful for debugging.

## Configuration

The watcher is opt-in. With `KYMA_FILEDROP_ENABLED=1`, a worker starts
that polls the configured prefixes on the kyma instance's own object
store (the same bucket the engine writes extents to).

| Variable                          | Default     | Notes                                                                         |
| --------------------------------- | ----------- | ----------------------------------------------------------------------------- |
| `KYMA_FILEDROP_ENABLED`           | `0`         | Set `1` to start the watcher.                                                 |
| `KYMA_FILEDROP_PREFIXES`          | `ingest`    | Comma-separated list of prefixes to scan each tick.                           |
| `KYMA_FILEDROP_PREFIX`            | _(legacy)_  | Single-prefix form. Used only if `KYMA_FILEDROP_PREFIXES` is unset.           |
| `KYMA_FILEDROP_POLL_SECS`         | `5`         | Scan interval, applied to every prefix.                                       |
| `KYMA_FILEDROP_AUTO_CREATE`       | `1`         | Auto-create the database and table on first file. Set `0` to require pre-existing tables. |
| `KYMA_FILEDROP_SCHEMA_EVOLVE`     | `1`         | `ALTER TABLE ADD COLUMN` for unknown top-level keys.                          |
| `KYMA_FILEDROP_DELETE_AFTER_INGEST`| `0`        | Delete the source object after a successful, non-replayed ingest.             |

Multiple prefixes let one kyma instance host watchers for many
independent pipelines without spawning N worker tasks. Each prefix is
scanned in turn per tick, in the order given.

## Path convention

Files are routed to a target table by path:

```
{prefix}/{database}/{table}/{filename}
```

For the default prefix `ingest`, an object at
`ingest/default/events/2026-05-02-10.ndjson` lands rows in
`default.events`. Nested subdirectories under `{table}/` are tolerated
(the full tail is the filename for parsing). Anything that doesn't
match the four-part shape is logged at debug and skipped.

The current MVP supports `.ndjson`, `.jsonl`, and `.json` extensions.
CSV and Parquet are tracked follow-ups.

## End-to-end example

With kyma running and the watcher enabled (the dev `docker-compose.yml`
sets `KYMA_FILEDROP_ENABLED=1` and uses MinIO):

```bash
# Drop a file under ingest/default/orders/.
mc cp orders.ndjson kyma/ingest/default/orders/2026-05-02-10.ndjson
```

Within `KYMA_FILEDROP_POLL_SECS`, a log line confirms the ingest:

```
INFO kyma_ingest_filedrop: filedrop: ingest complete
  path=ingest/default/orders/2026-05-02-10.ndjson rows=4321 replayed=false
```

Re-uploading the same file is a no-op:

```
INFO kyma_ingest_filedrop: filedrop: ingest complete rows=0 replayed=true
```

…because the SHA256 of the bytes is already in the idempotency ledger.
Modify a single byte and the file ingests again as a fresh extent.

## Schema

Files are parsed with the shared NDJSON path used by REST and Kafka.
Auto-create runs the same default schema (`at, label, body, props`)
on first file. Schema evolve runs a one-pass scan of the file for
unknown top-level keys and `ALTER`s the table before parsing — capped
at the same 32-new-columns-per-request limit. Coercion rules are the
same too: see [Idempotency and coercion](/ingest/idempotency-and-coercion).

Empty files still record an idempotency-ledger entry so the next scan
doesn't re-download them.

## Failure modes

- **Bad path layout.** Files outside the
  `{prefix}/{database}/{table}/...` convention are skipped with a
  debug log. The watcher does not crash on unexpected files; it just
  ignores them.
- **Bad NDJSON.** The file is logged and skipped. The ledger isn't
  updated, so it'll retry on the next tick — fix the source and the
  next scan picks it up.
- **Object-store outage.** A failed `list` for one prefix logs and
  skips that prefix for the tick; the others still run.
- **Delete-after-ingest race.** If
  `KYMA_FILEDROP_DELETE_AFTER_INGEST=1` and the delete fails after a
  successful ingest, the file stays — the next scan sees it as a
  replay and the ledger no-ops cleanly. No double-ingest.

The watcher does not parallelize across prefixes on purpose. A single
file-drop watcher is I/O-bound on the bucket; serial scanning keeps
per-prefix logs deterministic and avoids head-of-line blocking on slow
listings.

## Where to go next

- The SHA256 / idempotency-key path other ingests share: [Idempotency and coercion](/ingest/idempotency-and-coercion).
- For higher-throughput, lower-latency pipes: [Kafka](/ingest/kafka), [REST / NDJSON](/ingest/rest-ndjson).
- How an ingest produces an extent: [Extents and snapshots](/concepts/extents-and-snapshots).
