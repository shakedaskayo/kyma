---
title: Observability
description: How to tell what kyma is doing. Prometheus metrics, request IDs, structured logs, agent run replay, dreaming run history, connector status, and the pushdown_summary that prevents federation from silently degrading.
---

# Observability

kyma exposes four distinct observability surfaces, each aimed at a different audience:

| Surface | Endpoint | Who uses it |
| ------- | -------- | ----------- |
| Prometheus metrics | `GET /metrics` | Operators, alerting systems |
| Agent run replay | `GET /v1/agent/runs/:run_id` | Engineers debugging a wrong answer |
| Dreaming run history | `GET /v1/agent/memory/dreaming/runs` | Engineers reviewing background memory synthesis |
| Connector status | `GET /v1/connectors/:id/status` | Operators of multi-source deployments |
| `pushdown_summary` | Response body on every federated query | Anyone whose federated query was unexpectedly slow |

Structured logs (`tracing` crate) and request-ID correlation round out the picture.

---

## Prometheus metrics

Standard Prometheus exposition at `GET /metrics`. Always public, no auth required — keep this
endpoint network-isolated, exactly like every other Prometheus target in your stack.

Metrics are grouped below by operational concern. Each group notes what you watch and what a
bad value means.

### Query

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_query_duration_seconds` | `language` (kql/sql/promql) | Wall time per query, by language. Rising p99 under steady load means storage or planning pressure. |
| `kyma_query_requests_total` | `language`, `status` | Query volume; `status=error` rate is your query-error SLI. |
| `kyma_query_rows_returned` | — | Histogram of result sizes; outlier queries scan enormous result sets. |
| `kyma_query_budget_exceeded_total` | — | Queries killed by the row/time budget. Spikes indicate runaway scans or missing indexes. |
| `kyma_scan_blocks_scanned_total` | — | Raw blocks touched per query. High value with low `_pruned_total` means poor time-range filtering. |
| `kyma_scan_blocks_pruned_total` | — | Blocks skipped by bloom / time pruning. Ratio `pruned / (pruned + scanned)` is the pruning efficiency; low values suggest stale statistics or poor partitioning. |
| `kyma_scan_extents_listed_total` | — | Catalog lookups per query; scales with table fragmentation. |

**Example PromQL — pruning efficiency:**
```promql
rate(kyma_scan_blocks_pruned_total[5m])
  /
(rate(kyma_scan_blocks_pruned_total[5m]) + rate(kyma_scan_blocks_scanned_total[5m]))
```
A value below 0.8 in production warrants investigation.

### Ingest

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_ingest_rows_total` | `frontend` (rest/otlp/kafka/filedrop), `table` | Rows landed per ingest path. Drop to zero = a writer stopped. |
| `kyma_ingest_bytes_total` | `frontend`, `table` | Bytes written. Combined with rows gives average row size. |
| `kyma_ingest_duration_seconds` | `frontend` | End-to-end ingest latency including staging + commit. |
| `kyma_ingest_idempotency_hits_total` | — | Exact-duplicate rows deduplicated at commit. High rate = client retrying without ETag. |
| `kyma_ingest_idempotency_races_total` | — | Two concurrent writers committed the same idempotency key. High rate = ingest fan-out without coordination. |
| `kyma_kafka_messages_ingested_total` | `topic`, `table` | Kafka consumer progress. |
| `kyma_otlp_log_records_total` | — | OTLP log records received. |
| `kyma_filedrop_objects_processed_total` | — | Files processed by the file-drop frontend. |
| `kyma_filedrop_rows_total` | — | Rows parsed from file-drop objects. |

