---
title: REST / NDJSON
description: POST /v1/ingest takes NDJSON, auto-creates the table and evolves the schema, and returns the snapshot the rows are visible in.
---

# REST / NDJSON

The default way data lands in pensieve. One HTTP `POST`, one body of NDJSON,
one snapshot. Headers tell the engine which database and table to write
to; the body is one JSON object per line. The table is auto-created on
first write and the schema evolves to fit new fields.

Use it for: web hooks, application logs, anything that already speaks
HTTP, and the entire quickstart path.

## Request shape

```
POST /v1/ingest
X-Database:        <database>      (default: "default")
X-Table:           <table>         (required)
X-Idempotency-Key: <opaque>        (optional, see below)
X-Auto-Create:     true|false      (default: true)
X-Schema-Evolve:   true|false      (default: true)
Content-Type:      application/x-ndjson
```

Body is NDJSON — one JSON object per line, blank lines tolerated. The
body cap is 64 MiB. A request that exceeds it returns `413
body_too_large`; split into smaller batches.

`X-Auto-Create=true` makes the database and table on first write.
`X-Auto-Create=false` returns `404 table_not_found` instead, which is
what you want when callers pre-provision and a typo should fail loud.

`X-Schema-Evolve=true` does a one-pass scan of the body, calls
`ALTER TABLE ADD COLUMN` for unknown fields, then parses against the
widened schema. With evolve off, unknown fields are dropped silently.
The cap is 32 new columns per request — past that, remaining unknown
fields land in the dynamic catch-all `props`. See
[Idempotency and coercion](/ingest/idempotency-and-coercion) for the
type-inference rules.

## End-to-end example

```bash
curl -sS -X POST http://localhost:8080/v1/ingest \
  -H "X-Database: default" \
  -H "X-Table: orders" \
  -H "X-Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/x-ndjson" \
  --data-binary @- <<'EOF'
{"_timestamp":"2026-05-02T10:00:00Z","order_id":"o-1","amount_cents":4200,"status":"placed"}
{"_timestamp":"2026-05-02T10:00:04Z","order_id":"o-2","amount_cents":1850,"status":"placed"}
{"_timestamp":"2026-05-02T10:00:09Z","order_id":"o-1","amount_cents":4200,"status":"shipped"}
EOF
```

Response on success:

```json
{
  "snapshot_id": "0190f3a0-...",
  "extent_count": 1,
  "rows_ingested": 3,
  "bytes_written": 5183,
  "replayed": false
}
```

`snapshot_id` is the catalog snapshot the rows are now visible at. A
query at this snapshot or later sees them.
`replayed` is `true` only when the same `X-Idempotency-Key` was already
applied; the body in that case is ignored entirely and the cached ack
is returned.

The response also carries `X-Request-ID`, set from the inbound header
or generated as a UUID. It's stamped into every server log line for the
request — quote it when filing tickets.

## Schema model

The default-created table has four columns:

| Column   | Type        | Notes                                                          |
| -------- | ----------- | -------------------------------------------------------------- |
| `at`     | `timestamp` | Auto-populated from `at`, `timestamp`, or `_timestamp` if present.|
| `label`  | `string`    | Free-form short kind name.                                     |
| `body`   | `string`    | Primary text payload — full-text-style queries land here.      |
| `props`  | `dynamic`   | Catch-all for anything not promoted to a typed column.         |

Any other top-level field in your NDJSON becomes a new `string` column
on first write (subject to the 32-column cap). Stronger inference comes
from pre-creating the table — `POST /v1/admin/databases/{db}/tables/{tbl}`
will eventually accept a schema body; for now it provisions the default
shape.

## Failure modes

- **Schema drift between lines.** Mid-batch evolution force-flushes
  the staging buffer at the new-schema boundary, then resumes. You see
  one ack with the post-flush snapshot id.
- **Snapshot CAS conflict.** Concurrent writers retry with exponential
  backoff up to 20 attempts. A truly hot table eventually returns
  `500 ingest_failed` with a `Conflict` cause; partition the writers
  across more tables to fix.
- **Idempotency race.** Two concurrent requests with the same key may
  both ingest; the ledger insert ensures only one wins, the loser logs
  a warning, and a small duplicate is accepted. The TTL on the ledger
  is 24 hours.
- **NDJSON parse error.** `400 bad_request_body` with the offending
  decoder error in the message. The whole batch is rejected — no
  partial commits.

## Where to go next

- The cross-cutting rules: [Idempotency and coercion](/ingest/idempotency-and-coercion).
- How an ingest produces an extent: [Extents and snapshots](/concepts/extents-and-snapshots).
- The other ingest paths: [OTLP gRPC](/ingest/otlp-grpc), [Kafka](/ingest/kafka), [File-drop](/ingest/file-drop).
