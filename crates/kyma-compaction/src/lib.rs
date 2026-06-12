//! Background compaction: merges small extents into larger ones.
//!
//! # Architecture
//!
//! Two long-running tokio tasks cooperate via the catalog work-queue:
//!
//! - [`CompactionScheduler`] periodically finds tables with too many small
//!   extents and submits `compaction` tasks into the queue.
//! - [`CompactionWorker`] claims `compaction` tasks, reads the specified
//!   source extents, writes a single merged extent, then commits an
//!   atomic add-new / remove-old snapshot via the catalog.
//!
//! Many workers can run in parallel against one catalog — claims are
//! serialized by `SELECT ... FOR UPDATE SKIP LOCKED`, so two workers never
//! pick the same task.

#![forbid(unsafe_code)]

pub mod artifact_retention;
pub mod retention;

pub use artifact_retention::{ArtifactRetentionWorker, ArtifactSweepStats};
pub use retention::{PhysicalDeleteWorker, RetentionSweeper};

use arrow_array::{new_null_array, ArrayRef, RecordBatch};
use chrono::Duration as ChronoDuration;
use kyma_core::catalog::{
    BackgroundTask, Catalog, ExtentManifest, PrunePredicate, SnapshotSummary, TableRef,
};
use kyma_core::errors::{CatalogError, Error, Result};
use kyma_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use kyma_core::types::{NodeId, TableId};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, instrument, warn};

/// Per-task payload (jsonb in the `background_tasks.payload` column).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionPayload {
    pub source_extent_ids: Vec<uuid::Uuid>,
}

/// Promote a batch read from an extent to `target` by null-filling any columns
/// added after that extent was written (schema evolution). Columns present in
/// the source are carried over by name; the rest become typed null arrays.
/// Mirrors the query scan-path promotion so compaction can merge narrow + wide
/// extents instead of erroring with "project index out of bounds".
fn promote_batch(batch: &RecordBatch, target: &arrow_schema::SchemaRef) -> Result<RecordBatch> {
    let src = batch.schema();
    if src.fields() == target.fields() {
        return Ok(batch.clone());
    }
    let n = batch.num_rows();
    let mut cols: Vec<ArrayRef> = Vec::with_capacity(target.fields().len());
    for f in target.fields() {
        match src.index_of(f.name()) {
            Ok(i) => cols.push(batch.column(i).clone()),
            Err(_) => cols.push(new_null_array(f.data_type(), n)),
        }
    }
    RecordBatch::try_new(target.clone(), cols)
        .map_err(|e| Error::Execution(format!("compaction schema promote: {e}")))
}

// ---------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------

/// A long-running compaction worker. Call [`CompactionWorker::run`] once;
/// the future runs until `shutdown` resolves.
#[derive(Clone)]
pub struct CompactionWorker {
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    node_id: NodeId,
    /// How long one compaction task may hold its claim before expiry.
    pub lease: Duration,
    /// Sleep between empty polls.
    pub idle_sleep: Duration,
}

