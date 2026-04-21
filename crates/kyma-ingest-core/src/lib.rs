//! Shared ingest write path.
//!
//! Phase-A simplification: one call = one extent. No WAL, no staging
//! buffer, no backpressure queue — those land in M2 once the vertical
//! slice is proven end-to-end.
//!
//! # Flow
//!
//! ```text
//! Frontend              WritePath                    Format          Catalog
//!    |                     |                           |                |
//!    |-- ingest(batch) --->|                           |                |
//!    |                     |-- start_extent(schema) -->|                |
//!    |                     |-- append(batch) --------->|                |
//!    |                     |-- finish() ------------->put object       |
//!    |                     |<---- ExtentWriteResult ---|                |
//!    |                     |-- begin_snapshot(table) ------------------>|
//!    |                     |-- add_extent(manifest) ------------------->|
//!    |                     |-- commit() -------------------------------->|
//!    |<-- IngestAck -------|                           |                |
//! ```

#![forbid(unsafe_code)]

pub mod commit_coordinator;
pub mod ndjson;
pub mod staging;
pub use commit_coordinator::{CommitCoordinator, CoordinatorConfig};
pub use ndjson::{parse_ndjson, NdjsonError};
pub use staging::{FlushOutcome, StagingBuffer, StagingConfig};

use chrono::{DateTime, Utc};
use kyma_core::catalog::{Catalog, ExtentManifest, IngestLedgerEntry, SnapshotSummary, TableRef};
use kyma_core::errors::{CatalogError, Error, Result};
use kyma_core::segment_format::{ExtentWriteResult, SegmentFormat};
use kyma_core::types::{SnapshotId, TableId};
use std::sync::Arc;
use tracing::instrument;

/// Ack returned on successful ingest.
#[derive(Debug, Clone)]
pub struct IngestAck {
    pub snapshot_id: SnapshotId,
    pub extent_count: usize,
    pub rows_ingested: u64,
    pub bytes_written: u64,
    /// `true` if this ack was replayed from the idempotency ledger, i.e. the
    /// request key had already been processed and no new data was written.
    pub replayed: bool,
}

/// The shared write path.
///
/// Cloneable; safe to share across async tasks. When constructed with
/// [`WritePath::with_staging`], group-commit kicks in — concurrent requests
/// are batched into shared extents. With [`WritePath::new`], each request
/// flushes its own extent (kept for simple tests + backward compat).
#[derive(Clone)]
pub struct WritePath {
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    staging: Option<StagingBuffer>,
}

impl std::fmt::Debug for WritePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WritePath").finish_non_exhaustive()
    }
}

impl WritePath {
    /// Non-batched write path — every request is its own extent. Good for
    /// tests and extremely low-throughput workloads.
    pub fn new(catalog: Arc<dyn Catalog>, format: Arc<dyn SegmentFormat>) -> Self {
        Self {
            catalog,
            format,
            staging: None,
        }
    }

    /// Batched write path with a group-commit staging buffer.
    pub fn with_staging(
        catalog: Arc<dyn Catalog>,
        format: Arc<dyn SegmentFormat>,
        staging: StagingBuffer,
    ) -> Self {
        Self {
            catalog,
            format,
            staging: Some(staging),
        }
    }

    /// Ingest a batch of `RecordBatch`es into `table`.
    ///
    /// Retries on snapshot CAS conflicts up to a small bound — other writers
    /// committed in the meantime and we re-lineage onto their snapshot.
    #[instrument(skip(self, batches), fields(table = %table.name, batch_count = batches.len()))]
    pub async fn ingest(
        &self,
        table: &TableRef,
        batches: Vec<arrow_array::RecordBatch>,
    ) -> Result<IngestAck> {
        self.ingest_with_idempotency(table, batches, None).await
    }

