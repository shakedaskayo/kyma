//! `embed_backfill` executor — decouples ingest from embedding.
//!
//! Rows can land with a NULL embedding column; this background job computes
//! embeddings for a configured **text** column and fills them into the
//! **vector** column. Because extents are immutable, it does so by REWRITING
//! the extent: it reads every block, replaces the embedding column with the
//! computed `FixedSizeList<Float32, dim>`, writes a NEW extent through the
//! format, and commits a snapshot that adds the new extent and removes the old
//! one (the same supersede pattern compaction uses). Queries then see the
//! embedded extent.
//!
//! Payload: [`EmbedBackfillPayload`]. For each extent the executor:
//!
//! 1. opens it through the [`SegmentFormat`] and reads all blocks,
//! 2. is **idempotent**: if the embedding column is already fully populated
//!    (no NULLs), the extent is skipped — re-runs and at-least-once delivery
//!    are no-ops,
//! 3. collects the source-column text per row, content-hashes it (sha256),
//! 4. batch-looks-up the [`Catalog`] embedding **cache**, embeds the misses via
//!    the injected [`EmbeddingBackend`], and writes the misses back to the
//!    cache — so re-ingested / duplicate text is embedded at most once,
//! 5. rebuilds each block's `RecordBatch` with the embedding column filled and
//!    writes a new extent, then commits add-new + remove-old.
//!
//! Model-id safety: the executor asserts `embedder.id() == payload.model_id`
//! and refuses to run otherwise — never writing vectors from a different model
//! than the config/payload declares (a distance-space-mismatch correctness
//! bug). Production may inject a per-model registry; for now one embedder is
//! accepted and id-checked.
//!
//! Degrade per-extent: one bad extent logs and continues; the job result
//! reports how many were embedded / skipped / failed.

use crate::executor::{JobCtx, JobError, JobExecutor};
use arrow_array::builder::{FixedSizeListBuilder, Float32Builder};
use arrow_array::cast::AsArray;
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use pensieve_core::catalog::{Catalog, ExtentManifest, PrunePredicate, SnapshotSummary, TableRef};
use pensieve_core::crypto::content_hash_hex;
use pensieve_core::fabric::{ClaimedJob, JOB_EMBED_BACKFILL};
use pensieve_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use pensieve_core::tenant::TenantId;
use pensieve_core::types::{ExtentId, TableId};
use pensieve_embed::EmbeddingBackend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// `jobs.payload` for [`JOB_EMBED_BACKFILL`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedBackfillPayload {
    pub table_id: Uuid,
    pub extent_ids: Vec<Uuid>,
    /// Text (`Utf8`) column to embed.
    pub source_column: String,
    /// Vector (`FixedSizeList<Float32, dim>`) column to fill.
    pub embedding_column: String,
    /// Embedding-backend id the vectors must come from. Asserted against the
    /// injected backend's [`EmbeddingBackend::id`].
    pub model_id: String,
}

/// Executor for [`JOB_EMBED_BACKFILL`] jobs.
pub struct EmbedBackfillExecutor {
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    embedder: Arc<dyn EmbeddingBackend>,
}

impl EmbedBackfillExecutor {
    pub fn new(
        catalog: Arc<dyn Catalog>,
        format: Arc<dyn SegmentFormat>,
        embedder: Arc<dyn EmbeddingBackend>,
    ) -> Self {
        Self {
            catalog,
            format,
            embedder,
        }
    }