impl CompactionWorker {
    pub fn new(catalog: Arc<dyn Catalog>, format: Arc<dyn SegmentFormat>, node_id: NodeId) -> Self {
        Self {
            catalog,
            format,
            node_id,
            lease: Duration::from_secs(120),
            idle_sleep: Duration::from_secs(2),
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(node_id = %self.node_id, "compaction worker starting");
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("compaction worker shutting down");
                    return;
                }
                res = self.claim_one() => {
                    match res {
                        Ok(Some(task)) => {
                            let task_id = task.id;
                            match self.run_one(task).await {
                                Ok(()) => {
                                    if let Err(e) = self.catalog.complete_task(task_id).await {
                                        error!(error = %e, task_id = %task_id, "failed to mark task done");
                                    }
                                    ::metrics::counter!("kyma_compaction_tasks_total", "result" => "ok")
                                        .increment(1);
                                }
                                Err(e) => {
                                    warn!(error = %e, task_id = %task_id, "compaction task failed; will retry");
                                    if let Err(err) = self.catalog.fail_task(task_id, &format!("{e}")).await {
                                        error!(error = %err, "failed to record task failure");
                                    }
                                    ::metrics::counter!("kyma_compaction_tasks_total", "result" => "error")
                                        .increment(1);
                                }
                            }
                        }
                        Ok(None) => {
                            tokio::time::sleep(self.idle_sleep).await;
                        }
                        Err(e) => {
                            error!(error = %e, "compaction worker claim error");
                            tokio::time::sleep(self.idle_sleep).await;
                        }
                    }
                }
            }
        }
    }

    async fn claim_one(&self) -> Result<Option<BackgroundTask>> {
        self.catalog
            .claim_task(
                "compaction",
                self.node_id,
                ChronoDuration::from_std(self.lease).unwrap_or(ChronoDuration::seconds(60)),
            )
            .await
    }

    #[instrument(skip(self, task), fields(task_id = %task.id))]
    async fn run_one(&self, task: BackgroundTask) -> Result<()> {
        let start = std::time::Instant::now();
        let payload: CompactionPayload = serde_json::from_value(task.payload.clone())
            .map_err(|e| Error::Internal(format!("bad compaction payload: {e}")))?;

        let Some(table_id) = task.table_id else {
            return Err(Error::Internal("compaction task missing table_id".into()));
        };

        // 1. Look up the table's current schema.
        let all_tables = self.list_tables_for(table_id).await?;
        let table_ref: TableRef = all_tables.into_iter().next().ok_or_else(|| {
            Error::Internal(format!("compaction: table {table_id} no longer exists"))
        })?;

        // 2. Resolve manifests for the source extents (they must still be live).
        let all_extents = self
            .catalog
            .list_extents(
                table_ref.id,
                table_ref.current_snapshot_id,
                &PrunePredicate::default(),
            )
            .await?;
        let source_manifests: Vec<ExtentManifest> = all_extents
            .into_iter()
            .filter(|m| payload.source_extent_ids.contains(m.id.as_uuid()))
            .collect();

        if source_manifests.is_empty() {
            info!(task_id = %task.id, "compaction no-op: source extents no longer live");
            return Ok(());
        }
        if source_manifests.len() == 1 {
            info!(task_id = %task.id, "compaction no-op: only one source extent survived");
            return Ok(());
        }

        debug!(
            source_count = source_manifests.len(),
            table = %table_ref.name,
            "merging extents"
        );

        // 3. Write a new merged extent.
        let mut writer = self
            .format
            .start_extent(table_ref.schema.clone(), 0)
            .await?;
        let mut merged_rows: u64 = 0;
        let mut input_bytes: u64 = 0;
        for m in &source_manifests {
            input_bytes += m.byte_size;
            let reader = self
                .format
                .open_extent(OpenExtentInput {
                    extent_id: m.id,
                    table_id: m.table_id,
                    schema: table_ref.schema.clone(),
                    object_path: m.object_path.clone(),
                    byte_size: m.byte_size,
                })
                .await?;
            let blocks = reader.pruned_blocks(&BlockPredicate::All).await?;
            for bid in blocks {
                // Read the extent's ACTUAL columns (empty projection = all), then
                // promote (null-fill) to the current table schema. Extents written
                // before a column was added are narrower; projecting current-schema
                // indices onto them is out of bounds. Mirrors the query scan-path
                // promotion so compaction can merge narrow + wide (schema-evolved)
                // extents instead of failing on them.
                let raw: RecordBatch = reader.read_block(bid, &[]).await?;
                let batch = promote_batch(&raw, &table_ref.schema)?;
                merged_rows += batch.num_rows() as u64;
                writer.append(batch).await?;
            }
        }
        let result = writer.finish().await?;

        // 4. Commit: add new extent, remove sources. Retry on CAS conflict.
        let new_manifest = new_manifest_from_write(&table_ref, &result, task.attempt);
        let source_ids: Vec<kyma_core::types::ExtentId> =
            source_manifests.iter().map(|m| m.id).collect();
        self.commit_with_retry(
            table_ref.id,
            new_manifest,
            source_ids.clone(),
            merged_rows,
            input_bytes,
        )
        .await?;

        // 5. Index-sidecar follow-up (best-effort; never fails the merge):
        //    enqueue an `index_build` job for the new extent for every
        //    (column, kind) that had sidecars on the inputs. The replaced
        //    extents keep their sidecar rows — sources are only soft-deleted
        //    here, in-grace readers may still use them, and the physical-GC
        //    worker deletes the sidecar objects (rows cascade) when the
        //    extent rows are hard-deleted after the grace period.
        if let Err(e) = self
            .enqueue_sidecar_rebuilds(&table_ref, &source_ids, result.extent_id)
            .await
        {
            warn!(error = %e, table = %table_ref.name, "sidecar rebuild enqueue failed");
        }

        let elapsed = start.elapsed().as_secs_f64();
        let bytes_out = result.byte_size;
        info!(
            task_id = %task.id,
            inputs = source_manifests.len(),
            rows = merged_rows,
            bytes_in = input_bytes,
            bytes_out,
            compression_ratio = ratio(input_bytes, bytes_out),
            seconds = elapsed,
            "compaction committed"
        );
        ::metrics::histogram!("kyma_compaction_duration_seconds").record(elapsed);
        ::metrics::histogram!("kyma_compaction_bytes_in").record(input_bytes as f64);
        ::metrics::histogram!("kyma_compaction_bytes_out").record(bytes_out as f64);
        Ok(())
    }

    /// For every distinct `(column, kind, model)` sidecar present on the
    /// replaced extents, enqueue a fabric `index_build` job covering the new
    /// merged extent so its sidecars get rebuilt.
    ///
    /// Fabric enqueue goes through `PgFabricStore`, reached by downcasting
    /// the catalog (the same phase-C shortcut `retention.rs` uses). On
    /// non-Postgres catalogs (local SQLite mode) this is a debug-logged
    /// no-op — local mode runs no index-build workers yet.
    async fn enqueue_sidecar_rebuilds(
        &self,
        table: &TableRef,
        replaced: &[kyma_core::types::ExtentId],
        new_extent: kyma_core::types::ExtentId,
    ) -> Result<()> {
        let sidecars = self
            .catalog
            .list_index_sidecars(kyma_core::DEFAULT_TENANT, table.id, replaced, None)
            .await?;
        if sidecars.is_empty() {
            return Ok(());
        }

        let any_ref: &dyn std::any::Any = self.catalog.as_ref_any();
        let Some(pg) = any_ref.downcast_ref::<kyma_catalog::PostgresCatalog>() else {
            debug!("non-Postgres catalog: skipping sidecar rebuild enqueue");
            return Ok(());
        };
        let fabric = kyma_catalog::PgFabricStore::new(pg.pool().clone());

        // Distinct (column, kind, model) groups; carry one group's params.
        let mut groups: std::collections::BTreeMap<(String, String, Option<String>), serde_json::Value> =
            std::collections::BTreeMap::new();
        for s in sidecars {
            groups
                .entry((s.column.clone(), s.kind.as_str().to_string(), s.embedding_model_id.clone()))
                .or_insert(s.params);
        }
        for ((column, kind, _model), params) in groups {
            let payload = serde_json::json!({
                "table_id": table.id.as_uuid(),
                "extent_ids": [new_extent.as_uuid()],
                "column": column,
                "kind": kind,
                "params": params,
            });
            let job = kyma_core::fabric::EnqueueJob {
                kind: kyma_core::fabric::JOB_INDEX_BUILD.to_string(),
                payload,
                priority: 0,
                affinity_worker_id: None,
                req_capabilities: Vec::new(),
                label_selector: serde_json::json!({}),
                max_attempts: 3,
            };
            let id = fabric
                .enqueue_job(kyma_core::DEFAULT_TENANT, &job)
                .await
                .map_err(|e| Error::Internal(format!("enqueue index_build: {e}")))?;
            debug!(job_id = ?id, extent = %new_extent, "enqueued sidecar rebuild");
        }
        Ok(())
    }

    async fn list_tables_for(&self, table_id: TableId) -> Result<Vec<TableRef>> {
        // Brute-force: list all databases' tables and filter. Goes through the
        // catalog trait (works on any backend) — the trait doesn't yet expose a
        // direct `lookup_table_by_id`, and adding one touches every impl.
        let dbs = self.catalog.list_databases().await?;
        for name in dbs {
            let ts = self.catalog.list_tables_in_database(&name).await?;
            for t in ts {
                if t.id == table_id {
                    return Ok(vec![t]);
                }
            }
        }
        Ok(vec![])
    }

    async fn commit_with_retry(
        &self,
        table_id: TableId,
        new_manifest: ExtentManifest,
        sources: Vec<kyma_core::types::ExtentId>,
        rows_merged: u64,
        bytes_in: u64,
    ) -> Result<()> {
        const MAX: u32 = 10;
        for attempt in 1..=MAX {
            let mut txn = self.catalog.begin_snapshot(table_id).await?;
            txn.add_extent(new_manifest.clone()).await?;
            txn.remove_extents(&sources).await?;
            match txn
                .commit(SnapshotSummary {
                    rows_added: rows_merged as i64,
                    rows_removed: rows_merged as i64, // ~equal; true bookkeeping would be per-extent
                    bytes_added: new_manifest.byte_size as i64,
                    bytes_removed: bytes_in as i64,
                    operation: "compaction".into(),
                })
                .await
            {
                Ok(_) => return Ok(()),
                Err(Error::Catalog(CatalogError::Conflict)) => {
                    ::metrics::counter!("kyma_catalog_cas_conflicts_total",
                        "table" => "_compaction")
                    .increment(1);
                    tokio::time::sleep(Duration::from_millis(50 * attempt as u64)).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::Catalog(CatalogError::Conflict))
    }
}

fn ratio(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

fn new_manifest_from_write(
    table: &TableRef,
    w: &kyma_core::segment_format::ExtentWriteResult,
    attempt: i32,
) -> ExtentManifest {
    ExtentManifest {
        id: w.extent_id,
        table_id: table.id,
        schema_snapshot_id: table.schema_snapshot_id,
        object_path: w.object_path.clone(),
        byte_size: w.byte_size,
        row_count: w.row_count,
        min_timestamp: w.min_timestamp_nanos.map(nanos_to_utc),
        max_timestamp: w.max_timestamp_nanos.map(nanos_to_utc),
        column_stats: w.column_stats.clone(),
        present_paths: w.present_paths.clone(),
        compaction_gen: (attempt as u32).max(1),
        created_at: chrono::Utc::now(),
    }
}

fn nanos_to_utc(n: i64) -> chrono::DateTime<chrono::Utc> {
    let secs = n.div_euclid(1_000_000_000);
    let nsec = n.rem_euclid(1_000_000_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec).unwrap_or_else(chrono::Utc::now)
}

// ---------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------

/// Periodically submits compaction tasks for tables with too many small
/// extents. Size-tiered policy: find the smallest `max_merge` extents in
/// any table that has more than `min_extents_to_compact`, submit one task
/// covering them.
#[derive(Clone)]
pub struct CompactionScheduler {
    catalog: Arc<dyn Catalog>,
    pub min_extents_to_compact: i64,
    pub max_merge: i64,
    pub small_extent_threshold_bytes: i64,
    pub poll_interval: Duration,
}

impl CompactionScheduler {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self {
            catalog,
            min_extents_to_compact: 4,
            max_merge: 8,
            small_extent_threshold_bytes: 64 * 1024 * 1024, // 64 MiB
            poll_interval: Duration::from_secs(30),
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(
            min_extents = self.min_extents_to_compact,
            max_merge = self.max_merge,
            small_bytes = self.small_extent_threshold_bytes,
            "compaction scheduler starting"
        );
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => {
                    info!("compaction scheduler shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.poll_interval) => {
                    if let Err(e) = self.tick().await {
                        warn!(error = %e, "compaction scheduler tick error");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> Result<()> {
        let candidates = find_compaction_candidates(
            &self.catalog,
            self.min_extents_to_compact,
            self.max_merge,
            self.small_extent_threshold_bytes,
        )
        .await?;
        for group in candidates {
            let payload = serde_json::to_value(CompactionPayload {
                source_extent_ids: group.source_extent_ids,
            })
            .map_err(|e| Error::Internal(format!("payload: {e}")))?;
            let id = self
                .catalog
                .submit_task("compaction", Some(group.table_id), payload, 0)
                .await?;
            debug!(task_id = %id, table_id = %group.table_id, "submitted compaction task");
            ::metrics::counter!("kyma_compaction_tasks_submitted_total").increment(1);
        }
        Ok(())
    }
}

struct CompactionCandidate {
    table_id: TableId,
    source_extent_ids: Vec<uuid::Uuid>,
}

/// Find tables with enough small extents to warrant a compaction.
///
/// Delegates to `Catalog::compaction_candidates`, so it works against any
/// backend (Postgres or the local SQLite catalog) — the backend runs the
/// extent-size window query natively.
async fn find_compaction_candidates(
    catalog: &Arc<dyn Catalog>,
    min_extents: i64,
    max_merge: i64,
    small_bytes: i64,
) -> Result<Vec<CompactionCandidate>> {
    let groups = catalog
        .compaction_candidates(small_bytes, min_extents, max_merge)
        .await?;
    Ok(groups
        .into_iter()
        .map(|(table_id, eids)| CompactionCandidate {
            table_id,
            source_extent_ids: eids.iter().map(|e| *e.as_uuid()).collect(),
        })
        .collect())
}