### Storage upkeep

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_staging_flush_duration_seconds` | — | Time the staging buffer spends in group-commit. P99 above ~200 ms indicates write pressure. |
| `kyma_staging_flushes_total` | — | Number of staging flushes committed. |
| `kyma_staging_flush_waiters` | — | Goroutines waiting on a flush. Sustained > 0 means group-commit is saturated. |
| `kyma_catalog_cas_conflicts_total` | — | Snapshot compare-and-swap retries during commit. High value = ingest contention; two writers racing on the same table. |
| `kyma_commit_batches_total` | — | Commit batches attempted. |
| `kyma_commit_batch_extents` | — | Extents per commit batch; reflects merge efficiency. |
| `kyma_compaction_tasks_total` | — | Compaction tasks completed. |
| `kyma_compaction_tasks_submitted_total` | — | Compaction tasks queued. If `submitted` >> `completed` for sustained periods, the compactor is falling behind. |
| `kyma_compaction_bytes_in` | — | Bytes read by the compactor. |
| `kyma_compaction_bytes_out` | — | Bytes written by the compactor. Ratio `out/in` close to 1 = no meaningful compression gain. |
| `kyma_compaction_duration_seconds` | — | Time per compaction task. |
| `kyma_retention_extents_soft_deleted_total` | — | Extents soft-deleted by the retention sweeper. Not advancing = retention not running or policy not matched. |
| `kyma_physical_gc_objects_deleted_total` | — | Objects physically removed from object storage. |
| `kyma_physical_gc_objects_delete_failed_total` | — | GC failures. Persistent non-zero = permission or network issue against object storage. |

### Connectors

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_connector_cursor_age_seconds` | `name`, `table` | Age of the sync cursor — how far the connector is behind the source. Rising = a sync source falling behind. Alert if > your RPO. |
| `kyma_connector_rows_ingested_total` | `name`, `table` | Rows synced from each source-table. |
| `kyma_connector_ticks_total` | `name` | Connector polling or CDC event cycles. Flat for a sync connector = it has stopped. |
| `kyma_connector_errors_total` | `name` | Hard errors per connector. Any sustained non-zero rate warrants investigation. |
| `kyma_connector_duration_seconds` | `name` | Time spent per connector tick. |
| `kyma_connector_last_success_timestamp_seconds` | `name` | Unix timestamp of last successful tick. Alert if `time() - kyma_connector_last_success_timestamp_seconds > threshold`. |

**Example PromQL — connectors that haven't succeeded in 5 minutes:**
```promql
time() - kyma_connector_last_success_timestamp_seconds > 300
```

