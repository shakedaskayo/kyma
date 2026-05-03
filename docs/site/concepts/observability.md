---
title: Observability
description: How to tell what kyma is doing. Prometheus metrics, the agent run trace, the connector status surface, and the `pushdown_summary` that prevents federation from silently degrading.
---

# Observability

kyma observes a lot of things; you should be able to observe kyma. Four
surfaces, by audience:

- **`/metrics`** — Prometheus exposition. Operators.
- **`/v1/agent/runs/:run_id`** — full trace replay of an agent run.
  Engineers debugging a wrong answer.
- **`/v1/connectors/:id/status`** — structured health doc per
  connector. Operators of multi-source deployments.
- **`pushdown_summary`** — every federated query response carries one.
  Anyone whose query was unexpectedly slow.

## Prometheus metrics

Standard Prometheus exposition at `GET /metrics`. Always public, no
auth required. Unauthenticated metrics are deliberate — keep this
endpoint network-isolated, exactly like every other Prometheus target.

Counter / histogram families that matter most:

| Metric                                            | Purpose                                          |
| ------------------------------------------------- | ------------------------------------------------ |
| `kyma_query_duration_seconds{language}`           | Wall time per query, by KQL/SQL/PromQL.          |
| `kyma_query_extents_pruned_ratio`                 | Histogram. The pruning cascade's headline.       |
| `kyma_ingest_rows_total{frontend, table}`         | Rows landed via REST/OTLP/Kafka/file-drop.       |
| `kyma_ingest_bytes_total{frontend, table}`        | Bytes written.                                   |
| `kyma_staging_buffer_flush_duration_seconds`      | How long group-commit waits.                     |
| `kyma_commit_cas_conflicts_total`                 | Snapshot CAS retries. High = ingest contention.  |
| `kyma_compaction_extents_merged_total{table}`     | Compaction throughput.                           |
| `kyma_retention_bytes_freed_total{table}`         | Retention sweeper progress.                      |
| `kyma_connector_lag_seconds{name, table}`         | Sync-mode CDC lag per source-table.              |
| `kyma_connector_pool_in_use{name}`                | Federation pool checkout count.                  |
| `kyma_agent_runs_total{status}`                   | Agent endpoint outcomes.                         |

Every metric is documented inline at the source — `grep -r '#\[metric'`
if you need an authoritative list.

## Agent run replay

Each `/v1/agent/ask` invocation persists to the `agent_runs` catalog
table. A run carries the question, the model, the full event log, and
the resulting tokens / wall time / status.

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
    { "kind": "tool_call", "tool": "run_sql", "arguments": {...} },
    { "kind": "tool_result", "rows": [...] },
    { "kind": "answer_delta", "text": "..." },
    { "kind": "answer_final", "text": "..." }
  ],
  "usage": { "tokens_in": 940, "tokens_out": 320, "tools_called": 2 }
}
```

The use case is "the agent gave a weird answer; what did it actually
do?" Open the run; the event log is everything.

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

The same data is exposed as a **kyma table** — `kyma_connector_health`
— so you can KQL/SQL it the same way you query everything else, and
chart it on dashboards alongside the rest of your observability.

```kql
kyma_connector_health
| where mode != "federation"
| where lag_seconds > 30
| project name, table, lag_seconds, last_event_at, last_error
```

## pushdown_summary

Every federated query response carries an array — one entry per
`FederatedScan`. The body of the array tells you exactly what got
pushed down to the source vs. what evaluated above the scan.

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

This is the trust mechanism for federation. If a federated query is
slow, the summary tells you whether kyma's planner failed to push
something it should have, or whether the source itself was the
bottleneck. If you see `filters_residual` populated for a filter you'd
expect to be pushable, file a bug against the planner.

## Tracing

OTLP-based tracing for kyma's own code paths is on the roadmap. The
relevant trait surface is in place; spans get emitted at major commit
boundaries already (`tracing` crate, `opentelemetry-otlp` exporter
behind a feature flag). When the trace exporter ships, kyma emits
into its own ingest path — kyma observing kyma.

## Where to go next

- Connector administration: [Connectors](/connectors/).
- The agent endpoint contract: [The agent loop](/concepts/the-agent-loop).
- Multi-source query semantics: [Multi-source data](/concepts/multi-source-data).