    /// Ingest variant that checks + records an idempotency key.
    ///
    /// - If `idempotency_key` is `Some` and has already been applied, returns
    ///   the cached ack (no re-ingest).
    /// - If not seen, performs the ingest, then records the key.
    /// - Phase-A race window: if two requests with the same key arrive
    ///   concurrently, both may ingest once; the ledger INSERT-ON-CONFLICT
    ///   ensures only one record wins. Tighter atomicity lands with M2.
    #[instrument(skip(self, batches), fields(table = %table.name, batch_count = batches.len(), idempotent = idempotency_key.is_some()))]
    pub async fn ingest_with_idempotency(
        &self,
        table: &TableRef,
        batches: Vec<arrow_array::RecordBatch>,
        idempotency_key: Option<&str>,
    ) -> Result<IngestAck> {
        let start = std::time::Instant::now();
        let table_label = table.name.clone();

        // 0. If the request carried an idempotency key, short-circuit if we
        //    already processed it.
        if let Some(key) = idempotency_key {
            if let Some(cached) = self.catalog.lookup_idempotency(key).await? {
                tracing::info!(
                    idempotency_key = %key,
                    cached_snapshot_id = %cached.snapshot_id,
                    "idempotency hit: replaying cached ack"
                );
                metrics::counter!(
                    "kyma_ingest_idempotency_hits_total",
                    "table" => table_label.clone()
                )
                .increment(1);
                return Ok(IngestAck {
                    snapshot_id: cached.snapshot_id,
                    extent_count: 0,
                    rows_ingested: cached.rows_ingested,
                    bytes_written: cached.bytes_written,
                    replayed: true,
                });
            }
        }

        if batches.is_empty() {
            return Ok(IngestAck {
                snapshot_id: table.current_snapshot_id,
                extent_count: 0,
                rows_ingested: 0,
                bytes_written: 0,
                replayed: false,
            });
        }

        // Group-commit fast path: hand off to the staging buffer and wait
        // for the shared flush to complete. Every concurrent caller whose
        // batch landed in the same flush returns the same snapshot_id.
        if let Some(staging) = &self.staging {
            let outcome = staging
                .submit(table, table.schema.clone(), batches.clone())
                .await?;
            let ack = IngestAck {
                snapshot_id: outcome.snapshot_id,
                extent_count: 1,
                rows_ingested: outcome
                    .row_count
                    .min(batches.iter().map(|b| b.num_rows() as u64).sum()),
                bytes_written: outcome.byte_size,
                replayed: false,
            };
            metrics::counter!("kyma_ingest_rows_total", "table" => table_label.clone())
                .increment(ack.rows_ingested);
            metrics::histogram!("kyma_ingest_duration_seconds", "table" => table_label.clone())
                .record(start.elapsed().as_secs_f64());
            if let Some(key) = idempotency_key {
                let entry = IngestLedgerEntry {
                    table_id: table.id,
                    snapshot_id: ack.snapshot_id,
                    rows_ingested: ack.rows_ingested,
                    bytes_written: ack.bytes_written,
                    applied_at: Utc::now(),
                };
                let _ = self
                    .catalog
                    .record_idempotency(key, entry, chrono::Duration::hours(24))
                    .await?;
            }
            return Ok(ack);
        }

        // 1. Write the extent to object storage (idempotent-ish — if we fail
        //    before commit, the object becomes garbage; M3's GC reaps it).
        let extent_target_bytes = table
            .config
            .extent_target_bytes
            .unwrap_or(1024 * 1024 * 1024);
        let mut writer = self
            .format
            .start_extent(table.schema.clone(), extent_target_bytes)
            .await?;
        let mut rows_ingested: u64 = 0;
        for batch in batches {
            rows_ingested += batch.num_rows() as u64;
            writer.append(batch).await?;
        }
        let result: ExtentWriteResult = writer.finish().await?;

        // 2. Commit the new extent into a fresh snapshot. On CAS conflict,
        //    re-read the current snapshot and retry — bounded attempts.
        let snapshot_id = self
            .commit_with_retry(
                table.id,
                &table_result_to_manifest(table, &result),
                &table_label,
            )
            .await?;

        metrics::counter!("kyma_ingest_rows_total", "table" => table_label.clone())
            .increment(rows_ingested);
        metrics::counter!("kyma_ingest_bytes_total", "table" => table_label.clone())
            .increment(result.byte_size);
        metrics::histogram!("kyma_ingest_duration_seconds", "table" => table_label.clone())
            .record(start.elapsed().as_secs_f64());

        // Record the idempotency key after successful commit.
        if let Some(key) = idempotency_key {
            let entry = IngestLedgerEntry {
                table_id: table.id,
                snapshot_id,
                rows_ingested,
                bytes_written: result.byte_size,
                applied_at: Utc::now(),
            };
            let recorded = self
                .catalog
                .record_idempotency(key, entry, chrono::Duration::hours(24))
                .await?;
            if recorded.is_none() {
                tracing::warn!(
                    idempotency_key = %key,
                    "idempotency race: a concurrent request won the ledger insert; both ingests committed extents (accept minor duplicate)"
                );
                metrics::counter!(
                    "kyma_ingest_idempotency_races_total",
                    "table" => table_label
                )
                .increment(1);
            }
        }

        Ok(IngestAck {
            snapshot_id,
            extent_count: 1,
            rows_ingested,
            bytes_written: result.byte_size,
            replayed: false,
        })
    }