### Agent and MCP

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_mcp_tool_calls_total` | `tool` | MCP tool calls dispatched by the agent. High rate of a single tool = agent looping. |
| `kyma_mcp_tool_results_total` | `tool`, `status` | MCP tool results returned. `status=error` rate per tool surfaces bad skill definitions. |
| `kyma_explore_search_requests_total` | — | `/explore/search` requests. |
| `kyma_explore_search_executed_total` | — | Searches that reached the query engine. |
| `kyma_explore_search_duration_seconds` | — | Search latency. |
| `kyma_explore_search_rows_returned` | — | Result rows per search. |
| `kyma_explore_search_cap_hits_total` | — | Searches that hit the result-row cap; indicates the cap may need raising for this deployment. |
| `kyma_explore_search_per_source_errors_total` | — | Per-source errors during multi-source search. |
| `kyma_explore_search_sources_resolved` | — | Number of sources each search fanned out to. |

### HTTP layer

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `kyma_http_errors_total` | `method`, `path`, `status` | HTTP 4xx/5xx responses. |
| `kyma_flight_do_get_total` | — | Arrow Flight DoGet requests (used by distributed scan). |
| `kyma_flight_serve_extent_total` | — | Flight extents served. |

---

## Request ID correlation

Every kyma HTTP response carries an `x-request-id` header — either echoed from the request if
the client supplied one, or generated as a fresh ULID. Structured log lines emitted during that
request carry the same ID. This lets you correlate a slow or failed API call to the server-side
log trace without any additional instrumentation.

```bash
# Capture the request ID from a query response
RID=$(curl -si http://localhost:8080/v1/query -d '{"sql":"SELECT 1"}' | grep -i x-request-id | awk '{print $2}' | tr -d '\r')

# Find all log lines for that request
journalctl -u kyma --no-pager | grep "$RID"
```

---

## Structured logs

kyma uses the `tracing` crate with structured fields. Set `RUST_LOG` to control verbosity:

```bash
# Production — warn + error only
RUST_LOG=warn kyma serve

# Debug a specific component
RUST_LOG=kyma_connectors=debug,kyma_ingest_core=info,warn kyma serve
```

Log lines include `request_id`, `table`, `connector`, and other relevant fields so they can be
joined against metrics or correlated across services.

---

## Agent run replay

Each `/v1/agent/ask` invocation persists to the `agent_runs` catalog table. A run carries the
question, the model, the full event log, and the resulting tokens / wall time / status.

```bash
curl http://localhost:8080/v1/agent/runs/01HZABCDEF...
```

```json
{
  "run_id": "01HZABCDEF...",
  "question": "which service errored most in the last hour?",
  "model_id": "claude-sonnet-4-5",
  "started_at": "2026-05-03T14:22:08Z",
  "finished_at": "2026-05-03T14:22:10Z",
  "status": "completed",
  "events": [
    { "kind": "thinking_delta", "text": "..." },
    { "kind": "tool_call", "tool": "run_sql", "arguments": {} },
    { "kind": "tool_result", "rows": [] },
    { "kind": "answer_delta", "text": "..." },
    { "kind": "answer_final", "text": "..." }
  ],
  "usage": { "tokens_in": 940, "tokens_out": 320, "tools_called": 2 }
}
```

The use case is "the agent gave a weird answer — what did it actually do?" Open the run; the
event log is everything.

---

## Dreaming run history

Background memory synthesis runs (dreaming) are persisted separately. You can list and inspect
them:

```bash
# List recent dreaming runs
curl http://localhost:8080/v1/agent/memory/dreaming/runs

# Inspect a specific run
curl http://localhost:8080/v1/agent/memory/dreaming/runs/01HZABCDEF...
```

Each run records what memories were synthesized, which sessions contributed, and the resulting
memory IDs written to the memory store. Use this to diagnose why a memory is stale or why a
background synthesis produced an unexpected result.

---

## Connector status

For each connector (federation, sync, or both):

```bash
curl http://localhost:8080/v1/connectors/<id>/status
```

```json
{
  "id": "01H...",
  "type": "postgres",
  "mode": "both",
  "source": {
    "reachable": true,
    "version": "PostgreSQL 16.2",
    "last_health_check": "2026-05-03T14:22:00Z"
  },
  "federation": {
    "status": "healthy",
    "pool_in_use": 2,
    "pool_max": 10,
    "p50_query_ms": 14,
    "p99_query_ms": 230,
    "queries_total_5m": 1240,
    "errors_5m": 0,
    "last_error": null
  },
  "sync": {
    "status": "streaming",
    "phase": "streaming",
    "lag_seconds": 4,
    "last_event_at": "2026-05-03T14:21:56Z",
    "events_per_sec": 1200,
    "rows_synced": 5240000,
    "schema_drift": [],
    "last_error": null
  }
}
```

### kyma_connector_health table

The same data is exposed as the `kyma_connector_health` built-in table so you can query it like
any other data, chart it on dashboards, or set up alerts as KQL rules.

Key fields:

| Field | Meaning | Bad value |
| ----- | ------- | --------- |
| `lag_seconds` | Seconds the sync cursor is behind the source. Same as `kyma_connector_cursor_age_seconds`. | > your RPO target |
| `schema_drift` | Array of detected column / type changes not yet reconciled. | Non-empty — connector will skip affected columns until resolved |
| `errors_5m` | Hard errors in the last 5 minutes (federation path). | Any non-zero, especially if rising |
| `last_error` | Most recent error message. | Non-null for a connector that should be healthy |
| `pool_in_use` | Federation connection pool utilization. | Close to `pool_max` — federation queries will queue |

**Example KQL — connectors falling behind on sync:**
```kql
kyma_connector_health
| where mode != "federation"
| where lag_seconds > 30
| project name, table, lag_seconds, last_event_at, last_error
```

**Example KQL — federation pool pressure:**
```kql
kyma_connector_health
| where mode != "sync"
| where pool_in_use * 1.0 / pool_max > 0.8
| project name, pool_in_use, pool_max, p99_query_ms
```

---

## pushdown_summary

Every federated query response carries an array — one entry per `FederatedScan`. The body
tells you exactly what got pushed down to the source vs. what evaluated above the scan.

For a query like:

```sql
SELECT u.email, COUNT(*)
  FROM pg_prod.public.users u JOIN otel_logs l ON l.user_id = u.id
 WHERE l.severity_text = 'ERROR'
   AND u.region = 'eu'
 GROUP BY u.email
 ORDER BY 2 DESC
 LIMIT 5
```

You'd see:

```json
[
  {
    "source": "pg_prod",
    "table": "public.users",
    "filters_pushed":   ["region = $1"],
    "filters_residual": [],
    "projection_pushed": true,
    "limit_pushed": null,
    "sort_pushed": null,
    "agg_pushed": null,
    "agg_residual_reason": "cross-source group-by",
    "join_pushed": false,
    "scan_duration_ms": 14,
    "rows_returned": 3127,
    "bytes_received": 162834
  }
]
```

This is the trust mechanism for federation. If a federated query is slow, the summary tells you
whether kyma's planner failed to push something it should have, or whether the source itself was
the bottleneck.

- `filters_residual` non-empty for a filter you'd expect to be pushable → file a bug against the planner.
- `join_pushed: false` with a large `rows_returned` → the cross-source join is materializing a big intermediate result; consider pre-filtering.
- `agg_residual_reason: "cross-source group-by"` is expected for joins that span two sources; kyma cannot push aggregation to either side independently.

---

## Tracing (roadmap)

OTLP-based distributed tracing for kyma's own code paths is on the roadmap. The `tracing` crate
and `opentelemetry-otlp` exporter are in place behind a feature flag; spans are emitted at major
commit boundaries already. When the trace exporter ships, kyma will emit into its own ingest
path — kyma observing kyma.

In the meantime, use request IDs + structured logs for correlation (see above).

---

## Where to go next

- Connector administration: [Connectors](/connectors/).
- The agent endpoint contract: [The agent loop](/concepts/the-agent-loop).
- Multi-source query semantics: [Multi-source data](/concepts/multi-source-data).
