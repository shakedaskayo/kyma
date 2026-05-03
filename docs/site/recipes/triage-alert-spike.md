---
title: Triage an alert spike
description: An error rate that doubled in the last fifteen minutes. Bucket by time, group by error code, project the signal that explains it.
---

# Triage an alert spike

## The problem

Your alerting tool just paged: error rate doubled fifteen minutes ago.
You don't know which service, which code path, or whether it's a real
spike or a single noisy customer. The pager wants an answer in five
minutes. KQL gets you there in three queries.

## The schema

Same `otel_logs` table the [debug-prod-incident](/recipes/debug-prod-incident)
recipe uses. This recipe leans more on `bin()` for time-bucketing and on
[`dynamic`-column access](/concepts/dynamic-and-vectors) for OTLP
attributes that don't always land as typed columns:

| Column                              | Type        | Notes                                       |
| ----------------------------------- | ----------- | ------------------------------------------- |
| `_timestamp`                        | `timestamp` |                                             |
| `severity_text`                     | `string`    | `INFO` / `WARN` / `ERROR`.                  |
| `service_name`                      | `string`    |                                             |
| `attributes["error.code"]`          | dynamic     | Application-level error code, if emitted.   |
| `attributes["http.status_code"]`    | dynamic     | HTTP status, when present.                  |
| `attributes["http.route"]`          | dynamic     | The matched route pattern.                  |

If your service doesn't emit `error.code`, replace the path with whatever
your apps do emit — `attributes["err.kind"]`, `attributes["exception.type"]`,
etc.

## The queries

### 1. Confirm the spike is real

Bucket the error rate by minute, last hour. Eyeball where it jumps.

```kql
otel_logs
| where _timestamp > ago(1h)
| where severity_text == "ERROR"
| extend bucket = bin(_timestamp, 1m)
| summarize errors = count() by bucket
| order by bucket asc
```

A genuine spike doubles or triples in two or three contiguous buckets.
A "spike" that's one bucket high and the next bucket back to baseline
is usually a flush or a job retry, not a customer-affecting incident.

### 2. Localize to a service

Same time bucket, group by service.

```kql
otel_logs
| where _timestamp > ago(15m)
| where severity_text == "ERROR"
| summarize errors = count() by service_name
| order by errors desc
| take 5
```

If one service holds 80%+ of the spike, your suspect is obvious. If the
distribution is even, the cause is shared infrastructure — a database,
a broker, a downstream API.

### 3. Group by error code in the suspect

```kql
otel_logs
| where _timestamp > ago(15m)
| where severity_text == "ERROR"
| where service_name == "payments-svc"
| extend code = attributes["error.code"]
| summarize n = count() by code
| order by n desc
| take 10
```

Now you know not just *where* but *what* — `STRIPE_TIMEOUT` × 412 vs.
`USER_DECLINED` × 8. The first is an incident; the second is a
customer-segmenting pattern, not a regression.

## What you should see

Query 1 — the buckets that triggered the alert:

| bucket                 | errors |
| ---------------------- | ------ |
| 2026-05-03T13:46:00Z   | 14     |
| 2026-05-03T13:47:00Z   | 12     |
| 2026-05-03T13:48:00Z   | 89     |
| 2026-05-03T13:49:00Z   | 152    |
| 2026-05-03T13:50:00Z   | 178    |

Spike starts at 13:48Z. Jumped 7×. Real.

Query 3 — the explanation:

| code                | n   |
| ------------------- | --- |
| STRIPE_TIMEOUT      | 412 |
| INVENTORY_LOCKED    | 19  |
| USER_DECLINED       | 8   |

That's an upstream Stripe issue, not a kyma deploy. Reroute to your
PagerDuty's "vendor incident" runbook; this isn't yours.

## Variations

- **Different window:** swap `ago(15m)` for `ago(1h)` or
  `ago(24h)` to widen.
- **Compare to last week:** run query 1 twice with two time bounds, then
  compare visually. The CTE form using SQL is cleaner if you do this
  often — see [SQL](/query/sql).
- **Per-customer:** add `extend customer = attributes["customer.id"]`
  before `summarize`. Confirms or refutes the "noisy customer" theory.
- **By route:** replace the group-by column with
  `attributes["http.route"]` to see which endpoints are failing.