    #[instrument(skip(self, manifest, table_label))]
    async fn commit_with_retry(
        &self,
        table_id: TableId,
        manifest: &ExtentManifest,
        table_label: &str,
    ) -> Result<SnapshotId> {
        // Under high write contention (many writers, one snapshot pointer per
        // table), retries can stack. Use exponential backoff + jitter and
        // enough attempts to succeed under realistic loads. If a table is
        // truly hot, partitioning the ingest path across tables is the fix;
        // a single-snapshot-pointer table is inherently serial at the commit.
        const MAX_ATTEMPTS: u32 = 20;
        for attempt in 1..=MAX_ATTEMPTS {
            let mut txn = self.catalog.begin_snapshot(table_id).await?;
            txn.add_extent(manifest.clone()).await?;
            match txn
                .commit(SnapshotSummary {
                    rows_added: manifest.row_count as i64,
                    rows_removed: 0,
                    bytes_added: manifest.byte_size as i64,
                    bytes_removed: 0,
                    operation: "ingest".to_string(),
                })
                .await
            {
                Ok(id) => return Ok(id),
                Err(Error::Catalog(CatalogError::Conflict)) => {
                    metrics::counter!(
                        "kyma_catalog_cas_conflicts_total",
                        "table" => table_label.to_owned()
                    )
                    .increment(1);
                    // Exponential backoff with jitter, capped at ~200ms.
                    let base_ms = (2u64.saturating_pow(attempt.min(8))).min(200);
                    let jitter_ms = fastrand::u64(..base_ms.max(1));
                    tracing::warn!(
                        attempt,
                        backoff_ms = base_ms + jitter_ms,
                        "snapshot CAS conflict on ingest; retrying with latest parent"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::Catalog(CatalogError::Conflict))
    }
}

fn table_result_to_manifest(table: &TableRef, result: &ExtentWriteResult) -> ExtentManifest {
    ExtentManifest {
        id: result.extent_id,
        table_id: table.id,
        schema_snapshot_id: table.schema_snapshot_id,
        object_path: result.object_path.clone(),
        byte_size: result.byte_size,
        row_count: result.row_count,
        min_timestamp: result.min_timestamp_nanos.map(|n| nanos_to_utc(n)),
        max_timestamp: result.max_timestamp_nanos.map(|n| nanos_to_utc(n)),
        // Per-column distinct-value sets from the writer; used by the
        // catalog for equality-pushdown pruning.
        column_stats: result.column_stats.clone(),
        present_paths: result.present_paths.clone(),
        compaction_gen: 0,
        created_at: Utc::now(),
    }
}

fn nanos_to_utc(n: i64) -> DateTime<Utc> {
    let secs = n.div_euclid(1_000_000_000);
    let nsec = n.rem_euclid(1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(secs, nsec).unwrap_or_else(Utc::now)
}
