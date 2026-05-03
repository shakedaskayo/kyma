---
title: Kafka
description: Consume NDJSON messages from Kafka topics into kyma tables. One topic maps to one table; offsets are tracked in the catalog.
---

# Kafka

kyma's built-in Kafka consumer subscribes to one or more topics, parses
each message body as NDJSON, and routes records into the configured
target table. Offsets are committed to Kafka after each successful
batch ingest, and tracked in the catalog so a restart resumes from the
last committed position.

Use it for: applications that already produce NDJSON to Kafka, fan-in
of multiple producers, and pipelines where Kafka is the durability
layer.

## Configuration

The consumer is opt-in. With `KYMA_KAFKA_ENABLED=1` and at least one
mapping in `KYMA_KAFKA_TOPICS`, a worker starts inside the kyma
process at boot.

| Variable                     | Default          | Notes                                                            |
| ---------------------------- | ---------------- | ---------------------------------------------------------------- |
| `KYMA_KAFKA_ENABLED`         | `0`              | Set `1` to start the consumer.                                   |
| `KYMA_KAFKA_BROKERS`         | `localhost:9092` | Comma-separated bootstrap list.                                  |
| `KYMA_KAFKA_GROUP`           | `kyma-ingest`    | Consumer group id.                                               |
| `KYMA_KAFKA_TOPICS`          | _(empty)_        | `topic1:db1.tbl1,topic2:db2.tbl2` — required for the consumer to start. |
| `KYMA_KAFKA_BATCH_SIZE`      | `500`            | Flush a topic's pending bucket at this many messages.            |
| `KYMA_KAFKA_BATCH_TIMEOUT_MS`| `500`            | Age-trigger flush; flushes any non-empty bucket on tick.         |

`KYMA_KAFKA_TOPICS` is the only thing you usually have to pick. The
mapping syntax is `topic:database.table` per entry; entries that don't
match are skipped at startup with a warning.

The consumer disables Kafka's auto-commit, runs the cooperative-sticky
assignor, and uses a tight 6 s session / 2 s heartbeat so a restarted
node rejoins the group quickly instead of stalling in
`CompletingRebalance`.

## End-to-end example

Boot kyma against a Redpanda or Kafka broker, with a topic-to-table
map:

```bash
docker run --rm --net=host \
  -e KYMA_CATALOG_URL=postgres://kyma:kyma_dev@localhost:5433/kyma \
  -e KYMA_KAFKA_ENABLED=1 \
  -e KYMA_KAFKA_BROKERS=localhost:9092 \
  -e KYMA_KAFKA_TOPICS=app.events:default.events \
  ghcr.io/shaked/kyma:latest
```

Produce NDJSON-shaped messages — one JSON object per Kafka message,
no newline required:

```bash
echo '{"_timestamp":"2026-05-02T10:00:00Z","service_name":"checkout","message":"order placed"}' \
  | rpk topic produce app.events
```

After at most `KYMA_KAFKA_BATCH_TIMEOUT_MS`, the message is in
`default.events` and queryable. Sustained producers pack into the
configured batch size; bursty producers pack into whatever the
timeout-driven flush picks up.

The consumer logs one line per committed batch:

```
INFO kyma_ingest_kafka: kafka batch committed topic=app.events msgs=500 rows=500
```

## Message schema

Each Kafka message body is treated as one NDJSON line — i.e., one JSON
object — against the target table's schema. Auto-create and
schema-evolve are not run for Kafka; the target table must exist, and
unknown fields are dropped silently. Pre-create the table over REST
before pointing producers at the topic:

```bash
curl -X POST http://localhost:8080/v1/admin/databases/default/tables/events
```

Coercion rules — type promotion, vector columns, dropped fields — are
the same as for the REST path. See
[Idempotency and coercion](/ingest/idempotency-and-coercion).

Multi-line message bodies (your producer wrote NDJSON, not a single
object) are accepted and become one batch per message.

## Failure modes

- **Target table missing.** The batch is dropped with a warning;
  Kafka offsets are not advanced. Pre-create the table or fix the
  mapping and the consumer will pick up from the same offset on the
  next poll.
- **NDJSON parse failure on a batch.** The whole batch is dropped
  (logged) and offsets advance — kyma keeps moving rather than
  blocking the whole topic on one bad producer. Source the bad
  message from the producer side.
- **kyma ingest failure.** The batch is *not* cleared; the next flush
  retries from the same in-memory bucket. Offsets stay where they
  were, so a crashed kyma re-consumes the unflushed window from Kafka
  on restart.
- **Restart.** kyma seeks to the last Kafka-committed offset. The
  current MVP gives at-least-once at the catalog boundary; on rare
  crash windows a few messages may re-ingest. True
  exactly-once — extent commit and Kafka offset commit in one
  Postgres transaction — is tracked work; see
  [Extents and snapshots](/concepts/extents-and-snapshots) for the
  CAS-commit shape it'd extend.

## Where to go next

- Cross-cutting rules: [Idempotency and coercion](/ingest/idempotency-and-coercion).
- Pre-creating tables and the admin surface: [REST / NDJSON](/ingest/rest-ndjson).
- File-based fan-in for slower producers: [File-drop](/ingest/file-drop).