    /// Resolve a `TableRef` by id, scanning the tenant's databases. The
    /// `Catalog` trait has no direct lookup-by-id; this mirrors the
    /// `index_build` / compaction workers so it works on any backend.
    async fn table_by_id(
        &self,
        tenant: TenantId,
        table_id: TableId,
    ) -> Result<Option<TableRef>, JobError> {
        let dbs = self
            .catalog
            .list_databases_in_tenant(tenant)
            .await
            .map_err(|e| JobError::Transient(format!("list databases: {e}")))?;
        for db in dbs {
            let tables = self
                .catalog
                .list_tables_in_database_in_tenant(tenant, &db)
                .await
                .map_err(|e| JobError::Transient(format!("list tables in {db}: {e}")))?;
            if let Some(t) = tables.into_iter().find(|t| t.id == table_id) {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// Backfill embeddings into ONE extent by rewriting it. Returns `Ok(true)`
    /// if the extent was rewritten, `Ok(false)` if it was skipped (already
    /// fully embedded). Per-extent errors are returned to the caller, which
    /// logs + continues (one bad extent doesn't fail the whole job).
    async fn backfill_one(
        &self,
        tenant: TenantId,
        table: &TableRef,
        manifest: &ExtentManifest,
        source_column: &str,
        embedding_column: &str,
        dim: usize,
    ) -> Result<bool, JobError> {
        // The embedding column must exist in the table schema and be a
        // FixedSizeList<Float32, dim>. We reuse its exact Field (inner field
        // name + nullability) so the rewritten batch schema matches the table.
        let emb_field = table
            .schema
            .fields()
            .iter()
            .find(|f| f.name() == embedding_column)
            .cloned()
            .ok_or_else(|| {
                JobError::Permanent(format!(
                    "embedding column '{embedding_column}' not in table '{}' schema",
                    table.name
                ))
            })?;
        let inner_field = match emb_field.data_type() {
            DataType::FixedSizeList(inner, list_dim) if *list_dim as usize == dim => {
                if !matches!(inner.data_type(), DataType::Float32) {
                    return Err(JobError::Permanent(format!(
                        "embedding column '{embedding_column}' inner type is {:?}, not Float32",
                        inner.data_type()
                    )));
                }
                inner.clone()
            }
            other => {
                return Err(JobError::Permanent(format!(
                    "embedding column '{embedding_column}' is {other:?}, \
                     expected FixedSizeList<Float32, {dim}>"
                )));
            }
        };

        let reader = self
            .format
            .open_extent(OpenExtentInput {
                extent_id: manifest.id,
                table_id: manifest.table_id,
                schema: table.schema.clone(),
                object_path: manifest.object_path.clone(),
                byte_size: manifest.byte_size,
            })
            .await
            .map_err(|e| JobError::Transient(format!("open extent {}: {e}", manifest.id)))?;

        let block_ids = reader
            .pruned_blocks(&BlockPredicate::All)
            .await
            .map_err(|e| JobError::Transient(format!("list blocks of {}: {e}", manifest.id)))?;

        // Read all blocks up front (all columns: empty projection). We need the
        // full batch to copy every column into the rewritten extent.
        let mut blocks: Vec<RecordBatch> = Vec::with_capacity(block_ids.len());
        for bid in block_ids {
            let b = reader
                .read_block(bid, &[])
                .await
                .map_err(|e| JobError::Transient(format!("read block of {}: {e}", manifest.id)))?;
            blocks.push(b);
        }

        // 1. Build a per-block, per-row plan. A row's plan entry is
        //    `Some(hash)` when it NEEDS a freshly-computed embedding (non-empty
        //    source text whose embedding slot is currently null), else `None`
        //    (preserve an existing vector, or leave null when the text is
        //    empty/null). Collect the distinct (hash → text) of needed rows.
        let mut plans: Vec<Vec<Option<String>>> = Vec::with_capacity(blocks.len());
        let mut wanted: HashSet<String> = HashSet::new();
        let mut hash_text: HashMap<String, String> = HashMap::new();
        let mut needs_work = false;
        for b in &blocks {
            let src = b
                .column_by_name(source_column)
                .ok_or_else(|| {
                    JobError::Permanent(format!(
                        "source column '{source_column}' not found in extent {}",
                        manifest.id
                    ))
                })?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| {
                    JobError::Permanent(format!(
                        "source column '{source_column}' is not Utf8 in extent {}",
                        manifest.id
                    ))
                })?
                .clone();
            let emb_present = embedding_presence_mask(b, embedding_column);
            let mut block_plan = Vec::with_capacity(b.num_rows());
            for r in 0..b.num_rows() {
                // Already-embedded slots are preserved as-is.
                let already = emb_present.get(r).copied().unwrap_or(false);
                let text = if src.is_null(r) { "" } else { src.value(r) };
                if already || text.is_empty() {
                    // Nothing to compute: preserve existing vector or null slot.
                    block_plan.push(None);
                } else {
                    let h = content_hash_hex(text.as_bytes());
                    wanted.insert(h.clone());
                    hash_text
                        .entry(h.clone())
                        .or_insert_with(|| text.to_string());
                    block_plan.push(Some(h));
                    needs_work = true;
                }
            }
            plans.push(block_plan);
        }

        // Idempotency: if no row needs a freshly-computed embedding (every
        // non-empty-text row already carries a vector), the extent is done —
        // skip the rewrite. Re-runs and at-least-once delivery are no-ops, and
        // we never call the backend.
        if !needs_work {
            return Ok(false);
        }

        // 2. Cache lookup → embed misses → cache write.
        let wanted_vec: Vec<String> = wanted.iter().cloned().collect();
        let mut cache = self
            .catalog
            .get_cached_embeddings(tenant, self.embedder.id(), &wanted_vec)
            .await
            .map_err(|e| JobError::Transient(format!("cache lookup: {e}")))?;

        let miss_hashes: Vec<String> = wanted_vec
            .iter()
            .filter(|h| !cache.contains_key(*h))
            .cloned()
            .collect();
        if !miss_hashes.is_empty() {
            let miss_texts: Vec<String> = miss_hashes
                .iter()
                .map(|h| hash_text.get(h).cloned().unwrap_or_default())
                .collect();
            let vectors = self.embedder.embed(&miss_texts).await.map_err(|e| {
                JobError::Transient(format!("embed {} texts: {e}", miss_texts.len()))
            })?;
            if vectors.len() != miss_hashes.len() {
                return Err(JobError::Permanent(format!(
                    "embedder returned {} vectors for {} inputs",
                    vectors.len(),
                    miss_hashes.len()
                )));
            }
            let mut new_entries: Vec<(String, Vec<f32>)> = Vec::with_capacity(miss_hashes.len());
            for (h, v) in miss_hashes.iter().zip(vectors.into_iter()) {
                if v.len() != dim {
                    return Err(JobError::Permanent(format!(
                        "embedder '{}' returned dim {} for column '{embedding_column}' (expected {dim})",
                        self.embedder.id(),
                        v.len()
                    )));
                }
                cache.insert(h.clone(), v.clone());
                new_entries.push((h.clone(), v));
            }
            self.catalog
                .put_cached_embeddings(tenant, self.embedder.id(), dim as u16, &new_entries)
                .await
                .map_err(|e| JobError::Transient(format!("cache write: {e}")))?;
        }

        // 3. Rewrite the extent: per block, rebuild the RecordBatch with the
        //    embedding column replaced by the filled FixedSizeList. All other
        //    columns are copied verbatim (same Arc, zero-copy).
        let mut writer = self
            .format
            .start_extent(table.schema.clone(), 0)
            .await
            .map_err(|e| JobError::Transient(format!("start extent: {e}")))?;

        let out_schema: Arc<Schema> = Arc::new(Schema::new(
            table.schema.fields().iter().cloned().collect::<Vec<_>>(),
        ));
        let mut rewritten_rows = 0u64;

        for (b, plan) in blocks.iter().zip(plans.iter()) {
            let emb_col = build_embedding_array(
                b,
                embedding_column,
                &inner_field,
                dim,
                plan.iter().map(|h| h.as_deref()),
                &cache,
            )?;

            // Assemble the output columns in table-schema order; the embedding
            // column is the freshly-built one, everything else is copied.
            let mut out_cols: Vec<ArrayRef> = Vec::with_capacity(out_schema.fields().len());
            for f in out_schema.fields() {
                if f.name() == embedding_column {
                    out_cols.push(emb_col.clone());
                } else if let Some(c) = b.column_by_name(f.name()) {
                    out_cols.push(c.clone());
                } else {
                    // Column in table schema absent from this (narrower) block:
                    // null-fill. Mirrors the query-path promotion.
                    out_cols.push(arrow_array::new_null_array(f.data_type(), b.num_rows()));
                }
            }
            let out_batch = RecordBatch::try_new(out_schema.clone(), out_cols)
                .map_err(|e| JobError::Permanent(format!("assemble rewritten batch: {e}")))?;
            rewritten_rows += out_batch.num_rows() as u64;
            writer
                .append(out_batch)
                .await
                .map_err(|e| JobError::Transient(format!("append rewritten block: {e}")))?;
        }

        let result = writer
            .finish()
            .await
            .map_err(|e| JobError::Transient(format!("finish rewritten extent: {e}")))?;

        // 4. Commit: add the new (embedded) extent, remove the old one. Same
        //    supersede mechanic as compaction (add_extent + remove_extents in
        //    one snapshot), with CAS-conflict retry.
        let new_manifest = ExtentManifest {
            id: result.extent_id,
            table_id: table.id,
            schema_snapshot_id: table.schema_snapshot_id,
            object_path: result.object_path.clone(),
            byte_size: result.byte_size,
            row_count: result.row_count,
            min_timestamp: result.min_timestamp_nanos.map(nanos_to_utc),
            max_timestamp: result.max_timestamp_nanos.map(nanos_to_utc),
            column_stats: result.column_stats.clone(),
            present_paths: result.present_paths.clone(),
            compaction_gen: manifest.compaction_gen.max(1),
            created_at: chrono::Utc::now(),
        };
        self.commit_supersede(tenant, table.id, new_manifest, manifest, rewritten_rows)
            .await?;
        Ok(true)
    }

    /// Commit add-new + remove-old in one snapshot, retrying on CAS conflict —
    /// the same supersede mechanic compaction uses.
    async fn commit_supersede(
        &self,
        tenant: TenantId,
        table_id: TableId,
        new_manifest: ExtentManifest,
        old: &ExtentManifest,
        rows: u64,
    ) -> Result<(), JobError> {
        const MAX: u32 = 10;
        for attempt in 1..=MAX {
            let mut txn = self
                .catalog
                .begin_snapshot_in_tenant(tenant, table_id)
                .await
                .map_err(|e| JobError::Transient(format!("begin snapshot: {e}")))?;
            txn.add_extent(new_manifest.clone())
                .await
                .map_err(|e| JobError::Transient(format!("add extent: {e}")))?;
            txn.remove_extents(&[old.id])
                .await
                .map_err(|e| JobError::Transient(format!("remove extent: {e}")))?;
            match txn
                .commit(SnapshotSummary {
                    rows_added: rows as i64,
                    rows_removed: old.row_count as i64,
                    bytes_added: new_manifest.byte_size as i64,
                    bytes_removed: old.byte_size as i64,
                    operation: "embed_backfill".into(),
                })
                .await
            {
                Ok(_) => return Ok(()),
                Err(pensieve_core::Error::Catalog(pensieve_core::errors::CatalogError::Conflict)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50 * attempt as u64)).await;
                    continue;
                }
                Err(e) => return Err(JobError::Transient(format!("commit: {e}"))),
            }
        }
        Err(JobError::Transient(
            "embed_backfill: snapshot CAS exhausted retries".into(),
        ))
    }
}

