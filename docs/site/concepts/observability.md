---
title: Observability
description: How to tell what pensieve is doing. Prometheus metrics, request IDs, structured logs, agent run replay, dreaming run history, data source status, and the pushdown_summary that prevents federation from silently degrading.
---

# Observability

pensieve exposes four distinct observability surfaces, each aimed at a different audience:

| Surface | Endpoint | Who uses it |
| ------- | -------- | ----------- |
| Prometheus metrics | `GET /metrics` | Operators, alerting systems |
| Agent run replay | `GET /v1/agent/runs/:run_id` | Engineers debugging a wrong answer |
| Dreaming run history | `GET /v1/agent/memory/dreaming/runs` | Engineers reviewing background memory synthesis |
| Data source status | `GET /v1/data-sources/:id` | Operators of multi-source deployments |
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
| `pensieve_query_duration_seconds` | `language` (kql/sql/promql) | Wall time per query, by language. Rising p99 under steady load means storage or planning pressure. |
| `pensieve_query_requests_total` | `language`, `status` | Query volume; `status=error` rate is your query-error SLI. |
| `pensieve_query_rows_returned` | — | Histogram of result sizes; outlier queries scan enormous result sets. |
| `pensieve_query_budget_exceeded_total` | — | Queries killed by the row/time budget. Spikes indicate runaway scans or missing indexes. |
| `pensieve_scan_blocks_scanned_total` | — | Raw blocks touched per query. High value with low `_pruned_total` means poor time-range filtering. |
| `pensieve_scan_blocks_pruned_total` | — | Blocks skipped by bloom / time pruning. Ratio `pruned / (pruned + scanned)` is the pruning efficiency; low values suggest stale statistics or poor partitioning. |
| `pensieve_scan_extents_listed_total` | — | Catalog lookups per query; scales with table fragmentation. |

**Example PromQL — pruning efficiency:**
```promql
rate(pensieve_scan_blocks_pruned_total[5m])
  /
(rate(pensieve_scan_blocks_pruned_total[5m]) + rate(pensieve_scan_blocks_scanned_total[5m]))
```
A value below 0.8 in production warrants investigation.

### Ingest

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `pensieve_ingest_rows_total` | `frontend` (rest/otlp/kafka/filedrop), `table` | Rows landed per ingest path. Drop to zero = a writer stopped. |
| `pensieve_ingest_bytes_total` | `frontend`, `table` | Bytes written. Combined with rows gives average row size. |
| `pensieve_ingest_duration_seconds` | `frontend` | End-to-end ingest latency including staging + commit. |
| `pensieve_ingest_idempotency_hits_total` | — | Exact-duplicate rows deduplicated at commit. High rate = client retrying without ETag. |
| `pensieve_ingest_idempotency_races_total` | — | Two concurrent writers committed the same idempotency key. High rate = ingest fan-out without coordination. |
| `pensieve_kafka_messages_ingested_total` | `topic`, `table` | Kafka consumer progress. |
| `pensieve_otlp_log_records_total` | — | OTLP log records received. |
| `pensieve_filedrop_objects_processed_total` | — | Files processed by the file-drop frontend. |
| `pensieve_filedrop_rows_total` | — | Rows parsed from file-drop objects. |

### Storage upkeep

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `pensieve_staging_flush_duration_seconds` | — | Time the staging buffer spends in group-commit. P99 above ~200 ms indicates write pressure. |
| `pensieve_staging_flushes_total` | — | Number of staging flushes committed. |
| `pensieve_staging_flush_waiters` | — | Goroutines waiting on a flush. Sustained > 0 means group-commit is saturated. |
| `pensieve_catalog_cas_conflicts_total` | — | Snapshot compare-and-swap retries during commit. High value = ingest contention; two writers racing on the same table. |
| `pensieve_commit_batches_total` | — | Commit batches attempted. |
| `pensieve_commit_batch_extents` | — | Extents per commit batch; reflects merge efficiency. |
| `pensieve_compaction_tasks_total` | — | Compaction tasks completed. |
| `pensieve_compaction_tasks_submitted_total` | — | Compaction tasks queued. If `submitted` >> `completed` for sustained periods, the compactor is falling behind. |
| `pensieve_compaction_bytes_in` | — | Bytes read by the compactor. |
| `pensieve_compaction_bytes_out` | — | Bytes written by the compactor. Ratio `out/in` close to 1 = no meaningful compression gain. |
| `pensieve_compaction_duration_seconds` | — | Time per compaction task. |
| `pensieve_retention_extents_soft_deleted_total` | — | Extents soft-deleted by the retention sweeper. Not advancing = retention not running or policy not matched. |
| `pensieve_physical_gc_objects_deleted_total` | — | Objects physically removed from object storage. |
| `pensieve_physical_gc_objects_delete_failed_total` | — | GC failures. Persistent non-zero = permission or network issue against object storage. |

### Data sources

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `pensieve_data_source_cursor_age_seconds` | `name`, `table` | Age of the sync cursor — how far the data source is behind the source. Rising = a sync source falling behind. Alert if > your RPO. |
| `pensieve_data_source_rows_ingested_total` | `name`, `table` | Rows synced from each source-table. |
| `pensieve_data_source_ticks_total` | `name` | Data source polling or CDC event cycles. Flat for a sync data source = it has stopped. |
| `pensieve_data_source_errors_total` | `name` | Hard errors per data source. Any sustained non-zero rate warrants investigation. |
| `pensieve_data_source_duration_seconds` | `name` | Time spent per data source tick. |
| `pensieve_data_source_last_success_timestamp_seconds` | `name` | Unix timestamp of last successful tick. Alert if `time() - pensieve_data_source_last_success_timestamp_seconds > threshold`. |

