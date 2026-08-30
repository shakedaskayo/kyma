# pensieve Metrics Taxonomy

The shared rules every metric pensieve exports must follow. Area specs (A1–A4) define which metrics each subsystem ships; this file defines the naming, label, and lifecycle rules they all follow.

## Naming

- Every metric name starts with `pensieve_`.
- Format: `pensieve_<subsystem>_<metric>_<unit>` where:
  - `<subsystem>` is `ingest`, `query`, `catalog`, `storage`, `compaction`, `retention`, `gc`, `auth`, `mcp`, `server`, `flight`, `agent`, `data_source`, `embed`, ...
  - `<metric>` is a short snake_case noun.
  - `<unit>` is the Prometheus unit suffix when applicable: `_seconds`, `_bytes`, `_total` (counters), `_ratio`, `_count`.

Examples:

- `pensieve_ingest_rows_total` (counter)
- `pensieve_ingest_commit_seconds` (histogram)
- `pensieve_query_pruning_extents_skipped_ratio` (gauge)
- `pensieve_storage_extent_bytes` (histogram)

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

Removing or renaming a metric follows the same 6-month / 2-minor-release policy as `docs/stability.md` section 10. The deprecated metric continues to be exported, and `pensieve_deprecation_used_total{surface}` ticks once per scrape of a deprecated name.

## Verification

The back-compat CI workflow scrapes `/metrics` against each fixture and SHA-256-hashes a normalized form of the output (counter values normalized to `0`, lines sorted) before comparing against the per-fixture `expected-hashes.txt`. Any change to the set of metric names or labels — additions OR removals — produces a hash mismatch and fails the workflow. A metric addition therefore requires a deliberate fixture re-capture; a metric removal without prior deprecation announcement fails CI.

## Current state (as of F1)

The following metrics are emitted by the engine today. Metrics that DO NOT yet follow the `pensieve_` naming rule are listed under "violations" — they MUST be renamed (with deprecation aliases for back-compat) before v1.0.0 ships. Area specs (A1–A4) own the rename work for their subsystem.

### Compliant metrics

- `pensieve_catalog_cas_conflicts_total` (counter)
- `pensieve_commit_batch_extents` (histogram) — **unit suffix missing**; should be renamed `pensieve_commit_batch_extents_count` before v1.0
- `pensieve_commit_batches_total` (counter)
- `pensieve_compaction_bytes_in` (histogram)
- `pensieve_compaction_bytes_out` (histogram)
- `pensieve_compaction_duration_seconds` (histogram)
- `pensieve_compaction_tasks_submitted_total` (counter)
- `pensieve_compaction_tasks_total` (counter)
- `pensieve_data_source_cursor_age_seconds` (gauge)
- `pensieve_data_source_duration_seconds` (histogram)
- `pensieve_data_source_errors_total` (counter)
- `pensieve_data_source_last_success_timestamp_seconds` (gauge)
- `pensieve_data_source_rows_ingested_total` (counter)
- `pensieve_data_source_ticks_total` (counter)
- `pensieve_filedrop_objects_processed_total` (counter)
- `pensieve_filedrop_rows_total` (counter)
- `pensieve_flight_do_get_total` (counter)
- `pensieve_flight_serve_extent_total` (counter)
- `pensieve_http_errors_total` (counter)
- `pensieve_ingest_bytes_total` (counter)
- `pensieve_ingest_duration_seconds` (histogram)
- `pensieve_ingest_idempotency_hits_total` (counter)
- `pensieve_ingest_idempotency_races_total` (counter)
- `pensieve_ingest_rows_total` (counter)
- `pensieve_kafka_messages_ingested_total` (counter)
- `pensieve_mcp_tool_calls_total` (counter)
- `pensieve_mcp_tool_results_total` (counter)
- `pensieve_otlp_log_records_total` (counter)
- `pensieve_physical_gc_objects_delete_failed_total` (counter)
- `pensieve_physical_gc_objects_deleted_total` (counter)
- `pensieve_query_budget_exceeded_total` (counter)
- `pensieve_query_duration_seconds` (histogram)
- `pensieve_query_frontend_total` (counter)
- `pensieve_query_requests_total` (counter)
- `pensieve_query_rows_returned` (histogram) — **unit suffix missing**; should be renamed `pensieve_query_rows_returned_count` before v1.0
- `pensieve_retention_extents_soft_deleted_total` (counter)
- `pensieve_scan_blocks_pruned_total` (counter)
- `pensieve_scan_blocks_scanned_total` (counter)
- `pensieve_scan_extents_listed_total` (counter)
- `pensieve_scan_extents_remote_assigned_total` (counter)
- `pensieve_scan_extents_remote_fallback_total` (counter)
- `pensieve_scan_extents_remote_total` (counter)
- `pensieve_staging_flush_duration_seconds` (histogram)
- `pensieve_staging_flush_waiters` (histogram) — **unit suffix missing**; should be renamed `pensieve_staging_flush_waiters_count` before v1.0
- `pensieve_staging_flushes_total` (counter)

### Violations (rename required before v1.0)

None. Every metric emitted by the engine already carries the `pensieve_` prefix.

Note: Three metrics (`pensieve_commit_batch_extents`, `pensieve_query_rows_returned`, `pensieve_staging_flush_waiters`) are missing the required unit suffix and are annotated above. These are naming-convention violations within the compliant set — they need renaming with deprecation aliases but are not prefix violations.

(If a violation is unavoidable because of a third-party library, note it as such.)

## Per-subsystem inventories

Each area spec (A1–A4) appends a section under `docs/runbooks/<area>.md` listing the metrics it ships and what each one means. This document only holds the rules.