/// Per-row presence mask for the embedding column (true = already has a vector).
/// A missing column → all false.
fn embedding_presence_mask(batch: &RecordBatch, column: &str) -> Vec<bool> {
    match batch.column_by_name(column) {
        Some(col) => (0..batch.num_rows()).map(|r| !col.is_null(r)).collect(),
        None => vec![false; batch.num_rows()],
    }
}

/// Build the rewritten embedding column for one block.
///
/// For each row, `row_hashes` yields `Some(hash)` when a freshly-computed vector
/// (looked up in `cache`) should be written, or `None` to either preserve an
/// existing vector or write a null. When `None` and the original block already
/// has a non-null vector at that row, the original vector is copied; otherwise a
/// null slot is written.
fn build_embedding_array<'a>(
    batch: &RecordBatch,
    column: &str,
    inner_field: &Arc<Field>,
    dim: usize,
    row_hashes: impl Iterator<Item = Option<&'a str>>,
    cache: &HashMap<String, Vec<f32>>,
) -> Result<ArrayRef, JobError> {
    let existing = batch.column_by_name(column).and_then(|c| {
        // Downcast to FixedSizeList to copy preserved rows.
        c.as_fixed_size_list_opt().cloned()
    });

    let values = Float32Builder::new();
    let mut builder = FixedSizeListBuilder::new(values, dim as i32).with_field(inner_field.clone());

    for (r, maybe_hash) in row_hashes.enumerate() {
        match maybe_hash {
            Some(h) => {
                let v = cache.get(h).ok_or_else(|| {
                    JobError::Permanent(format!("embedding for hash {h} missing from cache"))
                })?;
                if v.len() != dim {
                    return Err(JobError::Permanent(format!(
                        "cached vector dim {} != {dim}",
                        v.len()
                    )));
                }
                builder.values().append_slice(v);
                builder.append(true);
            }
            None => {
                // Preserve an existing vector if present, else null slot.
                let preserved = existing.as_ref().and_then(|fsl| {
                    if r < fsl.len() && !fsl.is_null(r) {
                        let vals = fsl.value(r);
                        vals.as_primitive_opt::<arrow_array::types::Float32Type>()
                            .map(|p| p.values().to_vec())
                    } else {
                        None
                    }
                });
                match preserved {
                    Some(v) if v.len() == dim => {
                        builder.values().append_slice(&v);
                        builder.append(true);
                    }
                    _ => {
                        // Null slot: still must append `dim` placeholder values
                        // for the fixed-size list, then mark the entry null.
                        for _ in 0..dim {
                            builder.values().append_null();
                        }
                        builder.append(false);
                    }
                }
            }
        }
    }
    Ok(Arc::new(builder.finish()))
}

