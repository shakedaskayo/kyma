---
title: Ingest
description: Get data into kyma — REST/NDJSON, OTLP, Kafka, file-drop. One write path, four frontends, one cross-cutting page on idempotency and coercion.
---

# Ingest

Four ways to get bytes into kyma. All of them coerce JSON-shaped input
into Arrow batches, hand off to the same staging buffer, and commit
through the same snapshot CAS — so what differs between them is the
wire format and the failure model, not the storage shape they produce.

Pick the path closest to where your data already lives.

<div class="feature-grid">

<div class="feature-card">

### [REST / NDJSON](/ingest/rest-ndjson)

`POST /v1/ingest` with NDJSON body. Auto-creates the table, evolves
the schema mid-batch, returns the snapshot the rows are visible at.
The default for application code, web hooks, and the entire
quickstart path.

</div>

<div class="feature-card">

### [OTLP gRPC](/ingest/otlp-grpc)

OpenTelemetry Protocol over gRPC on port 4317. Logs land in a fixed
`otel_logs` table in the configured database. Phase A is
logs-only — traces and metrics are tracked follow-ups.

</div>

<div class="feature-card">

### [Kafka](/ingest/kafka)

Built-in consumer that maps one topic to one table. Subscribes,
parses NDJSON message bodies, and commits Kafka offsets after each
batch. Use it where Kafka is already the durability layer.

</div>

<div class="feature-card">

### [File-drop](/ingest/file-drop)

Watcher polls an object-store prefix; each file's SHA256 is its
idempotency key. Path convention `{prefix}/{database}/{table}/...`
routes the file to a target table. Re-scans of the same file are
no-ops.

</div>

<div class="feature-card">

### [Idempotency and coercion](/ingest/idempotency-and-coercion)

Cross-cutting reference — JSON-to-Arrow type rules, schema-evolves
mid-batch, the three idempotency-key shapes (REST header, file-drop
SHA256, Kafka offsets). Read this once and the four pages above are
mostly examples.

</div>

</div>

## What's the same across all four

- **One write path.** Frontend bytes become Arrow batches; the staging
  buffer group-commits them; the commit coordinator publishes a new
  snapshot via Postgres CAS. See
  [Extents and snapshots](/concepts/extents-and-snapshots).
- **Schema only widens.** New columns get added; old ones never get
  narrowed or deleted. Old extents stay readable through schema
  changes. See [Schema model](/concepts/schema-model).
- **Idempotent by design.** REST sends a key, file-drop hashes the
  bytes, Kafka tracks offsets. A replayed input never produces a
  duplicate extent at the catalog boundary.

## The write path, in stages

Every frontend — and every [connector](/connectors/) tick — converges on one
pipeline:

1. **Coerce.** JSON-shaped input becomes an Arrow `RecordBatch`. Unknown fields
   evolve the schema; types follow the
   [coercion rules](/ingest/idempotency-and-coercion).
2. **Stage.** Batches land in a per-table **staging buffer** instead of becoming
   one extent each — group-commit amortizes the per-extent cost of small writes.
3. **Flush.** A table flushes on the first trigger it hits: `KYMA_FLUSH_MAX_ROWS`
   (8 000), `KYMA_FLUSH_MAX_BYTES` (16 MiB), or `KYMA_FLUSH_MAX_AGE_MS` (50 ms).
4. **Commit.** The **commit coordinator** collects flushes within a
   `KYMA_COMMIT_WINDOW_MS` (5 ms) window — up to `KYMA_COMMIT_MAX_EXTENTS` (128) —
   into a single snapshot, published via a Postgres compare-and-swap. See
   [Extents and snapshots](/concepts/extents-and-snapshots).

Set `KYMA_STAGING_DISABLED=1` to bypass staging entirely — each ingest request
then becomes exactly one extent (simpler, higher per-write cost). All staging
knobs are in the [environment reference](/reference/env#ingest-staging-group-commit).