**Example PromQL — data sources that haven't succeeded in 5 minutes:**
```promql
time() - pensieve_data_source_last_success_timestamp_seconds > 300
```

### Agent and MCP

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `pensieve_mcp_tool_calls_total` | `tool` | MCP tool calls dispatched by the agent. High rate of a single tool = agent looping. |
| `pensieve_mcp_tool_results_total` | `tool`, `status` | MCP tool results returned. `status=error` rate per tool surfaces bad skill definitions. |
| `pensieve_explore_search_requests_total` | — | `/explore/search` requests. |
| `pensieve_explore_search_executed_total` | — | Searches that reached the query engine. |
| `pensieve_explore_search_duration_seconds` | — | Search latency. |
| `pensieve_explore_search_rows_returned` | — | Result rows per search. |
| `pensieve_explore_search_cap_hits_total` | — | Searches that hit the result-row cap; indicates the cap may need raising for this deployment. |
| `pensieve_explore_search_per_source_errors_total` | — | Per-source errors during multi-source search. |
| `pensieve_explore_search_sources_resolved` | — | Number of sources each search fanned out to. |

### HTTP layer

| Metric | Labels | What it measures |
| ------ | ------ | ---------------- |
| `pensieve_http_errors_total` | `method`, `path`, `status` | HTTP 4xx/5xx responses. |
| `pensieve_flight_do_get_total` | — | Arrow Flight DoGet requests (used by distributed scan). |
| `pensieve_flight_serve_extent_total` | — | Flight extents served. |

---

## Request ID correlation

Every pensieve HTTP response carries an `x-request-id` header — either echoed from the request if
the client supplied one, or generated as a fresh ULID. Structured log lines emitted during that
request carry the same ID. This lets you correlate a slow or failed API call to the server-side
log trace without any additional instrumentation.

```bash
# Capture the request ID from a query response
RID=$(curl -si http://localhost:8080/v1/query -d '{"sql":"SELECT 1"}' | grep -i x-request-id | awk '{print $2}' | tr -d '\r')

# Find all log lines for that request
journalctl -u pensieve --no-pager | grep "$RID"
```

---

## Structured logs

pensieve uses the `tracing` crate with structured fields. Set `RUST_LOG` to control verbosity:

```bash
# Production — warn + error only
RUST_LOG=warn pensieve serve

# Debug a specific component
RUST_LOG=pensieve_datasources=debug,pensieve_ingest_core=info,warn pensieve serve
```

Log lines include `request_id`, `table`, `data_source_id`, and other relevant fields so they can be
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

## Data source status

List data sources, or fetch one by id for its last-run state:

```bash
curl http://localhost:8080/v1/data-sources           # all data sources
curl http://localhost:8080/v1/data-sources/<id>       # one, with last-run fields
```

The per-data-source response carries the run state (secrets scrubbed):

```json
{
  "id": "01H...",
  "name": "prod-postgres",
  "type": "postgres",
  "target_database": "pg_prod",
  "target_table": null,
  "schedule_ms": 60000,
  "enabled": true,
  "disabled_reason": null,
  "last_run_at": "2026-05-03T14:21:56Z",
  "last_success_at": "2026-05-03T14:21:56Z",
  "last_error": null,
  "last_rows_ingested": 5240
}
```

| Field | Watch for |
| ----- | --------- |
| `last_success_at` | Stale relative to `schedule_ms` — the data source has stopped making progress. |
| `last_error` | Non-null on a data source that should be healthy. |
| `enabled` / `disabled_reason` | A data source auto-disabled after repeated failures. |
| `last_rows_ingested` | Persistently `0` for a source you expect to change. |

For time-series health (alerting, dashboards), use the Prometheus data source
metrics above — `pensieve_data_source_cursor_age_seconds` (sync lag),
`pensieve_data_source_last_success_timestamp_seconds`, and `pensieve_data_source_errors_total`
are the operational signals. Full endpoint list in the
[API reference](/reference/api).

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
whether pensieve's planner failed to push something it should have, or whether the source itself was
the bottleneck.

- `filters_residual` non-empty for a filter you'd expect to be pushable → file a bug against the planner.
- `join_pushed: false` with a large `rows_returned` → the cross-source join is materializing a big intermediate result; consider pre-filtering.
- `agg_residual_reason: "cross-source group-by"` is expected for joins that span two sources; pensieve cannot push aggregation to either side independently.

---

## Tracing (roadmap)

OTLP-based distributed tracing for pensieve's own code paths is on the roadmap. The `tracing` crate
and `opentelemetry-otlp` exporter are in place behind a feature flag; spans are emitted at major
commit boundaries already. When the trace exporter ships, pensieve will emit into its own ingest
path — pensieve observing pensieve.

In the meantime, use request IDs + structured logs for correlation (see above).

---

## Where to go next

- Data source administration: [Data sources](/data-sources/).
- The agent endpoint contract: [The agent loop](/concepts/the-agent-loop).
- Multi-source query semantics: [Multi-source data](/concepts/multi-source-data).
