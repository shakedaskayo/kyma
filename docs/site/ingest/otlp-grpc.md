---
title: OTLP gRPC
description: Point an OpenTelemetry Collector at pensieve's OTLP gRPC endpoint. Logs land in a fixed otel_logs table, auto-created on first export.
---

# OTLP gRPC

pensieve speaks the OpenTelemetry Protocol over gRPC. Point your existing
OTel Collector — or any OTLP-gRPC exporter — at pensieve's OTLP port. Logs
land in the `otel_logs` table in the configured database; the table is
auto-created on first export.

Trace and metric signals are not in the phase-A surface. Only
`ExportLogsService` is implemented; trace and metric exports get a
clean "unimplemented" gRPC error.

Use it for: anything that already emits OTLP, or any pipeline you'd
otherwise stand up the OTel Collector for.

## Configuration

Two environment variables drive the server:

| Variable             | Default     | Notes                                              |
| -------------------- | ----------- | -------------------------------------------------- |
| `PENSIEVE_OTLP_ADDR`     | `off`       | gRPC listen addr. Standard port is `4317`.         |
| `PENSIEVE_OTLP_DATABASE` | `default`   | Target database for all OTLP logs.                 |

Set `PENSIEVE_OTLP_ADDR=0.0.0.0:4317` to enable. With the default `off`,
the OTLP server doesn't bind and the rest of pensieve runs unchanged.

The dev `docker-compose.yml` exposes `4317` and sets `PENSIEVE_OTLP_ADDR`
already, so the path below works out of the box.

## End-to-end example

Wire an OTel Collector to forward logs to pensieve:

```yaml
# otel-collector.yaml
receivers:
  filelog:
    include: [/var/log/app/*.log]

exporters:
  otlp/pensieve:
    endpoint: pensieve:4317
    tls:
      insecure: true            # local dev; use TLS in prod
    sending_queue:
      enabled: true
    retry_on_failure:
      enabled: true

service:
  pipelines:
    logs:
      receivers:  [filelog]
      exporters:  [otlp/pensieve]
```

Or, from your application directly, with an OTLP-gRPC exporter:

```python
from opentelemetry.sdk._logs import LoggerProvider, LoggingHandler
from opentelemetry.sdk._logs.export import BatchLogRecordProcessor
from opentelemetry.exporter.otlp.proto.grpc._log_exporter import OTLPLogExporter
import logging

provider = LoggerProvider()
provider.add_log_record_processor(
    BatchLogRecordProcessor(OTLPLogExporter(endpoint="pensieve:4317", insecure=True))
)
logging.getLogger().addHandler(LoggingHandler(logger_provider=provider))
logging.getLogger().setLevel(logging.INFO)
logging.getLogger(__name__).info("hello pensieve over otlp")
```

A successful export returns the standard
`ExportLogsServiceResponse{}`. If pensieve rows-ingested doesn't match
the export's record count, a `partial_success` is set with the
delta — sending clients see this as a soft retry signal.

## Schema

`otel_logs` is fixed. Auto-created on first export if missing:

| Column            | Type        | Source                                                |
| ----------------- | ----------- | ----------------------------------------------------- |
| `timestamp`       | `timestamp` | `time_unix_nano`, falling back to `observed_time_unix_nano`. |
| `severity_number` | `int`       | OTLP `severity_number` (0 → null).                    |
| `severity_text`   | `string`    | OTLP `severity_text`.                                 |
| `body`            | `string`    | Stringified `body` — primitives stringify; complex bodies serialize to JSON. |
| `service_name`    | `string`    | Pulled out of resource attributes by name.            |
| `trace_id`        | `string`    | Hex-encoded raw bytes.                                |
| `span_id`         | `string`    | Hex-encoded raw bytes.                                |
| `attributes_json` | `string`    | Resource + scope + record attributes merged into one JSON object. |

`attributes_json` is a JSON string today, not a `dynamic` column.
Path-level pruning over OTLP attributes lands when the OTLP receiver
moves to writing into the real `dynamic` type — see
[Schema model](/concepts/schema-model) for the column types and
[Concepts](/concepts/) for the broader plan.

## Failure modes

- **Both timestamps zero.** `timestamp` lands as null. Add an
  `_timestamp` enrichment in your collector pipeline if you need
  guaranteed event time.
- **Database doesn't exist.** OTLP doesn't have a "create database"
  step, so the receiver creates one named `PENSIEVE_OTLP_DATABASE` on
  first export. Idempotent.
- **Receiver disabled.** With `PENSIEVE_OTLP_ADDR=off`, the server logs
  `otlp: disabled` at startup. Clients see TCP refusal — set up
  retries on the exporter side.

OTLP ingest shares the staging buffer and commit coordinator with the
REST path. See [Extents and snapshots](/concepts/extents-and-snapshots)
for what happens after the export call returns.

## Where to go next

- Common rules across ingest paths: [Idempotency and coercion](/ingest/idempotency-and-coercion).
- The REST path, for non-OTLP shapes: [REST / NDJSON](/ingest/rest-ndjson).
- Querying what just landed: [Query](/query/).
