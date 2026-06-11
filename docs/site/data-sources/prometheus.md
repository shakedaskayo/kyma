---
title: Prometheus
description: The shipped reference data source. Scrapes a /metrics endpoint on a schedule, parses OpenMetrics / Prometheus text exposition, and lands one row per sample in the configured table.
---

# Prometheus

Prometheus is the reference implementation of the
[Data source framework](/data-sources/framework). It does the smallest
useful thing: GET a `/metrics` URL, parse OpenMetrics text format,
emit one row per sample. ~200 lines of code, no cursor, no CDC. Read
this once and the framework's contract is concrete.

`type_id`: `"prometheus"`. Drive model: `Periodic`.

## Configuration

```json
{
  "endpoint": "http://node-exporter:9100/metrics",
  "auth":     { "type": "none" },
  "timeout_ms": 5000
}
```

| Field        | Type       | Notes                                                |
| ------------ | ---------- | ---------------------------------------------------- |
| `endpoint`   | `string`   | `http://` or `https://` URL. Validated at create.    |
| `auth`       | object     | One of `{type: "none"}`, `{type: "bearer", token_ref: "..."}`, or `{type: "basic", username: "...", password_ref: "..."}`. |
| `timeout_ms` | `int`      | Per-request HTTP timeout. Bounded `[100, 60000]`. Default `5000`. |

The config schema is `deny_unknown_fields`; an extra key fails
`validate_config` with the serde error verbatim, so operator typos
fail at `POST` time, not at the next tick.

`token_ref` and `password_ref` are
[secret references](/data-sources/framework#secrets-by-reference) —
typically `"$env:NAME"`.

## Creating a data source

```bash
curl -sS -X POST http://localhost:8080/v1/data-sources \
  -H "Content-Type: application/json" \
  --data-binary @- <<'JSON'
{
  "name": "node-exporter-1",
  "type": "prometheus",
  "target_database": "default",
  "target_table": "metrics",
  "schedule_ms": 15000,
  "config": {
    "endpoint": "http://node-exporter:9100/metrics",
    "auth": {
      "type": "bearer",
      "token_ref": "$env:NODE_EXPORTER_TOKEN"
    },
    "timeout_ms": 5000
  }
}
JSON
```

The response is `{"id": "<uuid>"}`. The first tick fires within
`schedule_ms` of creation; pre-trigger one with
`POST /v1/data-sources/:id/trigger`.

## What lands in your table

Each sample produces one row:

```json
{
  "timestamp": "2026-05-02T10:00:00Z",
  "name":      "http_requests_total",
  "value":     1234.0,
  "labels":    { "method": "GET", "status": "200" }
}
```

| Column      | Type        | Notes                                                  |
| ----------- | ----------- | ------------------------------------------------------ |
| `timestamp` | `timestamp` | The tick's bucketed `scheduled_for` — not the per-sample timestamp from the source. Scrape time wins; trailing per-sample timestamps in the exposition are parsed and discarded. |
| `name`      | `string`    | Metric name as written by the source.                  |
| `value`     | `real`      | Sample value. `NaN`, `+Inf`, `-Inf` land as JSON `null`.|
| `labels`    | `dynamic`   | Map of label-name → label-value. Empty `{}` if none.   |

`HELP` / `TYPE` / `UNIT` lines are consumed and discarded — Prometheus
exposition is mostly samples, and we keep the table to one row shape.

## Auth

Three modes:

- **`{"type": "none"}`** — anonymous. The default.
- **`{"type": "bearer", "token_ref": "$env:..."}`** — `Authorization: Bearer <resolved>`.
- **`{"type": "basic", "username": "...", "password_ref": "$env:..."}`** —
  HTTP Basic with the resolved password.

The data source sends an `Accept` header advertising both OpenMetrics 1.0
and Prometheus 0.0.4 text format, so most exporters serve their preferred
encoding without further config.

## Retry behavior

Inside one tick, transient HTTP failures (`429`, `5xx`, timeout, connect
error) retry up to 3 times with exponential backoff (100 ms × 2^n with
jitter). After that the tick returns `Transient`, which the
[runner](/data-sources/framework#scheduler-and-runner) treats as a tick
failure — the next scheduled tick runs normally. `4xx` other than `429`
is `Permanent` and does not retry within the tick.

## Failure modes

- **Bad scheme.** `validate_config` rejects anything that isn't `http://`
  or `https://`. `400` at create.
- **Endpoint returns HTML.** Parser error → `Permanent`. Data source keeps
  ticking; `last_error` shows the parse line and offset. Fix the URL or
  re-point the exporter.
- **Endpoint returns mid-batch garbage.** Same path. The whole tick is
  rejected — no partial row commits.
- **Token rotates and `$env:NAME` no longer resolves.** Returns `Config`,
  the data source is disabled with `disabled_reason="token resolve: ..."`.
  Update the env var and `POST /resume`.
- **Concurrent ticks.** Bucketed scheduling and the lease lock prevent
  it; if two runners race, the second loses `claim_task` and idles.

## Where to go next

- The trait, registry, and admin API: [Framework](/data-sources/framework).
- How rows become extents: [Extents and snapshots](/concepts/extents-and-snapshots).
- Querying scrape data: [PromQL](/query/promql) (roadmap),
  [SQL](/query/sql), [KQL](/query/kql).