fn nanos_to_utc(n: i64) -> chrono::DateTime<chrono::Utc> {
    let secs = n.div_euclid(1_000_000_000);
    let nsec = n.rem_euclid(1_000_000_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec).unwrap_or_else(chrono::Utc::now)
}

#[async_trait]
impl JobExecutor for EmbedBackfillExecutor {
    fn kind(&self) -> &'static str {
        JOB_EMBED_BACKFILL
    }

    async fn run(&self, ctx: &JobCtx, job: &ClaimedJob) -> Result<Json, JobError> {
        let payload: EmbedBackfillPayload = serde_json::from_value(job.payload.clone())
            .map_err(|e| JobError::Config(format!("bad embed_backfill payload: {e}")))?;
        let tenant = TenantId::from_uuid(job.tenant_id);

        // Model-id safety: never write vectors from a different model than the
        // payload declares. The injected backend must match.
        if self.embedder.id() != payload.model_id {
            return Err(JobError::Config(format!(
                "embed_backfill: injected embedder id '{}' != payload model_id '{}' \
                 (refusing to write mismatched-model vectors)",
                self.embedder.id(),
                payload.model_id
            )));
        }
        let dim = self.embedder.dimension() as usize;

        let table_id = TableId::from_uuid(payload.table_id);
        let Some(table) = self.table_by_id(tenant, table_id).await? else {
            return Err(JobError::Permanent(format!(
                "embed_backfill: table {table_id} no longer exists"
            )));
        };

        let wanted: Vec<ExtentId> = payload
            .extent_ids
            .iter()
            .map(|u| ExtentId::from_uuid(*u))
            .collect();

        // Resolve live manifests at the current snapshot; extents superseded
        // since enqueue are silently skipped.
        let live = self
            .catalog
            .list_extents_in_tenant(
                tenant,
                table_id,
                table.current_snapshot_id,
                &PrunePredicate::default(),
            )
            .await
            .map_err(|e| JobError::Transient(format!("list extents: {e}")))?;
        let by_id: HashMap<ExtentId, ExtentManifest> =
            live.into_iter().map(|m| (m.id, m)).collect();

        let mut embedded = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        for (i, eid) in wanted.iter().enumerate() {
            ctx.progress
                .push(json!({
                    "current_phase": "embedding",
                    "extent": eid.to_string(),
                    "pct": (i * 100) / wanted.len().max(1),
                }))
                .await;
            let Some(manifest) = by_id.get(eid) else {
                skipped += 1;
                continue;
            };
            match self
                .backfill_one(
                    tenant,
                    &table,
                    manifest,
                    &payload.source_column,
                    &payload.embedding_column,
                    dim,
                )
                .await
            {
                Ok(true) => embedded += 1,
                Ok(false) => skipped += 1,
                Err(e) => {
                    // Degrade per-extent: log and continue.
                    failed += 1;
                    tracing::warn!(
                        extent = %eid,
                        table = %table.name,
                        error = %e,
                        "embed_backfill: extent failed, continuing"
                    );
                }
            }
        }

        Ok(json!({ "embedded": embedded, "skipped": skipped, "failed": failed }))
    }
}
