---
title: Debug a prod incident across services
description: Three KQL queries take you from "checkout broke" to a single trace ID spanning three services. Time bound, service equality, top-N rank — the textbook pruning shape.
---

# Debug a prod incident across services

## The problem

A user filed a bug saying "checkout broke around 14:32." You don't know
which service to blame. You have OTLP logs flowing into kyma from every
service. Three KQL questions get you from "something happened" to a
specific request that exposes the failure.

## The schema

This recipe assumes the auto-created `otel_logs` table populated by
your services' OTLP exporters via the OTLP frontend. The columns you'll
use:

| Column            | Type        | Source                              |
| ----------------- | ----------- | ----------------------------------- |
| `_timestamp`      | `timestamp` | OTLP `time_unix_nano`.              |
| `severity_text`   | `string`    | OTLP `severity_text` (e.g., `ERROR`). |
| `service_name`    | `string`    | OTLP resource attribute.            |
| `body`            | `string`    | OTLP log body.                      |
| `trace_id`        | `string`    | OTLP `trace_id`, present on logs from a traced request. |

If you're shipping in non-OTLP logs, ensure `service_name` and
`trace_id` exist as typed columns; otherwise [promote them from
dynamic](/concepts/dynamic-and-vectors) before running the recipe.

## The queries

### 1. Find the noise

Bound by time, filter to errors, count by service.

```kql
otel_logs
| where _timestamp > datetime("2026-05-03T14:30:00Z")
| where _timestamp < datetime("2026-05-03T14:35:00Z")
| where severity_text == "ERROR"
| summarize n = count() by service_name
| order by n desc
```

The service with the most errors is your suspect — usually obvious from
the count.

### 2. Read the suspect's bodies

Same window, filter to the suspect, project the bodies.

```kql
otel_logs
| where _timestamp > datetime("2026-05-03T14:30:00Z")
| where _timestamp < datetime("2026-05-03T14:35:00Z")
| where service_name == "checkout-svc"
| where severity_text == "ERROR"
| project _timestamp, trace_id, body
| take 20
```

Skim the bodies; pick a representative `trace_id`.

### 3. Follow the trace across services

Drop the service filter; bound by `trace_id`.

```kql
otel_logs
| where trace_id == "abc1234..."
| project _timestamp, service_name, severity_text, body
| order by _timestamp asc
```

You see the request enter `web-svc`, hop to `checkout-svc`, and crash
in the `payments-svc` call. Total time from "got the bug" to
"specific failed call": about a minute.

## What you should see

Query 1 returns one row per service in the window. Realistic shape:

| service_name    | n   |
| --------------- | --- |
| checkout-svc    | 247 |
| payments-svc    | 89  |
| web-svc         | 14  |

Query 3, ordered by time, walks the request through your stack:

| _timestamp                  | service_name | severity_text | body (truncated)                            |
| --------------------------- | ------------ | ------------- | ------------------------------------------- |
| 2026-05-03T14:32:08.124Z    | web-svc      | INFO          | POST /api/checkout received                 |
| 2026-05-03T14:32:08.131Z    | checkout-svc | INFO          | reserving inventory for cart_id=42          |
| 2026-05-03T14:32:08.219Z    | checkout-svc | INFO          | charging via payments-svc                   |
| 2026-05-03T14:32:11.487Z    | payments-svc | ERROR         | upstream timeout: stripe-api 30000ms        |
| 2026-05-03T14:32:11.488Z    | checkout-svc | ERROR         | charge failed; rolling back reservation     |
| 2026-05-03T14:32:11.489Z    | web-svc      | ERROR         | 502 Bad Gateway                             |

## Variations

- **Error rate over time:** in query 1, replace `summarize n = count() by service_name` with `extend bucket = bin(_timestamp, 30s) | summarize n = count() by service_name, bucket`.
- **Last hour, no specific window:** replace the two `_timestamp` filters in query 1 with `where _timestamp > ago(1h)`.
- **Compare to baseline:** subtract two `summarize` results — one for the suspect window, one for the same length of time the day before.
- **Different signal:** swap `otel_logs` for `traces` if your spans table is populated. Same shape; the equivalent of `severity_text == "ERROR"` is `status == "ERROR"` on most exporters.
