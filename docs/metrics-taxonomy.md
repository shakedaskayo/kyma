# kyma Metrics Taxonomy

The shared rules every metric kyma exports must follow. Area specs (A1–A4) define which metrics each subsystem ships; this file defines the naming, label, and lifecycle rules they all follow.

## Naming

- Every metric name starts with `kyma_`.
- Format: `kyma_<subsystem>_<metric>_<unit>` where:
  - `<subsystem>` is `ingest`, `query`, `catalog`, `storage`, `compaction`, `retention`, `gc`, `auth`, `mcp`, `server`, `flight`, `agent`, `connector`, `embed`, ...
  - `<metric>` is a short snake_case noun.
  - `<unit>` is the Prometheus unit suffix when applicable: `_seconds`, `_bytes`, `_total` (counters), `_ratio`, `_count`.

Examples:

- `kyma_ingest_rows_total` (counter)
- `kyma_ingest_commit_seconds` (histogram)
- `kyma_query_pruning_extents_skipped_ratio` (gauge)
- `kyma_storage_extent_bytes` (histogram)

## Labels

- High-cardinality labels (anything per-query, per-trace-id, per-tenant-token) are forbidden.
- Allowed labels:
  - `database` — bounded by user database count.
  - `table` — bounded by table count per database.
  - `tenant_id` — bounded by tenant count.
  - `subsystem` — bounded.
  - `code` — bounded enum of error codes.
  - `result` — `success` / `error` / `cancelled` / `timeout`.
- `tenant_id` is allowed only for metrics where tenant-level breakdown is a documented operator need; otherwise omit.

## Histograms

- Latency histograms use the seconds unit (`_seconds`).
- Bucket boundaries are documented per metric in the area spec that owns it.

## Deprecation

Removing or renaming a metric follows the same 6-month / 2-minor-release policy as `docs/stability.md` section 10. The deprecated metric continues to be exported, and `kyma_deprecation_used_total{surface}` ticks once per scrape of a deprecated name.

## Verification

The back-compat CI workflow scrapes `/metrics` against each fixture and SHA-256-hashes a normalized form of the output (counter values normalized to `0`, lines sorted) before comparing against the per-fixture `expected-hashes.txt`. Any change to the set of metric names or labels — additions OR removals — produces a hash mismatch and fails the workflow. A metric addition therefore requires a deliberate fixture re-capture; a metric removal without prior deprecation announcement fails CI.

## Current state (as of F1)

The following metrics are emitted by the engine today. Metrics that DO NOT yet follow the `kyma_` naming rule are listed under "violations" — they MUST be renamed (with deprecation aliases for back-compat) before v1.0.0 ships. Area specs (A1–A4) own the rename work for their subsystem.

### Compliant metrics

- `kyma_catalog_cas_conflicts_total` (counter)
- `kyma_commit_batch_extents` (histogram) — **unit suffix missing**; should be renamed `kyma_commit_batch_extents_count` before v1.0
- `kyma_commit_batches_total` (counter)
- `kyma_compaction_bytes_in` (histogram)
- `kyma_compaction_bytes_out` (histogram)
- `kyma_compaction_duration_seconds` (histogram)
- `kyma_compaction_tasks_submitted_total` (counter)
- `kyma_compaction_tasks_total` (counter)
- `kyma_connector_cursor_age_seconds` (gauge)
- `kyma_connector_duration_seconds` (histogram)
- `kyma_connector_errors_total` (counter)
- `kyma_connector_last_success_timestamp_seconds` (gauge)
- `kyma_connector_rows_ingested_total` (counter)
- `kyma_connector_ticks_total` (counter)
- `kyma_filedrop_objects_processed_total` (counter)
- `kyma_filedrop_rows_total` (counter)
- `kyma_flight_do_get_total` (counter)
- `kyma_flight_serve_extent_total` (counter)
- `kyma_http_errors_total` (counter)
- `kyma_ingest_bytes_total` (counter)
- `kyma_ingest_duration_seconds` (histogram)
- `kyma_ingest_idempotency_hits_total` (counter)
- `kyma_ingest_idempotency_races_total` (counter)
- `kyma_ingest_rows_total` (counter)
- `kyma_kafka_messages_ingested_total` (counter)
- `kyma_mcp_tool_calls_total` (counter)
- `kyma_mcp_tool_results_total` (counter)
- `kyma_otlp_log_records_total` (counter)
- `kyma_physical_gc_objects_delete_failed_total` (counter)
- `kyma_physical_gc_objects_deleted_total` (counter)
- `kyma_query_budget_exceeded_total` (counter)
- `kyma_query_duration_seconds` (histogram)
- `kyma_query_frontend_total` (counter)
- `kyma_query_requests_total` (counter)
- `kyma_query_rows_returned` (histogram) — **unit suffix missing**; should be renamed `kyma_query_rows_returned_count` before v1.0
- `kyma_retention_extents_soft_deleted_total` (counter)
- `kyma_scan_blocks_pruned_total` (counter)
- `kyma_scan_blocks_scanned_total` (counter)
- `kyma_scan_extents_listed_total` (counter)
- `kyma_scan_extents_remote_assigned_total` (counter)
- `kyma_scan_extents_remote_fallback_total` (counter)
- `kyma_scan_extents_remote_total` (counter)
- `kyma_staging_flush_duration_seconds` (histogram)
- `kyma_staging_flush_waiters` (histogram) — **unit suffix missing**; should be renamed `kyma_staging_flush_waiters_count` before v1.0
- `kyma_staging_flushes_total` (counter)

### Violations (rename required before v1.0)

None. Every metric emitted by the engine already carries the `kyma_` prefix.

Note: Three metrics (`kyma_commit_batch_extents`, `kyma_query_rows_returned`, `kyma_staging_flush_waiters`) are missing the required unit suffix and are annotated above. These are naming-convention violations within the compliant set — they need renaming with deprecation aliases but are not prefix violations.

(If a violation is unavoidable because of a third-party library, note it as such.)

## Per-subsystem inventories

Each area spec (A1–A4) appends a section under `docs/runbooks/<area>.md` listing the metrics it ships and what each one means. This document only holds the rules.
